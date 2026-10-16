//! Durable changed-block journal. The base image is never opened for writing.
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::Path,
};

const MAGIC: &[u8; 8] = b"ROCCOVL1";
const HEADER_BYTES: usize = 48;
const RECORD_BYTES: usize = 8 + 256 + 32;
type Blocks = BTreeMap<usize, Vec<u8>>;

pub(crate) struct OverlayFile {
    file: File,
    failed: bool,
}

impl OverlayFile {
    #[allow(dead_code)] // Used by the boot attachment task.
    pub(crate) fn identity(&self) -> Result<crate::persistence::FileIdentity, String> {
        self.file
            .metadata()
            .map(|m| crate::persistence::FileIdentity::from(&m))
            .map_err(|e| e.to_string())
    }
    pub(crate) fn open(path: &Path, image: &File, base: &[u8]) -> Result<(Self, Blocks), String> {
        Self::open_inner(path, image, base, true)
            .map_err(|e| format!("overlay {}: {e}", path.display()))
    }

    fn open_inner(
        path: &Path,
        image: &File,
        base: &[u8],
        recover: bool,
    ) -> Result<(Self, Blocks), String> {
        // Never truncate, follow a final symlink, or block while opening a FIFO.
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(recover)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        let (mut file, created) = if recover {
            match options.create_new(true).open(path) {
                Ok(file) => (file, true),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (
                    options
                        .create_new(false)
                        .open(path)
                        .map_err(|e| e.to_string())?,
                    false,
                ),
                Err(e) => return Err(e.to_string()),
            }
        } else {
            (options.open(path).map_err(|e| e.to_string())?, false)
        };
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        let original = image.metadata().map_err(|e| e.to_string())?;
        if metadata.dev() == original.dev() && metadata.ino() == original.ino() {
            return Err("must be a separate file from the original image".into());
        }
        if !metadata.is_file() {
            return Err("must be a regular file".into());
        }
        // Keep the lock on the open descriptor for the entire emulation session.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(format!(
                "cannot lock (another emulator may be using it): {}",
                std::io::Error::last_os_error()
            ));
        }
        let header = encode_header(base);
        if created {
            file.write_all(&header).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            File::open(parent)
                .and_then(|dir| dir.sync_all())
                .map_err(|e| e.to_string())?;
        } else {
            let mut actual = [0; HEADER_BYTES];
            file.read_exact(&mut actual)
                .map_err(|_| "invalid or incomplete header")?;
            if actual != header.as_slice() {
                return Err("invalid header or base image size/SHA-256 mismatch".into());
            }
        }
        let length = file.metadata().map_err(|e| e.to_string())?.len();
        let records = (length - HEADER_BYTES as u64) / RECORD_BYTES as u64;
        let mut blocks = BTreeMap::new();
        let mut record = [0; RECORD_BYTES];
        for _ in 0..records {
            file.read_exact(&mut record).map_err(|e| e.to_string())?;
            if Sha256::digest(&record[..264]).as_slice() != &record[264..] {
                return Err("record checksum mismatch".into());
            }
            let offset = u64::from_be_bytes(record[..8].try_into().unwrap());
            if !offset.is_multiple_of(256) || offset > (base.len() - 256) as u64 {
                return Err("record block offset is out of bounds or unaligned".into());
            }
            blocks.insert(offset as usize, record[8..264].to_vec());
        }
        let valid_length = HEADER_BYTES as u64 + records * RECORD_BYTES as u64;
        if length != valid_length && recover {
            // An interrupted append cannot have been acknowledged. Keep every
            // complete record and discard only the incomplete final record.
            file.set_len(valid_length).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
        }
        file.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
        Ok((
            Self {
                file,
                failed: false,
            },
            blocks,
        ))
    }

    pub(crate) fn write_block(&mut self, offset: usize, raw: &[u8]) -> Result<(), String> {
        if self.failed {
            return Err("previous overlay write failed; restart to recover".into());
        }
        if raw.len() != 256 {
            return Err("overlay record must contain exactly 256 bytes".into());
        }
        let record = encode_record(offset, raw);
        let result = self
            .file
            .write_all(&record)
            .and_then(|_| self.file.sync_data());
        if let Err(e) = result {
            self.failed = true;
            return Err(e.to_string());
        }
        Ok(())
    }
}

fn encode_header(base: &[u8]) -> Vec<u8> {
    let mut h = Vec::with_capacity(HEADER_BYTES);
    h.extend_from_slice(MAGIC);
    h.extend_from_slice(&(base.len() as u64).to_be_bytes());
    h.extend_from_slice(&Sha256::digest(base));
    h
}
fn encode_record(offset: usize, raw: &[u8]) -> Vec<u8> {
    let mut r = Vec::with_capacity(RECORD_BYTES);
    r.extend_from_slice(&(offset as u64).to_be_bytes());
    r.extend_from_slice(raw);
    let checksum = Sha256::digest(&r);
    r.extend_from_slice(&checksum);
    r
}
#[derive(Debug, serde::Serialize)]
pub struct CompactionSummary {
    pub original_bytes: u64,
    pub compacted_bytes: u64,
    pub blocks: usize,
}
pub fn compact_overlay(
    image: &Path,
    source: &Path,
    destination: &Path,
) -> Result<CompactionSummary, String> {
    use std::io::Read;
    crate::artifact::check_destination(destination).map_err(|e| e.to_string())?;
    let image_file = File::open(image).map_err(|e| e.to_string())?;
    let mut base = Vec::new();
    (&image_file)
        .take(512 * 1024 * 1024 + 1)
        .read_to_end(&mut base)
        .map_err(|e| e.to_string())?;
    if base.len() < 6144 || base.len() > 512 * 1024 * 1024 || !base.len().is_multiple_of(256) {
        return Err("invalid base image size".into());
    }
    // PreserveSource opens read-only and ignores an incomplete tail without truncation.
    let (journal, blocks) = OverlayFile::open_inner(source, &image_file, &base, false)
        .map_err(|e| format!("overlay {}: {e}", source.display()))?;
    let original_bytes = journal.file.metadata().map_err(|e| e.to_string())?.len();
    crate::artifact::publish_new(destination, |f| {
        f.write_all(&encode_header(&base))?;
        for (offset, raw) in &blocks {
            f.write_all(&encode_record(*offset, raw))?;
        }
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    // Keep journal (and its exclusive lock) alive through publication and directory sync.
    drop(journal);
    Ok(CompactionSummary {
        original_bytes,
        compacted_bytes: HEADER_BYTES as u64 + blocks.len() as u64 * RECORD_BYTES as u64,
        blocks: blocks.len(),
    })
}
