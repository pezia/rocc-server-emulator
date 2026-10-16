use rocc_emulator::machine::Machine;
use std::{
    fs::{self, File},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static SERIAL: AtomicU64 = AtomicU64::new(0);
pub struct Fixture {
    pub dir: PathBuf,
    pub image: PathBuf,
    pub overlay: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "rocc-tape-api-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let image = dir.join("image");
        let mut raw = vec![0; 6144];
        raw[..2].copy_from_slice(&0x663fu16.to_be_bytes());
        fs::write(&image, raw).unwrap();
        Self {
            overlay: dir.join("disk.overlay"),
            image,
            dir,
        }
    }
    pub fn machine(&self) -> (Machine, File) {
        let image = File::open(&self.image).unwrap();
        let mut m = Machine::new(fs::read(&self.image).unwrap(), true, 16, false, None).unwrap();
        m.attach_overlay(&self.overlay, &image).unwrap();
        (m, image)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.dir).unwrap();
    }
}
