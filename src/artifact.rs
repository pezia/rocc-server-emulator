//! Private, durable, no-overwrite publication shared by reports and journals.
use std::{
    fs::{self, File, OpenOptions},
    io,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static SERIAL: AtomicU64 = AtomicU64::new(0);
pub fn check_destination(path: &Path) -> io::Result<PathBuf> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "destination already exists; choose a new path",
            ))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(fs::canonicalize(parent)?.join(
        path.file_name()
            .ok_or_else(|| io::Error::other("destination needs a filename"))?,
    ))
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Write,
    FileSync,
    Link,
    Unlink,
    DirSync,
}
#[cfg(test)]
thread_local! {static FAIL:std::cell::Cell<Option<Stage>>=const{std::cell::Cell::new(None)};}
fn step(stage: Stage) -> io::Result<()> {
    #[cfg(test)]
    if FAIL.with(|f| {
        if f.get() == Some(stage) {
            f.set(None);
            true
        } else {
            false
        }
    }) {
        return Err(io::Error::other("injected publication failure"));
    }
    let _ = stage;
    Ok(())
}
pub fn publish_new<F>(dest: &Path, write: F) -> io::Result<()>
where
    F: FnOnce(&mut File) -> io::Result<()>,
{
    let dest = check_destination(dest)?;
    let parent = dest
        .parent()
        .ok_or_else(|| io::Error::other("missing destination parent"))?;
    let directory = File::open(parent)?;
    let mut opened = None;
    for _ in 0..128 {
        let path = parent.join(format!(
            ".rocc-artifact-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(f) => {
                opened = Some((path, f));
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    let (temp, mut file) =
        opened.ok_or_else(|| io::Error::other("cannot allocate a unique temporary artifact"))?;
    let mut published = false;
    let result = (|| {
        step(Stage::Write)?;
        write(&mut file)?;
        step(Stage::FileSync)?;
        file.sync_all()?;
        step(Stage::Link)?;
        fs::hard_link(&temp, &dest)?;
        published = true;
        step(Stage::Unlink)?;
        fs::remove_file(&temp)?;
        Ok(())
    })();
    // Always attempt cleanup and directory sync, including failures after publication.
    let cleanup = match fs::remove_file(&temp) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    };
    let synced = step(Stage::DirSync).and_then(|_| directory.sync_all());
    result.and(cleanup).and(synced).map_err(|e: io::Error| {
        if published {
            io::Error::other(format!(
                "artifact published at {}; finalization failed: {e}",
                dest.display()
            ))
        } else {
            e
        }
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "rocc-publish-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&d).unwrap();
        d
    }
    #[test]
    fn writer_error_cleans_temporary() {
        let d = dir();
        assert!(publish_new(&d.join("out"), |f| {
            f.write_all(b"partial")?;
            Err(io::Error::other("writer failed"))
        })
        .is_err());
        assert_eq!(fs::read_dir(&d).unwrap().count(), 0);
        fs::remove_dir(d).unwrap();
    }
    #[test]
    fn publication_failures_preserve_complete_output_or_nothing() {
        for stage in [
            Stage::Write,
            Stage::FileSync,
            Stage::Link,
            Stage::Unlink,
            Stage::DirSync,
        ] {
            let d = dir();
            let dest = d.join("out");
            FAIL.with(|f| f.set(Some(stage)));
            let e = publish_new(&dest, |f| f.write_all(b"complete")).unwrap_err();
            if matches!(stage, Stage::Unlink | Stage::DirSync) {
                assert_eq!(fs::read(&dest).unwrap(), b"complete");
                assert!(e.to_string().contains("published"));
            } else {
                assert!(!dest.exists());
            }
            assert!(fs::read_dir(&d)
                .unwrap()
                .all(|p| p.unwrap().file_name() == "out"));
            fs::remove_dir_all(d).unwrap();
        }
    }
}
