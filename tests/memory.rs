use rocc_emulator::{
    config::*,
    memory::{Mapping, Memory},
};
fn memory() -> Memory {
    Memory::new(
        &MachineConfig::new(
            CpuProfile::Extended,
            16,
            TerminalProfile::None,
            None,
            ResourceLimits::default(),
            CaptureMode::Stop,
        )
        .unwrap(),
    )
}
#[test]
fn invalid_addresses_and_banks_return_faults() {
    let mut m = memory();
    assert!(m.read(65536).is_err());
    assert!(m.write(65536, 1).is_err());
    assert!(m.set_mapping(Mapping::SelectedBank(128)).is_err());
    assert!(m
        .validate_range(usize::MAX, 2, Mapping::SelectedBank(0))
        .is_err());
    assert!(m.validate_range(65536, 0, Mapping::SelectedBank(0)).is_ok());
    assert!(m
        .validate_range(65408, 128, Mapping::SelectedBank(0))
        .is_ok());
    m.set_mapping(Mapping::GlobalOnly).unwrap();
    assert_eq!(m.read(0xf000).unwrap_err().reason(), "unmapped_memory_bank");
    assert!(m.initialize_physical(585728, 1).is_err());
}
