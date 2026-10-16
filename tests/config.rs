use rocc_emulator::config::{
    CaptureMode, CpuProfile, MachineConfig, ResourceLimits, TerminalProfile,
};
#[test]
fn config_rejects_invalid_profiles_and_limits() {
    for (e, b, p) in [
        (CpuProfile::Extended, 15, TerminalProfile::None),
        (CpuProfile::Loader, 15, TerminalProfile::Experimental),
        (CpuProfile::Loader, 14, TerminalProfile::None),
    ] {
        assert!(
            MachineConfig::new(e, b, p, None, ResourceLimits::default(), CaptureMode::Stop)
                .is_err()
        );
    }
    for (e, b, p) in [
        (0, 1, 1),
        (65537, 1, 1),
        (1, 0, 1),
        (1, 67108865, 1),
        (1, 1, 0),
        (1, 1, 65537),
    ] {
        assert!(ResourceLimits::new(e, b, p).is_err());
    }
    assert!(ResourceLimits::new(65536, 67108864, 65536).is_ok());
    let l = ResourceLimits::default();
    assert_eq!((l.events(), l.bytes(), l.packets()), (4096, 1048576, 4096));
    assert!(MachineConfig::new(
        CpuProfile::Extended,
        16,
        TerminalProfile::None,
        Some(1),
        l,
        CaptureMode::Stop
    )
    .is_err());
}
