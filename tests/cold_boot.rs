use rocc_emulator::machine::Machine;

#[test]
fn cpu_switch_read_updates_only_the_selected_accumulator() {
    for ac in 0..4 {
        let mut m = Machine::new(vec![0; 6144], true, 16, false, None).unwrap();
        m.cpu.memory.write(127, 0x613f | (ac << 11)).unwrap();
        m.cpu.ac = [1, 2, 3, 4];
        m.cpu.carry = 1;
        m.step();
        assert_eq!(m.reason, None);
        let mut expected = [1, 2, 3, 4];
        expected[ac as usize] = 0; // Default panel: all switches off.
        assert_eq!(m.cpu.ac, expected);
        assert_eq!(m.cpu.carry, 1);
        assert_eq!(m.cpu.pc, 128);
    }
}

#[test]
fn unsupported_cpu_output_and_pulse_still_fault() {
    for (word, reason) in [
        (0x623f, "unsupported_cpu_control"),
        (0x61ff, "unsupported_cpu_pulse"),
    ] {
        let mut m = Machine::new(vec![0; 6144], true, 16, false, None).unwrap();
        m.cpu.memory.write(127, word).unwrap();
        m.step();
        assert_eq!(m.reason.as_deref(), Some(reason));
    }
}

#[test]
fn switch_reads_preserve_panel_and_apply_interrupt_pulses() {
    for (pulse, enabled, delay) in [(0, false, 0), (1, true, 1), (2, false, 0)] {
        let mut m = Machine::new(vec![0; 6144], true, 16, false, None).unwrap();
        m.bus.switches = 0xa55a;
        let result = m.bus.io(&mut m.cpu.memory, 0o77, 1, pulse, 0x1234).unwrap();
        assert_eq!(result.value, Some(0xa55a));
        assert_eq!((m.bus.enabled, m.bus.delay), (enabled, delay));
        m.bus.io(&mut m.cpu.memory, 0o77, 5, 2, 0).unwrap();
        assert_eq!(m.bus.switches, 0xa55a);
    }
}

#[test]
fn recovery_requires_exact_handshake_and_is_one_shot() {
    let mut m = Machine::new(vec![0; 6144], true, 16, false, None).unwrap();
    let config = m.config().clone().with_cold_boot(true).unwrap();
    m = Machine::with_config(vec![0; 6144], config).unwrap();
    for (n, word) in [
        0x613f, 0x29e1, 0x8d0c, 0x010d, 0x663f, 0x613f, 0x8004, 0x01fd,
    ]
    .into_iter()
    .enumerate()
    {
        m.cpu.memory.write(40 + n, word).unwrap();
    }
    m.cpu.ac = [0xf0f0, 0xf0f0, 0, 0];
    m.cpu.pc = 44;
    // A changed instruction means this is not the verified recovery sequence.
    m.cpu.memory.write(47, 0x8200).unwrap();
    m.step();
    assert_eq!(m.reason.as_deref(), Some("halt"));
    assert!(!m.cold_boot_completed());
    assert_eq!(m.bus.switches, 0xf0f0);
    m.reason = None;
    m.cpu.pc = 44;
    m.cpu.memory.write(47, 0x01fd).unwrap();
    for operands in [[0, 0xf0f0], [0xf0f0, 0]] {
        m.cpu.ac[..2].copy_from_slice(&operands);
        m.cpu.pc = 44;
        m.reason = None;
        m.step();
        assert_eq!(m.reason.as_deref(), Some("halt"));
        assert!(!m.cold_boot_completed());
        assert_eq!(m.bus.switches, 0xf0f0);
    }
    m.reason = None;
    m.cpu.pc = 44;
    m.cpu.ac[..2].copy_from_slice(&[0xf0f0, 0xf0f0]);
    m.step();
    assert_eq!(m.reason, None);
    assert!(m.cold_boot_completed());
    assert_eq!(m.bus.switches, 0xffff);
    // Even if the operator changes switches back, later HALTs must stop.
    m.bus.switches = 0xf0f0;
    m.cpu.pc = 44;
    m.step();
    assert_eq!(m.reason.as_deref(), Some("halt"));
}

#[test]
fn loader_profile_keeps_cpu_switch_reads_unsupported() {
    let mut m = Machine::new(vec![0; 6144], false, 15, false, None).unwrap();
    assert!(m.config().clone().with_cold_boot(true).is_err());
    m.cpu.memory.write(127, 0x613f).unwrap();
    m.step();
    assert_eq!(m.reason.as_deref(), Some("unsupported_io"));
}
