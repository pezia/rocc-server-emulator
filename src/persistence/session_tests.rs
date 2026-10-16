use super::*;
use std::{
    ffi::CString,
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    dir: PathBuf,
    path: PathBuf,
    protected: ProtectedPaths,
    memory: Memory,
    identity: SnapshotIdentity,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "rocc-ram-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let base = dir.join("base");
        let overlay = dir.join("overlay");
        fs::write(&base, b"original image").unwrap();
        fs::write(&overlay, b"disk journal").unwrap();
        let protected = ProtectedPaths {
            base: FileIdentity::from(&fs::metadata(&base).unwrap()),
            overlay: FileIdentity::from(&fs::metadata(&overlay).unwrap()),
            report: None,
        };
        let (memory, identity) = super::tests::fixture();
        Self {
            path: dir.join("overlay.ram"),
            dir,
            protected,
            memory,
            identity,
        }
    }
    fn saved(&self) -> Vec<u8> {
        let raw = encode_snapshot(&self.memory, &self.identity).unwrap();
        fs::write(&self.path, &raw).unwrap();
        raw
    }
    fn prepare(&self, cold: bool) -> Result<(MemorySession, Option<Vec<u8>>), String> {
        MemorySession::prepare(
            &self.path,
            &self.protected,
            &self.memory,
            &self.identity,
            cold,
        )
    }
    fn session(&self) -> MemorySession {
        let (mut session, _) = self.prepare(false).unwrap();
        session.begin().unwrap();
        session
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.dir).unwrap();
    }
}
#[test]
fn prepare_does_not_mutate_sidecar() {
    let f = Fixture::new();
    let before = f.saved();
    let (_, payload) = f.prepare(false).unwrap();
    assert!(payload.is_some());
    assert_eq!(fs::read(&f.path).unwrap(), before);
}
#[test]
fn begin_makes_previous_snapshot_unrestorable() {
    let f = Fixture::new();
    f.saved();
    let s = f.session();
    assert_eq!(fs::read(&f.path).unwrap()[10], 1);
    assert!(!s.status().committed);
    drop(s);
    assert!(f.prepare(false).err().unwrap().contains("interrupted"));
}
#[test]
fn cold_recovery_accepts_recognized_damaged_snapshot() {
    let f = Fixture::new();
    let mut raw = f.saved();
    raw[160] ^= 1;
    fs::write(&f.path, &raw).unwrap();
    assert!(f.prepare(false).is_err());
    let (mut s, payload) = f.prepare(true).unwrap();
    assert!(payload.is_none());
    assert!(s.status().bypassed_for_cold_boot);
    s.begin().unwrap();
    s.commit(&f.memory, &f.identity).unwrap();
    assert!(f.prepare(false).is_ok());
}
#[test]
fn unrecognized_file_is_preserved() {
    let f = Fixture::new();
    fs::write(&f.path, b"my unrelated file").unwrap();
    assert!(f.prepare(true).is_err());
    assert_eq!(fs::read(&f.path).unwrap(), b"my unrelated file");
}
#[test]
fn sidecar_paths_reject_aliases_and_special_files() {
    for kind in 0..7 {
        let mut f = Fixture::new();
        let original = f.saved();
        fs::remove_file(&f.path).unwrap();
        match kind {
            0 => fs::hard_link(f.dir.join("base"), &f.path).unwrap(),
            1 => fs::hard_link(f.dir.join("overlay"), &f.path).unwrap(),
            2 => symlink(f.dir.join("base"), &f.path).unwrap(),
            3 => fs::create_dir(&f.path).unwrap(),
            4 => {
                let name = CString::new(f.path.as_os_str().as_encoded_bytes()).unwrap();
                assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
            }
            5 => {
                fs::write(&f.path, &original).unwrap();
                fs::hard_link(&f.path, f.dir.join("alias")).unwrap();
            }
            _ => {
                f.protected.report = Some(f.path.clone());
            }
        }
        assert!(f.prepare(true).is_err(), "kind {kind}");
        assert_eq!(fs::read(f.dir.join("base")).unwrap(), b"original image");
        assert_eq!(fs::read(f.dir.join("overlay")).unwrap(), b"disk journal");
        if kind == 5 {
            assert_eq!(fs::read(&f.path).unwrap(), original);
        }
        if kind == 6 {
            assert!(!f.path.exists());
        }
    }
}
#[test]
fn commit_failure_preserves_active_or_complete_snapshot() {
    for stage in [
        SaveStage::Write,
        SaveStage::FileSync,
        SaveStage::Rename,
        SaveStage::DirSync,
    ] {
        let f = Fixture::new();
        let mut s = f.session();
        FAIL.with(|value| value.set(Some(stage)));
        assert!(s.commit(&f.memory, &f.identity).is_err());
        assert!(!s.status().committed);
        let raw = fs::read(&f.path).unwrap();
        if stage == SaveStage::DirSync {
            assert!(decode_snapshot(&raw, &f.memory, &f.identity).is_ok());
        } else {
            assert_eq!(raw[10], 1);
        }
        assert!(fs::read_dir(&f.dir).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".rocc-ram-")));
        assert_eq!(fs::read(f.dir.join("base")).unwrap(), b"original image");
    }
}
#[test]
fn sidecar_permissions_follow_private_umask_policy() {
    let f = Fixture::new();
    let mut s = f.session();
    assert_eq!(
        fs::metadata(&f.path).unwrap().permissions().mode() & 0o077,
        0
    );
    s.commit(&f.memory, &f.identity).unwrap();
    assert_eq!(
        fs::metadata(&f.path).unwrap().permissions().mode() & 0o077,
        0
    );
    assert!(s.status().committed);
}
#[test]
fn destination_replaced_during_session_is_not_overwritten() {
    let f = Fixture::new();
    let mut s = f.session();
    fs::rename(&f.path, f.dir.join("old")).unwrap();
    fs::write(&f.path, b"unrelated replacement").unwrap();
    assert!(s.commit(&f.memory, &f.identity).is_err());
    assert_eq!(fs::read(&f.path).unwrap(), b"unrelated replacement");
}
