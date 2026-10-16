use rocc_emulator::{
    machine::Machine,
    report::{write_report, RunMetadata},
};
#[test]
fn metadata_distinguishes_address_width_and_initial_overlay() {
    for bits in [15, 16] {
        let m = Machine::new(vec![0; 6144], false, bits, false, None).unwrap();
        let mut bytes = Vec::new();
        write_report(&mut bytes, &m, &RunMetadata::default()).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["report_schema_version"], 2);
        assert_eq!(v["configuration"]["address_bits"], bits);
        assert!(v["initial_overlay_sha256"].as_str().unwrap().len() == 64);
        assert_eq!(v["initial_overlay_sha256"], v["final_overlay_sha256"]);
        assert_eq!(v["display"]["sha256"], v["display"]["retained_sha256"]);
    }
}
#[test]
fn streamed_report_matches_legacy_fields() {
    let mut m = Machine::new(vec![0; 6144], true, 16, true, None).unwrap();
    m.cpu.memory.write(127, 0x663f).unwrap();
    m.step();
    let mut bytes = Vec::new();
    write_report(&mut bytes, &m, &RunMetadata::default()).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["reason"], "halt");
    assert_eq!(v["steps"], 1);
    assert_eq!(v["state"]["pc"], 128);
    assert_eq!(v["overlay"]["blocks"], serde_json::json!([]));
    let summary = m.summary();
    assert_eq!(summary.reason, "halt");
    assert_eq!(summary.pc, 128);
}
#[test]
fn replay_history_is_bounded() {
    let mut history = rocc_emulator::report::ReplayHistory::default();
    for step in 0..65536 {
        history.record(step, 65);
    }
    assert!(history.is_complete());
    history.record(65536, 66);
    assert!(!history.is_complete());
    assert_eq!(history.entries().len(), 65536);
    assert_eq!(history.entries().front().unwrap().step, 1);
    assert_eq!(history.entries().back().unwrap().byte, 66);
}
