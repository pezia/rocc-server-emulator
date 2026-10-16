use rocc_emulator::{machine::Machine, memory::Mapping};
#[test]
fn irq_fault_preserves_actual_instruction() {
    let mut m = Machine::new(vec![0; 6144], true, 16, true, None).unwrap();
    m.cpu.memory.write(1, 300).unwrap();
    m.cpu.memory.write(300, 0x8208).unwrap();
    m.cpu.ac = [65535, 0, 0, 2];
    m.bus.disk.done = true;
    m.bus.enabled = true;
    m.step();
    assert_eq!(m.steps, 1);
    assert_eq!(m.bus.interrupts, 1);
    assert_eq!(m.cpu.memory.read(0).unwrap(), 127);
    assert_eq!(m.cpu.last_instruction, Some((300, 0x8208)));
    assert_eq!(m.reason.as_deref(), Some("block_move_bounds"));
}
#[test]
fn explicit_source_bank_forward_overlap_is_atomic() {
    let mut m = Machine::new(vec![0; 6144], true, 16, false, None).unwrap();
    m.cpu.memory.write(127, 0x8208).unwrap();
    m.cpu.memory.write(2, 0x100).unwrap();
    m.cpu.memory.set_mapping(Mapping::SelectedBank(1)).unwrap();
    m.cpu.memory.write(0xf000, 77).unwrap();
    m.cpu.memory.set_mapping(Mapping::SelectedBank(0)).unwrap();
    m.cpu.ac = [0xf000, 0xf000, 0, 1];
    m.step();
    assert_eq!(m.cpu.memory.read(0xf000).unwrap(), 77);
}
#[test]
fn invalid_controller_fields_return_faults() {
    let mut m = Machine::new(vec![0; 6144], true, 16, true, None).unwrap();
    assert!(m.bus.io(&mut m.cpu.memory, 0o33, 7, 4, 0).is_err());
    assert!(m.bus.disk.io(&mut m.cpu.memory, true, 7, 4, 0).is_err());
}
#[test]
fn disk_dma_ranges_and_uninitialized_writes_are_atomic() {
    let mut m = Machine::new(vec![0; 6144], true, 16, false, None).unwrap();
    let disk = &mut m.bus.disk;
    disk.io(&mut m.cpu.memory, true, 2, 0, 0).unwrap();
    disk.io(&mut m.cpu.memory, true, 4, 0, 65408).unwrap();
    disk.io(&mut m.cpu.memory, true, 6, 1, 0).unwrap();
    assert_eq!(m.cpu.memory.read(65535).unwrap(), 0);
    assert_eq!(
        disk.io(&mut m.cpu.memory, true, 3, 0, 0).unwrap().value,
        Some(0)
    );
    disk.io(&mut m.cpu.memory, true, 4, 0, 65409).unwrap();
    assert_eq!(
        disk.io(&mut m.cpu.memory, true, 6, 1, 0)
            .unwrap_err()
            .reason(),
        "disk_dma_bounds"
    );
    disk.io(&mut m.cpu.memory, true, 2, 0, 0x1000).unwrap();
    disk.io(&mut m.cpu.memory, true, 4, 0, 1000).unwrap();
    assert_eq!(
        disk.io(&mut m.cpu.memory, true, 6, 1, 0)
            .unwrap_err()
            .reason(),
        "uninitialized_memory"
    );
    assert_eq!(disk.writes, 0);
}
#[test]
fn program_combines_irq_delay_masking_banks_dma_and_terminal_ack() {
    let mut m = Machine::new(vec![0; 6144], true, 16, true, None).unwrap();
    // Main enables IRQs, executes the delay-slot NOP, then halts on return.
    for (address, word) in [
        (127, 0x607f),
        (128, 0x8200),
        (129, 0x663f),
        (1, 300),
        (300, 0x621d),
        (301, 0x721b),
        (302, 0x6c1b),
        (303, 0x765b),
        (304, 0x799b),
        (305, 0x6128),
        (306, 0x61a8),
        (307, 0x0400),
    ] {
        m.cpu.memory.write(address, word).unwrap();
    }
    m.cpu.ac = [0x100, 0xf000, 0, 0];
    m.bus.terminal.connect().unwrap();
    m.bus.disk.done = true;
    m.bus.mask = 0x8000;
    m.step();
    assert_eq!(m.cpu.pc, 128);
    m.step();
    assert_eq!(m.cpu.pc, 129);
    assert_eq!(m.bus.interrupts, 0);
    // Disk wins despite masked terminal event. IRQ entry saves the return address.
    m.step();
    assert_eq!(m.bus.interrupts, 1);
    assert_eq!(m.cpu.memory.read(0).unwrap(), 129);
    assert_eq!(m.cpu.memory.bank(), 1);
    for _ in 0..7 {
        m.step();
    }
    assert_eq!(m.cpu.pc, 129);
    assert_eq!(m.bus.disk.reads, 1);
    assert_eq!(m.bus.terminal.pending_events(), 0);
    assert_eq!(m.cpu.memory.read(0xf000).unwrap(), 0);
    m.cpu.memory.set_mapping(Mapping::SelectedBank(0)).unwrap();
    assert!(m.cpu.memory.read(0xf000).is_err());
    m.step();
    assert_eq!(m.reason.as_deref(), Some("halt"));
}
