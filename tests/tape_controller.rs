#[path = "common/tape.rs"]
mod common;
use common::Fixture;
use rocc_emulator::tape::TapeMount;
#[test]
fn public_bus_replays_sanitized_driver_fixtures() {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/tape-controller.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let f = Fixture::new();
        let (mut m, _image) = f.machine();
        m.attach_tape(
            TapeMount::CreateNew(f.dir.join("reel")),
            &f.image,
            &f.overlay,
            None,
        )
        .unwrap();
        for e in case["events"].as_array().unwrap() {
            let r = m
                .bus
                .io(
                    &mut m.cpu.memory,
                    0o20,
                    e["operation"].as_u64().unwrap() as u8,
                    e["pulse"].as_u64().unwrap() as u8,
                    e["value"].as_u64().unwrap() as u16,
                )
                .unwrap();
            assert_eq!(r.value, e["input"].as_u64().map(|v| v as u16));
            assert_eq!(r.skip, e["skip"].as_bool().unwrap());
            assert_eq!(m.bus.pending(), None);
        }
    }
}
