#[path = "common/tape.rs"]
mod common;
use common::Fixture;
use rocc_emulator::tape::TapeMount;
use std::fs;
#[test]
fn mounted_tape_survives_ram_reload_at_bot() {
    let f = Fixture::new();
    let (mut m, image) = f.machine();
    let path = f.dir.join("reel");
    m.attach_tape(
        TapeMount::CreateNew(path.clone()),
        &f.image,
        &f.overlay,
        None,
    )
    .unwrap();
    m.attach_memory(&f.overlay, &image, None).unwrap();
    assert!(m
        .attach_tape(
            TapeMount::CreateNew(f.dir.join("late")),
            &f.image,
            &f.overlay,
            None
        )
        .is_err());
    m.bus.io(&mut m.cpu.memory, 0o20, 2, 1, 0x30).unwrap();
    m.step();
    m.finish_tape().unwrap();
    m.finish_memory().unwrap();
    assert_eq!(m.tape_status().unwrap().position, 1);
    assert_eq!(m.report()["tape"]["marks"], 1);
    let hash = m.tape_status().unwrap().final_sha256.clone();
    drop(m);
    let (mut m, image) = f.machine();
    m.attach_tape(TapeMount::ReadOnly(path), &f.image, &f.overlay, None)
        .unwrap();
    m.attach_memory(&f.overlay, &image, None).unwrap();
    assert!(m.memory_status().restored);
    assert_eq!(m.tape_status().unwrap().position, 0);
    assert_eq!(Some(m.tape_status().unwrap().initial_sha256.clone()), hash);
    m.step();
    m.finish_tape().unwrap();
    m.finish_memory().unwrap();
}
#[test]
fn validation_precedes_file_creation_and_memory_activation() {
    let f = Fixture::new();
    let (mut m, image) = f.machine();
    let path = f.dir.join("report-and-reel");
    assert!(m
        .attach_tape(
            TapeMount::CreateNew(path.clone()),
            &f.image,
            &f.overlay,
            Some(&path)
        )
        .is_err());
    assert!(!path.exists());
    m.attach_memory(&f.overlay, &image, None).unwrap();
    let snapshot = fs::read(f.dir.join("disk.overlay.ram")).unwrap();
    assert!(m
        .attach_tape(
            TapeMount::CreateNew(path.clone()),
            &f.image,
            &f.overlay,
            None
        )
        .is_err());
    assert!(!path.exists());
    assert_eq!(fs::read(f.dir.join("disk.overlay.ram")).unwrap(), snapshot);
    m.step();
    m.finish_memory().unwrap();
}

#[test]
fn tape_finalize_failure_keeps_ram_save_available() {
    let f = Fixture::new();
    let (mut m, image) = f.machine();
    let path = f.dir.join("reel");
    m.attach_tape(
        TapeMount::CreateNew(path.clone()),
        &f.image,
        &f.overlay,
        None,
    )
    .unwrap();
    m.attach_memory(&f.overlay, &image, None).unwrap();
    fs::rename(&path, f.dir.join("moved-reel")).unwrap();
    m.step();
    assert!(m.finish_tape().is_err());
    m.finish_memory().unwrap();
    assert!(m.memory_status().committed);
}
