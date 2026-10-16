use rocc_emulator::artifact::publish_new;
use std::{fs, io::Write};
#[test]
fn raced_destination_is_preserved() {
    let d = std::env::temp_dir().join(format!("rocc-artifact-{}", std::process::id()));
    fs::create_dir_all(&d).unwrap();
    let dest = d.join("report");
    let _ = fs::remove_file(&dest);
    assert!(publish_new(&dest, |f| {
        f.write_all(b"new")?;
        fs::write(&dest, b"keep")
    })
    .is_err());
    assert_eq!(fs::read(&dest).unwrap(), b"keep");
    fs::remove_dir_all(d).unwrap();
}
