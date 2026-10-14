use crate::{cpu::Cpu, devices::Bus, Fault};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;

pub fn hex(raw: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(raw.len() * 2);
    for &b in raw {
        out.push(H[(b >> 4) as usize] as char);
        out.push(H[(b & 15) as usize] as char);
    }
    out
}
pub fn digest(raw: &[u8]) -> String {
    hex(&Sha256::digest(raw))
}

pub struct Machine {
    pub cpu: Cpu,
    pub bus: Bus,
    pub steps: u64,
    pub block_moves: u64,
    pub reason: Option<String>,
    pub fault: Option<Fault>,
    pub image_sha256: String,
    recent: VecDeque<(u16, u16, u16)>,
}
impl Machine {
    pub fn new(
        raw: Vec<u8>,
        extended: bool,
        address_bits: u8,
        terminal: bool,
        connect: Option<u64>,
    ) -> Result<Self, String> {
        if raw.len() < 6144 || !raw.len().is_multiple_of(256) {
            return Err("boot image must contain at least24 complete256-byte blocks".into());
        }
        if raw.len() > 512 * 1024 * 1024 {
            return Err("image exceeds512MiB limit".into());
        }
        if ![15, 16].contains(&address_bits) || (extended && address_bits != 16) {
            return Err("invalid address width for profile".into());
        }
        let image_sha256 = digest(&raw);
        let mut cpu = Cpu::new(address_bits, extended);
        for n in 0..128 {
            cpu.memory
                .set(n, u16::from_be_bytes([raw[2 * n], raw[2 * n + 1]]));
        }
        cpu.memory.set(10941, 0);
        cpu.pc = 127;
        Ok(Self {
            cpu,
            bus: Bus::new(raw, extended, terminal, connect),
            steps: 0,
            block_moves: 0,
            reason: None,
            fault: None,
            image_sha256,
            recent: VecDeque::with_capacity(32),
        })
    }
    pub fn step(&mut self) {
        if self.reason.is_some() {
            return;
        }
        match self.cpu.step(&mut self.bus) {
            Ok(info) => {
                self.block_moves += u64::from(info.block_move);
                if self.recent.len() == 32 {
                    self.recent.pop_front();
                }
                self.recent.push_back((info.pc, info.word, self.cpu.pc));
                if info.halt {
                    self.reason = Some("halt".into());
                }
            }
            Err(f) => {
                let (pc, word) = self.cpu.last_instruction.unwrap_or((self.cpu.pc, 0));
                self.block_moves +=
                    u64::from(self.cpu.last_instruction.is_some_and(|(_, w)| w == 0x8208));
                self.reason = Some(f.reason.clone());
                self.fault = Some(f);
                if self.recent.len() == 32 {
                    self.recent.pop_front();
                }
                self.recent.push_back((pc, word, self.cpu.pc));
            }
        }
        self.steps += 1;
    }
    pub fn state(&self) -> Value {
        cpu_state(&self.cpu, self.bus.extended)
    }
    pub fn report(&self) -> Value {
        let mut memory = Vec::with_capacity(self.cpu.memory.storage.len() * 3);
        for v in &self.cpu.memory.storage {
            match v {
                Some(w) => {
                    memory.push(1);
                    memory.extend(w.to_be_bytes());
                }
                None => memory.extend([0, 0, 0]),
            }
        }
        json!({"implementation":"rust", "profile":if self.bus.extended {"rocc-startup-v2"} else {"rocc-startup-v1"},
            "warning":"Experimental inferred hardware; not complete vendor hardware.",
            "trace_format":"bounded recent-instruction trace",
            "image_sha256":self.image_sha256,"steps":self.steps,"reason":self.reason.as_deref().unwrap_or("step_limit"),
            "state":self.state(),"disk_reads":self.bus.disk.reads,"disk_writes":self.bus.disk.writes,
            "disk_controls":self.bus.disk.controls,"block_moves":self.block_moves,"interrupts":self.bus.interrupts,
            "bank_selectors":self.bus.bank_selectors,"terminal":self.bus.terminal.snapshot(),
            "display":self.bus.terminal.display_report(),"memory_sha256":digest(&memory),
            "overlay":self.bus.overlay_report(&self.image_sha256),
            "recent_instructions":self.recent.iter().map(|(pc,word,next)|json!({"pc":pc,"word":word,"next_pc":next})).collect::<Vec<_>>(),
            "fault":self.fault.as_ref().map(|f|&f.details)})
    }
}
pub fn cpu_state(cpu: &Cpu, extended: bool) -> Value {
    let mut v = json!({"pc":cpu.pc,"ac":cpu.ac,"carry":cpu.carry});
    if extended {
        v["bank"] = if cpu.global_only {
            Value::Null
        } else {
            json!(cpu.memory.bank)
        };
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_move_and_interrupt_fault_report_actual_instruction() {
        let mut m = Machine::new(vec![0; 6144], true, 16, true, None).unwrap();
        m.cpu.memory.set(1, 300);
        m.cpu.memory.set(300, 0x8208);
        m.cpu.ac = [0xffff, 0, 0, 2];
        m.bus.disk.done = true;
        m.bus.enabled = true;
        m.step();
        assert_eq!(m.reason.as_deref(), Some("block_move_bounds"));
        assert_eq!(m.block_moves, 1);
        assert_eq!(m.report()["recent_instructions"][0]["pc"], 300);
        assert_eq!(m.report()["recent_instructions"][0]["word"], 0x8208);
    }
    #[test]
    fn boot_rejects_partial_sector_and_halts_with_correct_pc() {
        assert!(Machine::new(vec![0; 6145], true, 16, true, None).is_err());
        let mut raw = vec![0; 6144];
        raw[254..256].copy_from_slice(&0x663fu16.to_be_bytes());
        let mut m = Machine::new(raw, true, 16, true, None).unwrap();
        m.step();
        assert_eq!(m.reason.as_deref(), Some("halt"));
        assert_eq!(m.cpu.pc, 128);
        assert_eq!(m.steps, 1);
        assert_eq!(m.report()["overlay"]["blocks"], json!([]));
    }
}
