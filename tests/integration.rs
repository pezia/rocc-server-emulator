use rocc_emulator::{machine::Machine,memory::Mapping};
#[test]
fn irq_fault_preserves_actual_instruction() {
 let mut m=Machine::new(vec![0;6144],true,16,true,None).unwrap();
 m.cpu.memory.write(1,300).unwrap();m.cpu.memory.write(300,0x8208).unwrap();m.cpu.ac=[65535,0,0,2];m.bus.disk.done=true;m.bus.enabled=true;
 m.step();assert_eq!(m.steps,1);assert_eq!(m.bus.interrupts,1);assert_eq!(m.cpu.memory.read(0).unwrap(),127);assert_eq!(m.cpu.last_instruction,Some((300,0x8208)));assert_eq!(m.reason.as_deref(),Some("block_move_bounds"));
}
#[test]
fn explicit_source_bank_forward_overlap_is_atomic(){
 let mut m=Machine::new(vec![0;6144],true,16,false,None).unwrap();m.cpu.memory.write(127,0x8208).unwrap();m.cpu.memory.write(2,0x100).unwrap();
 m.cpu.memory.set_mapping(Mapping::SelectedBank(1)).unwrap();m.cpu.memory.write(0xf000,77).unwrap();m.cpu.memory.set_mapping(Mapping::SelectedBank(0)).unwrap();m.cpu.ac=[0xf000,0xf000,0,1];m.step();assert_eq!(m.cpu.memory.read(0xf000).unwrap(),77);
}
