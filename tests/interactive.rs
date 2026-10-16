#[test]
fn interactive_rejects_inconsistent_capture_policy_before_touching_terminal() {
    let mut machine =
        rocc_emulator::machine::Machine::new(vec![0; 6144], true, 16, true, None).unwrap();
    let error =
        rocc_emulator::interactive::run(&mut machine, Some(1), Default::default()).unwrap_err();
    assert!(error.contains("tail capture"), "{error}");
    assert_eq!(machine.steps, 0);
}
