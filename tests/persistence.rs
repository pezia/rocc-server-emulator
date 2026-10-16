use rocc_emulator::{machine::Machine, memory::Mapping};
use std::{
    fs::{self, File},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    dir: PathBuf,
    image: PathBuf,
    overlay: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "rocc-memory-api-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let image = dir.join("image");
        let mut raw = vec![0; 6144];
        raw[254..256].copy_from_slice(&0x663fu16.to_be_bytes());
        fs::write(&image, raw).unwrap();
        Self {
            overlay: dir.join("disk.overlay"),
            image,
            dir,
        }
    }
    fn machine(&self) -> (Machine, File) {
        let file = File::open(&self.image).unwrap();
        let mut m = Machine::new(fs::read(&self.image).unwrap(), true, 16, false, None).unwrap();
        m.attach_overlay(&self.overlay, &file).unwrap();
        (m, file)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.dir).unwrap();
    }
}
#[test]
fn retained_ram_restores_before_bootstrap() {
    let f = Fixture::new();
    let (mut m, image) = f.machine();
    m.attach_memory(&f.overlay, &image, None).unwrap();
    m.cpu.memory.write(0, 0x1234).unwrap();
    m.cpu.memory.write(200, 0).unwrap();
    m.cpu.memory.write(10941, 123).unwrap();
    m.cpu
        .memory
        .set_mapping(Mapping::SelectedBank(127))
        .unwrap();
    m.cpu.memory.write(65535, 0xabcd).unwrap();
    // Persist a boot block that differs from both the base and retained RAM.
    m.cpu.memory.write(0, 0x663f).unwrap();
    m.bus
        .disk
        .io(&mut m.cpu.memory, true, 2, 0, 0x1000)
        .unwrap();
    m.bus.disk.io(&mut m.cpu.memory, true, 4, 0, 0).unwrap();
    m.bus.disk.io(&mut m.cpu.memory, true, 6, 1, 0).unwrap();
    m.cpu.memory.write(0, 0x1234).unwrap();
    m.step();
    m.finish_memory().unwrap();
    assert!(m.memory_status().committed);
    drop(m);
    let (mut restored, image) = f.machine();
    restored.attach_memory(&f.overlay, &image, None).unwrap();
    assert!(restored.memory_status().restored);
    assert!(restored.memory_status().loaded_sha256.is_some());
    assert_eq!(restored.cpu.memory.read(0).unwrap(), 0x663f);
    assert_eq!(restored.cpu.memory.peek(200).unwrap(), Some(0));
    assert_eq!(restored.cpu.memory.peek(201).unwrap(), None);
    assert_eq!(restored.cpu.memory.read(10941).unwrap(), 123);
    assert_eq!(restored.cpu.memory.mapping(), Mapping::SelectedBank(0));
    assert_eq!(restored.cpu.ac, [0; 4]);
    assert_eq!(restored.cpu.pc, 127);
    assert_eq!(restored.steps, 0);
    restored
        .cpu
        .memory
        .set_mapping(Mapping::SelectedBank(127))
        .unwrap();
    assert_eq!(restored.cpu.memory.read(65535).unwrap(), 0xabcd);
    assert!(restored.attach_overlay(&f.overlay, &image).is_err());
    restored.step();
    assert!(restored.attach_memory(&f.overlay, &image, None).is_err());
    restored.finish_memory().unwrap();
}
#[test]
fn library_without_attachment_stays_stateless() {
    let f = Fixture::new();
    let (mut m, _) = f.machine();
    m.step();
    m.finish_memory().unwrap();
    assert!(!m.memory_status().enabled);
    assert!(!f.dir.join("disk.overlay.ram").exists());
}
#[test]
fn guest_fault_leaves_memory_active() {
    let f = Fixture::new();
    let (mut m, image) = f.machine();
    m.attach_memory(&f.overlay, &image, None).unwrap();
    m.cpu.memory.write(127, 0x20c8).unwrap();
    m.step();
    assert!(m.fault.is_some());
    m.finish_memory().unwrap();
    assert!(!m.memory_status().committed);
    drop(m);
    let (mut m, image) = f.machine();
    assert!(m
        .attach_memory(&f.overlay, &image, None)
        .unwrap_err()
        .contains("interrupted"));
}
#[test]
fn attachment_requires_matching_overlay_and_zero_steps() {
    let f = Fixture::new();
    let file = File::open(&f.image).unwrap();
    let mut m = Machine::new(fs::read(&f.image).unwrap(), true, 16, false, None).unwrap();
    assert!(m.attach_memory(&f.overlay, &file, None).is_err());
    m.attach_overlay(&f.overlay, &file).unwrap();
    let wrong = f.dir.join("wrong");
    fs::write(&wrong, b"different").unwrap();
    assert!(m.attach_memory(&wrong, &file, None).is_err());
    m.step();
    assert!(m.attach_memory(&f.overlay, &file, None).is_err());
}

#[test]
fn multiply_linked_disk_overlays_cannot_select_separate_ram() {
    let f = Fixture::new();
    let (mut m, image) = f.machine();
    m.attach_memory(&f.overlay, &image, None).unwrap();
    m.step();
    m.finish_memory().unwrap();
    drop(m);
    let ram = f.dir.join("disk.overlay.ram");
    let before = fs::read(&ram).unwrap();
    let alias = f.dir.join("alias.overlay");
    fs::hard_link(&f.overlay, &alias).unwrap();
    // A prior interrupted run through the alias must not be bypassed by
    // selecting the original name's otherwise matching, committed RAM.
    let alias_ram = f.dir.join("alias.overlay.ram");
    let mut interrupted = before.clone();
    interrupted[10] = 1;
    fs::write(&alias_ram, &interrupted).unwrap();
    for path in [&f.overlay, &alias] {
        let mut m = Machine::new(fs::read(&f.image).unwrap(), true, 16, false, None).unwrap();
        m.attach_overlay(path, &image).unwrap();
        assert!(m
            .attach_memory(path, &image, None)
            .unwrap_err()
            .contains("multiple hard links"));
        assert_eq!(fs::read(&ram).unwrap(), before);
        assert_eq!(fs::read(&alias_ram).unwrap(), interrupted);
    }
}
