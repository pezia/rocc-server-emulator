//! Repeatable synthetic probes; elapsed time excludes setup.
use rocc_emulator::{cpu::Cpu, machine::Machine, terminal::Terminal};
use std::{io, time::Instant};
fn main() {
    let mut args = std::env::args().skip(1);
    let case = args.next().expect("CASE");
    let n: usize = args
        .next()
        .expect("ITERATIONS")
        .parse()
        .expect("integer iterations");
    assert!(n > 0);
    let elapsed = match case.as_str() {
        "instructions" => {
            let mut m = Machine::new(vec![0; 6144], true, 16, false, None).unwrap();
            let start = Instant::now();
            for _ in 0..n {
                m.step();
            }
            assert_eq!(m.steps, n as u64);
            start.elapsed()
        }
        "terminal-tail" => {
            let mut cpu = Cpu::new(16, true).unwrap();
            for a in 0..1024 {
                cpu.memory.write(a, 0x4141).unwrap();
            }
            use rocc_emulator::config::*;
            let config = MachineConfig::new(
                CpuProfile::Extended,
                16,
                TerminalProfile::Experimental,
                None,
                ResourceLimits::default(),
                CaptureMode::Tail,
            )
            .unwrap();
            let mut terminal = Terminal::with_config(&config);
            for _ in 0..514 {
                terminal.io(&cpu.memory, 2, 0, 0).unwrap();
                terminal.io(&cpu.memory, 4, 0, 0).unwrap();
                terminal.io(&cpu.memory, 6, 0, 0xe7ff).unwrap();
                terminal.io(&cpu.memory, 0, 2, 0).unwrap();
            }
            let start = Instant::now();
            for _ in 0..n {
                terminal.io(&cpu.memory, 2, 0, 0).unwrap();
                terminal.io(&cpu.memory, 4, 0, 0).unwrap();
                terminal.io(&cpu.memory, 6, 0, 65).unwrap();
                terminal.io(&cpu.memory, 0, 2, 0).unwrap();
            }
            start.elapsed()
        }
        "report" => {
            assert!(n <= 65536);
            let mut m = Machine::new(vec![0; n.max(24) * 256], true, 16, false, None).unwrap();
            for a in 1000..1128 {
                m.cpu.memory.write(a, 0x4142).unwrap();
            }
            for block in 0..n {
                let head = (block / 32) % 20;
                let a = (block % 32) as u16 | ((head % 2) as u16) << 5 | 0x1000;
                let c = (block / (32 * 20)) as u16 | ((head / 2) as u16) << 10;
                m.bus.disk.io(&mut m.cpu.memory, true, 2, 0, a).unwrap();
                m.bus.disk.io(&mut m.cpu.memory, true, 4, 0, 1000).unwrap();
                m.bus.disk.io(&mut m.cpu.memory, true, 6, 1, c).unwrap();
            }
            let start = Instant::now();
            rocc_emulator::report::write_report(
                io::sink(),
                &m,
                &rocc_emulator::report::RunMetadata::default(),
            )
            .unwrap();
            start.elapsed()
        }
        _ => panic!("unknown CASE (instructions|terminal-tail|report)"),
    };
    println!(
        "{{\"case\":\"{case}\",\"iterations\":{n},\"seconds\":{:.9}}}",
        elapsed.as_secs_f64()
    );
}
