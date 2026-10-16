use super::{
    medium::{
        Direction, TapeMedium, TapeObject, TapeRead, TapeStep, MAX_BYTES, MAX_OBJECTS, MAX_RECORD,
    },
    TapeMount,
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};

pub(crate) struct TapeProtectedPaths {
    pub image: PathBuf,
    pub overlay: PathBuf,
    pub ram: PathBuf,
    pub report: Option<PathBuf>,
}
pub(crate) struct TapeSession {
    file: File,
    path: PathBuf,
    identity: (u64, u64),
    pub(super) medium: TapeMedium,
    pub(super) writable: bool,
    failed: bool,
    pub(super) initial_sha256: String,
    pub(super) bytes_written: u64,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Write,
    Sync,
    DirectorySync,
}
#[cfg(test)]
thread_local! { static FAIL: std::cell::Cell<Option<Stage>> = const { std::cell::Cell::new(None) }; }
fn stage(value: Stage) -> Result<(), String> {
    #[cfg(test)]
    if FAIL.with(|v| {
        if v.get() == Some(value) {
            v.set(None);
            true
        } else {
            false
        }
    }) {
        return Err("injected tape persistence failure".into());
    }
    let _ = value;
    Ok(())
}
fn normalized(path: &Path) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(fs::canonicalize(parent)
        .map_err(|e| e.to_string())?
        .join(path.file_name().ok_or("tape path requires a filename")?))
}
impl TapeSession {
    pub(super) fn open(mount: &TapeMount, protected: &TapeProtectedPaths) -> Result<Self, String> {
        let (source, writable) = match mount {
            TapeMount::ReadOnly(p) => (p, false),
            TapeMount::CreateNew(p) => (p, true),
        };
        let path = normalized(source)?;
        let metadata = match fs::symlink_metadata(&path) {
            Ok(m) => Some(m),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.to_string()),
        };
        if metadata.as_ref().is_some_and(|m| !m.is_file()) {
            return Err("tape must be a regular file, not a symlink".into());
        }
        let protected_paths = [
            Some(&protected.image),
            Some(&protected.overlay),
            Some(&protected.ram),
            protected.report.as_ref(),
        ];
        for other in protected_paths.into_iter().flatten() {
            if path == normalized(other)?
                || metadata
                    .as_ref()
                    .zip(fs::metadata(other).ok().as_ref())
                    .is_some_and(|(a, b)| a.dev() == b.dev() && a.ino() == b.ino())
            {
                return Err("tape aliases image, overlay, RAM or report".into());
            }
        }
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(writable)
            .create_new(writable)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        let mut file = options.open(&path).map_err(|e| e.to_string())?;
        let meta = file.metadata().map_err(|e| e.to_string())?;
        if !meta.is_file() {
            return Err("tape must be a regular file".into());
        }
        for other in protected_paths.into_iter().flatten() {
            if fs::metadata(other)
                .ok()
                .is_some_and(|m| m.dev() == meta.dev() && m.ino() == meta.ino())
            {
                return Err("opened tape aliases a protected file".into());
            }
        }
        let lock = if writable {
            libc::LOCK_EX
        } else {
            libc::LOCK_SH
        };
        if unsafe { libc::flock(file.as_raw_fd(), lock | libc::LOCK_NB) } != 0 {
            return Err(format!(
                "cannot lock tape: {}",
                std::io::Error::last_os_error()
            ));
        }
        let medium = TapeMedium::scan(&mut file)?;
        if writable {
            file.sync_all().map_err(|e| e.to_string())?;
            stage(Stage::DirectorySync)?;
            File::open(path.parent().ok_or("missing tape directory")?)
                .and_then(|f| f.sync_all())
                .map_err(|e| e.to_string())?;
        }
        let initial_sha256 = hash(&mut file)?;
        Ok(Self {
            file,
            path,
            identity: (meta.dev(), meta.ino()),
            medium,
            writable,
            failed: false,
            initial_sha256,
            bytes_written: 0,
        })
    }
    fn check(&self) -> Result<(), String> {
        if self.failed {
            return Err("tape session failed; remount required".into());
        }
        let meta = fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?;
        if !meta.is_file()
            || (meta.dev(), meta.ino()) != self.identity
            || (self.writable && meta.nlink() != 1)
        {
            return Err("mounted tape path identity changed or acquired hard links".into());
        }
        Ok(())
    }
    pub(super) fn read(&mut self) -> Result<TapeRead, String> {
        self.check()?;
        let result = self.medium.read(&mut self.file);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    pub(super) fn space(&mut self, direction: Direction) -> Result<TapeStep, String> {
        self.check()?;
        self.medium.space(direction)
    }
    pub(super) fn rewind(&mut self) {
        self.medium.rewind();
    }
    pub(super) fn write_record(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.is_empty() || bytes.len() > MAX_RECORD as usize {
            return Err("tape data record length must be 1..0xFFFFFF".into());
        }
        self.append(Some(bytes))
    }
    pub(super) fn write_mark(&mut self) -> Result<(), String> {
        self.append(None)
    }
    fn append(&mut self, data: Option<&[u8]>) -> Result<(), String> {
        self.check()?;
        if !self.writable {
            return Err("tape is write protected".into());
        }
        if self.medium.position() != self.medium.objects.len() {
            return Err("tape overwrite operation is not established".into());
        }
        let length = data.map_or(0, |b| b.len() as u32);
        let frame = if data.is_some() {
            8 + u64::from(length) + u64::from(length & 1)
        } else {
            4
        };
        if self.medium.end + frame > MAX_BYTES || self.medium.objects.len() >= MAX_OBJECTS {
            return Err("tape capacity limit reached".into());
        }
        let offset = self.medium.end;
        let result = (|| {
            self.file
                .seek(SeekFrom::Start(offset))
                .map_err(|e| e.to_string())?;
            self.file
                .write_all(&length.to_le_bytes())
                .map_err(|e| e.to_string())?;
            stage(Stage::Write)?;
            if let Some(bytes) = data {
                self.file.write_all(bytes).map_err(|e| e.to_string())?;
                if length & 1 != 0 {
                    self.file.write_all(&[0]).map_err(|e| e.to_string())?;
                }
                self.file
                    .write_all(&length.to_le_bytes())
                    .map_err(|e| e.to_string())?;
            }
            stage(Stage::Sync)?;
            self.file.sync_all().map_err(|e| e.to_string())
        })();
        if let Err(e) = result {
            self.failed = true;
            return Err(e);
        }
        self.medium.offsets.push(offset);
        self.medium.objects.push(if data.is_some() {
            TapeObject::Data { offset, length }
        } else {
            TapeObject::Mark
        });
        self.medium.end += frame;
        self.medium.space(Direction::Forward)?;
        self.bytes_written += frame;
        Ok(())
    }
    pub(super) fn finish(&mut self) -> Result<String, String> {
        self.check()?;
        hash(&mut self.file)
    }
}
fn hash(file: &mut File) -> Result<String, String> {
    file.rewind().map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut block = [0; 65536];
    let mut total = 0;
    loop {
        let n = file.read(&mut block).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > MAX_BYTES {
            return Err("tape changed beyond byte limit".into());
        }
        digest.update(&block[..n]);
    }
    Ok(crate::machine::hex(&digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::{
        medium::{Direction, TapeObject, TapeRead, TapeStep, MAX_BYTES, MAX_OBJECTS},
        TapeMount,
    };
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::{
        fs::{self, File},
        os::{fd::AsRawFd, unix::fs::MetadataExt},
        path::PathBuf,
    };
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    struct Dir(PathBuf);
    impl Dir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "rocc-tape-session-{}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
        fn paths(&self) -> TapeProtectedPaths {
            TapeProtectedPaths {
                image: self.0.join("image"),
                overlay: self.0.join("overlay"),
                ram: self.0.join("ram"),
                report: Some(self.0.join("report")),
            }
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn durable_frames_before_ack_and_readonly_protection() {
        let d = Dir::new();
        let path = d.0.join("tape");
        let mut s = TapeSession::open(&TapeMount::CreateNew(path.clone()), &d.paths()).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        s.write_record(b"abc").unwrap();
        s.write_mark().unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            [3, 0, 0, 0, b'a', b'b', b'c', 0, 3, 0, 0, 0, 0, 0, 0, 0]
        );
        assert!(TapeSession::open(&TapeMount::CreateNew(path.clone()), &d.paths()).is_err());
        drop(s);
        let mut s = TapeSession::open(&TapeMount::ReadOnly(path.clone()), &d.paths()).unwrap();
        assert_eq!(s.read().unwrap(), TapeRead::Data(b"abc".to_vec()));
        assert_eq!(s.read().unwrap(), TapeRead::Mark);
        assert!(s.write_record(b"x").is_err());
        assert!(s.write_mark().is_err());
        assert_eq!(fs::metadata(&path).unwrap().len(), 16);
        assert_eq!(s.space(Direction::Reverse).unwrap(), TapeStep::Mark);
        s.rewind();
        assert_eq!(s.read().unwrap(), TapeRead::Data(b"abc".to_vec()));
    }
    #[test]
    fn aliases_symlinks_nonregular_and_locks_are_rejected() {
        let d = Dir::new();
        let paths = d.paths();
        for protected in [
            &paths.image,
            &paths.overlay,
            &paths.ram,
            paths.report.as_ref().unwrap(),
        ] {
            assert!(TapeSession::open(&TapeMount::CreateNew(protected.clone()), &paths).is_err());
            assert!(!protected.exists());
            fs::write(protected, [0; 4]).unwrap();
            let alias = d.0.join("alias");
            fs::hard_link(protected, &alias).unwrap();
            assert!(TapeSession::open(&TapeMount::ReadOnly(alias.clone()), &paths).is_err());
            fs::remove_file(alias).unwrap();
        }
        let path = d.0.join("tape");
        fs::write(&path, [0; 4]).unwrap();
        let alias = d.0.join("symlink");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(TapeSession::open(&TapeMount::ReadOnly(alias), &paths).is_err());
        assert!(TapeSession::open(&TapeMount::ReadOnly(d.0.clone()), &paths).is_err());
        let handle = File::open(&path).unwrap();
        assert_eq!(
            unsafe { libc::flock(handle.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        assert!(TapeSession::open(&TapeMount::ReadOnly(path.clone()), &paths).is_err());
        drop(handle);
        assert!(TapeSession::open(&TapeMount::ReadOnly(path), &paths).is_ok());
    }
    #[test]
    fn failed_record_is_never_acknowledged_or_reused() {
        for stage in [Stage::Write, Stage::Sync] {
            let d = Dir::new();
            let path = d.0.join("tape");
            let mut s = TapeSession::open(&TapeMount::CreateNew(path.clone()), &d.paths()).unwrap();
            FAIL.with(|v| v.set(Some(stage)));
            assert!(s.write_record(b"abc").is_err());
            assert!(s.write_mark().is_err());
            assert!(s.read().is_err());
            assert!(s.finish().is_err());
        }
        let d = Dir::new();
        FAIL.with(|v| v.set(Some(Stage::DirectorySync)));
        assert!(TapeSession::open(&TapeMount::CreateNew(d.0.join("tape")), &d.paths()).is_err());
    }
    #[test]
    fn append_is_linear_and_limits_precede_modification() {
        let d = Dir::new();
        let path = d.0.join("tape");
        let mut s = TapeSession::open(&TapeMount::CreateNew(path.clone()), &d.paths()).unwrap();
        assert!(s.write_record(&[]).is_err());
        for _ in 0..100 {
            s.write_record(&[0x12, 0x34]).unwrap();
        }
        assert_eq!(s.bytes_written, 1000);
        assert_eq!(fs::metadata(&path).unwrap().len(), 1000);
        s.medium.end = MAX_BYTES - 9;
        assert!(s.write_record(&[1, 2]).is_err());
        assert_eq!(fs::metadata(&path).unwrap().len(), 1000);
        s.medium.end = 1000;
        s.medium.objects.resize(MAX_OBJECTS, TapeObject::Mark);
        assert!(s.write_mark().is_err());
        assert_eq!(fs::metadata(&path).unwrap().len(), 1000);
    }
}
