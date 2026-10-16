use super::{
    decode_snapshot, encode_snapshot, header, recognize, MemoryStatus, SnapshotIdentity, HEADER,
};
use crate::{machine::digest, memory::Memory};
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static SERIAL: AtomicU64 = AtomicU64::new(0);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileIdentity {
    pub dev: u64,
    pub ino: u64,
}
impl From<&Metadata> for FileIdentity {
    fn from(value: &Metadata) -> Self {
        Self {
            dev: value.dev(),
            ino: value.ino(),
        }
    }
}
pub(crate) struct ProtectedPaths {
    pub base: FileIdentity,
    pub overlay: FileIdentity,
    pub report: Option<PathBuf>,
}
pub(crate) struct MemorySession {
    path: PathBuf,
    directory: File,
    file: Option<File>,
    owned: Option<FileIdentity>,
    active_header: Vec<u8>,
    begun: bool,
    status: MemoryStatus,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SaveStage {
    Write,
    FileSync,
    Rename,
    DirSync,
}
#[cfg(test)]
thread_local! {pub(super) static FAIL:std::cell::Cell<Option<SaveStage>>=const{std::cell::Cell::new(None)};}
fn step(stage: SaveStage) -> Result<(), String> {
    #[cfg(test)]
    if FAIL.with(|value| {
        if value.get() == Some(stage) {
            value.set(None);
            true
        } else {
            false
        }
    }) {
        return Err("injected RAM publication failure".into());
    }
    let _ = stage;
    Ok(())
}
fn open_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    options
}
fn checked_identity(metadata: &Metadata) -> Result<FileIdentity, String> {
    if !metadata.is_file() {
        return Err("RAM sidecar must be a regular file".into());
    }
    if metadata.nlink() != 1 {
        return Err("RAM sidecar must not have multiple hard links".into());
    }
    Ok(FileIdentity::from(metadata))
}
impl MemorySession {
    pub(crate) fn prepare(
        path: &Path,
        protected: &ProtectedPaths,
        memory: &Memory,
        identity: &SnapshotIdentity,
        cold_boot: bool,
    ) -> Result<(Self, Option<Vec<u8>>), String> {
        Self::prepare_inner(path, protected, memory, identity, cold_boot)
            .map_err(|e| format!("memory {}: {e}", path.display()))
    }
    fn prepare_inner(
        path: &Path,
        protected: &ProtectedPaths,
        memory: &Memory,
        identity: &SnapshotIdentity,
        cold_boot: bool,
    ) -> Result<(Self, Option<Vec<u8>>), String> {
        let parent = fs::canonicalize(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )
        .map_err(|e| e.to_string())?;
        let path = parent.join(path.file_name().ok_or("RAM sidecar needs a filename")?);
        if protected.report.as_ref() == Some(&path) {
            return Err("report and memory sidecar must use separate paths".into());
        }
        let directory = File::open(&parent).map_err(|e| e.to_string())?;
        let mut file = match open_options().open(&path) {
            Ok(file) => Some(file),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.to_string()),
        };
        let mut owned = None;
        let mut payload = None;
        let mut loaded = None;
        if let Some(file) = file.as_mut() {
            let metadata = file.metadata().map_err(|e| e.to_string())?;
            let file_id = checked_identity(&metadata)?;
            if file_id == protected.base || file_id == protected.overlay {
                return Err("RAM sidecar aliases a protected image or disk overlay".into());
            }
            if let Some(report) = &protected.report {
                if fs::metadata(report).ok().as_ref().map(FileIdentity::from) == Some(file_id) {
                    return Err("RAM sidecar aliases the report".into());
                }
            }
            owned = Some(file_id);
            let limit = if cold_boot {
                HEADER
            } else {
                HEADER + (memory.global_words() + memory.bank_words() * memory.banks()) * 3 + 1
            };
            let mut raw = Vec::new();
            file.take(limit as u64)
                .read_to_end(&mut raw)
                .map_err(|e| e.to_string())?;
            recognize(&raw)?;
            if !cold_boot {
                payload = Some(decode_snapshot(&raw, memory, identity)?);
                loaded = Some(digest(&raw));
            }
        }
        let status = MemoryStatus {
            enabled: true,
            path: Some(path.clone()),
            restored: payload.is_some(),
            bypassed_for_cold_boot: cold_boot,
            committed: false,
            loaded_sha256: loaded,
        };
        Ok((
            Self {
                path,
                directory,
                file,
                owned,
                active_header: header(memory, identity, 1)?,
                begun: false,
                status,
            },
            payload,
        ))
    }
    fn verify_destination(&self) -> Result<(), String> {
        let metadata = fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?;
        if Some(checked_identity(&metadata)?) != self.owned {
            return Err("RAM sidecar was replaced during this session".into());
        }
        Ok(())
    }
    pub(crate) fn begin(&mut self) -> Result<(), String> {
        let result = (|| {
            if self.begun {
                return Err("RAM session already started".into());
            }
            if self.file.is_some() {
                self.verify_destination()?;
            }
            if let Some(file) = self.file.as_mut() {
                file.seek(SeekFrom::Start(10)).map_err(|e| e.to_string())?;
                file.write_all(&[1]).map_err(|e| e.to_string())?;
                file.sync_all().map_err(|e| e.to_string())?;
            } else {
                let file = open_options()
                    .create_new(true)
                    .open(&self.path)
                    .map_err(|e| e.to_string())?;
                self.owned = Some(checked_identity(
                    &file.metadata().map_err(|e| e.to_string())?,
                )?);
                self.file = Some(file);
                let file = self.file.as_mut().unwrap();
                file.write_all(&self.active_header)
                    .map_err(|e| e.to_string())?;
                file.sync_all().map_err(|e| e.to_string())?;
                self.directory.sync_all().map_err(|e| e.to_string())?;
            }
            self.begun = true;
            Ok(())
        })();
        result.map_err(|e: String| format!("memory {}: {e}", self.path.display()))
    }
    pub(crate) fn commit(
        &mut self,
        memory: &Memory,
        identity: &SnapshotIdentity,
    ) -> Result<(), String> {
        self.commit_inner(memory, identity)
            .map_err(|e| format!("memory {}: {e}", self.path.display()))
    }
    fn commit_inner(&mut self, memory: &Memory, identity: &SnapshotIdentity) -> Result<(), String> {
        if !self.begun || self.status.committed {
            return Err("RAM session is not active".into());
        }
        self.verify_destination()?;
        let raw = encode_snapshot(memory, identity)?;
        let parent = self.path.parent().ok_or("missing RAM parent")?;
        let mut opened = None;
        for _ in 0..128 {
            let path = parent.join(format!(
                ".rocc-ram-{}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            match open_options().create_new(true).open(&path) {
                Ok(file) => {
                    opened = Some((path, file));
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        let (temp, mut file) = opened.ok_or("cannot allocate RAM temporary")?;
        let result = (|| {
            step(SaveStage::Write)?;
            file.write_all(&raw).map_err(|e| e.to_string())?;
            step(SaveStage::FileSync)?;
            file.sync_all().map_err(|e| e.to_string())?;
            self.verify_destination()?;
            step(SaveStage::Rename)?;
            fs::rename(&temp, &self.path).map_err(|e| e.to_string())?;
            step(SaveStage::DirSync)?;
            self.directory.sync_all().map_err(|e| e.to_string())?;
            Ok(())
        })();
        // After rename, never remove the new snapshot, even if directory sync failed.
        let cleanup = match fs::remove_file(&temp) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        };
        let result = result.and(cleanup);
        self.status.committed = result.is_ok();
        result
    }
    pub(crate) fn status(&self) -> &MemoryStatus {
        &self.status
    }
}
