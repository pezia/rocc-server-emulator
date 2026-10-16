use crate::persistence::{
    FileIdentity, MemorySession, MemoryStatus, ProtectedPaths, SnapshotIdentity,
};
use crate::{cpu::Cpu, devices::Bus, Fault};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::os::unix::fs::MetadataExt;
use std::{fs::File, path::Path};

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
    config: crate::config::MachineConfig,
    initial_overlay_sha256: String,
    pub cpu: Cpu,
    pub bus: Bus,
    pub steps: u64,
    pub block_moves: u64,
    pub reason: Option<String>,
    pub fault: Option<Fault>,
    pub image_sha256: String,
    pub(crate) recent: VecDeque<(u16, u16, u16)>,
    cold_boot_completed: bool,
    memory_session: Option<MemorySession>,
    disabled_memory_status: MemoryStatus,
}
impl Machine {
    pub fn new(
        raw: Vec<u8>,
        extended: bool,
        address_bits: u8,
        terminal: bool,
        connect: Option<u64>,
    ) -> Result<Self, String> {
        use crate::config::*;
        let config = MachineConfig::new(
            if extended {
                CpuProfile::Extended
            } else {
                CpuProfile::Loader
            },
            address_bits,
            if terminal {
                TerminalProfile::Experimental
            } else {
                TerminalProfile::None
            },
            connect,
            ResourceLimits::default(),
            CaptureMode::Stop,
        )?;
        Self::with_config(raw, config)
    }
    pub fn config(&self) -> &crate::config::MachineConfig {
        &self.config
    }
    pub fn with_config(raw: Vec<u8>, config: crate::config::MachineConfig) -> Result<Self, String> {
        let (extended, address_bits, terminal, connect) = (
            config.extended(),
            config.address_bits(),
            config.experimental(),
            config.connect_after(),
        );
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
        let mut cpu = Cpu::new(address_bits, extended)?;
        for n in 0..128 {
            cpu.memory
                .set(n, u16::from_be_bytes([raw[2 * n], raw[2 * n + 1]]));
        }
        cpu.memory.set(10941, 0);
        cpu.pc = 127;
        let mut bus = Bus::new(raw, extended, terminal, connect);
        if config.cold_boot() {
            bus.switches = 0xf0f0;
        }
        bus.terminal = crate::terminal::Terminal::with_config(&config);
        Ok(Self {
            initial_overlay_sha256: crate::report::overlay_digest(&bus.disk.overlay),
            config,
            cpu,
            bus,
            steps: 0,
            block_moves: 0,
            reason: None,
            fault: None,
            image_sha256,
            recent: VecDeque::with_capacity(32),
            cold_boot_completed: false,
            memory_session: None,
            disabled_memory_status: MemoryStatus::default(),
        })
    }
    pub fn attach_overlay(
        &mut self,
        path: &std::path::Path,
        image: &std::fs::File,
    ) -> Result<(), String> {
        if self.steps != 0 || self.memory_session.is_some() {
            return Err("overlay must be attached before executing guest instructions".into());
        }
        self.bus.disk.attach_overlay(path, image)?;
        self.initial_overlay_sha256 = crate::report::overlay_digest(&self.bus.disk.overlay);
        self.reload_bootstrap();
        Ok(())
    }
    fn reload_bootstrap(&mut self) {
        let boot = self
            .bus
            .disk
            .overlay
            .get(&0)
            .map(Vec::as_slice)
            .unwrap_or(&self.bus.disk.base[..256]);
        for n in 0..128 {
            self.cpu
                .memory
                .set(n, u16::from_be_bytes([boot[2 * n], boot[2 * n + 1]]));
        }
    }
    fn snapshot_identity(&self) -> SnapshotIdentity {
        SnapshotIdentity {
            profile: self.config.profile(),
            address_bits: self.config.address_bits(),
            image_bytes: self.bus.disk.base.len() as u64,
            image_sha256: self.image_sha256.clone(),
            overlay_sha256: crate::report::overlay_digest(&self.bus.disk.overlay),
        }
    }
    /// Attach retained RAM before executing the bootstrap. The disk overlay stays
    /// locked until this machine is dropped, including snapshot finalization.
    pub fn attach_memory(
        &mut self,
        overlay_path: &Path,
        image: &File,
        report_path: Option<&Path>,
    ) -> Result<(), String> {
        if self.steps != 0 || self.memory_session.is_some() {
            return Err("memory must be attached once, before executing guest instructions".into());
        }
        let overlay = self.bus.disk.overlay_identity()?;
        let overlay_path = std::fs::canonicalize(overlay_path).map_err(|e| e.to_string())?;
        let overlay_metadata = std::fs::metadata(&overlay_path).map_err(|e| e.to_string())?;
        if FileIdentity::from(&overlay_metadata) != overlay {
            return Err("memory path must belong to the attached disk overlay".into());
        }
        if overlay_metadata.nlink() != 1 {
            return Err("memory persistence does not support disk overlays with multiple hard links; use a dedicated overlay file".into());
        }
        let mut path = overlay_path.into_os_string();
        path.push(".ram");
        let protected = ProtectedPaths {
            base: FileIdentity::from(&image.metadata().map_err(|e| e.to_string())?),
            overlay,
            report: report_path
                .map(|p| crate::artifact::check_destination(p).map_err(|e| e.to_string()))
                .transpose()?,
        };
        let (mut session, payload) = MemorySession::prepare(
            Path::new(&path),
            &protected,
            &self.cpu.memory,
            &self.snapshot_identity(),
            self.config.cold_boot(),
        )?;
        if let Some(payload) = payload {
            self.cpu.memory.restore_snapshot_bytes(&payload)?;
        }
        self.cpu
            .memory
            .set_mapping(crate::memory::Mapping::SelectedBank(0))
            .map_err(|e| e.reason().to_owned())?;
        self.reload_bootstrap();
        session.begin()?;
        self.memory_session = Some(session);
        Ok(())
    }
    /// Finalize an orderly run; faults leave the durable active-session marker.
    pub fn finish_memory(&mut self) -> Result<(), String> {
        if self.memory_session.is_none() || self.memory_status().committed {
            return Ok(());
        }
        if self.fault.is_some()
            || !matches!(
                self.reason.as_deref(),
                None | Some("halt" | "step_limit" | "user_exit" | "input_eof" | "signal")
            )
        {
            return Ok(());
        }
        let identity = self.snapshot_identity();
        self.memory_session
            .as_mut()
            .unwrap()
            .commit(&self.cpu.memory, &identity)?;
        // Finalization ends execution even when a library caller used a budget
        // without explicitly setting a stop reason.
        self.reason.get_or_insert_with(|| "step_limit".into());
        Ok(())
    }
    pub fn memory_status(&self) -> &MemoryStatus {
        self.memory_session
            .as_ref()
            .map(MemorySession::status)
            .unwrap_or(&self.disabled_memory_status)
    }
    pub fn execute_step(cpu: &mut Cpu, bus: &mut Bus) -> Result<crate::cpu::StepInfo, Fault> {
        cpu.last_instruction = None;
        if bus.extended() {
            bus.terminal.tick()?;
            if bus.delay > 0 {
                bus.delay -= 1;
            } else if bus.enabled && bus.pending().is_some() {
                let vector = cpu.read(1)?;
                cpu.memory.write(0, cpu.pc)?;
                cpu.pc = vector;
                bus.enabled = false;
                bus.interrupts += 1;
            }
        }
        cpu.execute(bus)
    }
    pub fn send_key(&mut self, byte: u8) -> Result<(), Fault> {
        if let Err(fault) = self.bus.terminal.send_key(byte) {
            self.reason = Some(fault.reason().into());
            self.fault = Some(fault.clone());
            return Err(fault);
        }
        Ok(())
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
                if info.halt && !self.complete_cold_boot(info.pc) {
                    self.reason = Some("halt".into());
                }
            }
            Err(f) => {
                let (pc, word) = self.cpu.last_instruction.unwrap_or((self.cpu.pc, 0));
                self.block_moves +=
                    u64::from(self.cpu.last_instruction.is_some_and(|(_, w)| w == 0x8208));
                self.reason = Some(f.reason().to_string());
                self.fault = Some(f);
                if self.recent.len() == 32 {
                    self.recent.pop_front();
                }
                self.recent.push_back((pc, word, self.cpu.pc));
            }
        }
        self.steps += 1;
    }
    fn complete_cold_boot(&mut self, halt_pc: u16) -> bool {
        if !self.config.cold_boot()
            || self.cold_boot_completed
            || self.bus.switches != 0xf0f0
            || self.cpu.ac[..2] != [0xf0f0, 0xf0f0]
        {
            return false;
        }
        // WMS loader's operator cold-start handshake: read F0F0, compare,
        // HALT, then wait for FFFF. Recognize the whole instruction sequence,
        // independent of its load address; never continue an arbitrary HALT.
        const HANDSHAKE: [u16; 8] = [
            0x613f, 0x29e1, 0x8d0c, 0x010d, 0x663f, 0x613f, 0x8004, 0x01fd,
        ];
        if !HANDSHAKE.iter().enumerate().all(|(n, word)| {
            self.cpu
                .read(halt_pc.wrapping_sub(4).wrapping_add(n as u16) as usize)
                .ok()
                == Some(*word)
        }) {
            return false;
        }
        // HALT already advanced PC. Operate only the panel and Continue switch;
        // the guest performs all initialization and persistent disk writes.
        self.bus.switches = 0xffff;
        self.cold_boot_completed = true;
        true
    }
    pub fn cold_boot_completed(&self) -> bool {
        self.cold_boot_completed
    }
    pub fn state(&self) -> Value {
        cpu_state(&self.cpu, self.bus.extended())
    }
    pub fn initial_overlay_sha256(&self) -> &str {
        &self.initial_overlay_sha256
    }
    pub fn summary(&self) -> RunSummary<'_> {
        RunSummary {
            reason: self.reason.as_deref().unwrap_or("step_limit"),
            pc: self.cpu.pc,
            steps: self.steps,
            disk_reads: self.bus.disk.reads,
            disk_writes: self.bus.disk.writes,
            overlay_blocks: self.bus.disk.overlay.len(),
            screen: &self.bus.terminal.screen,
        }
    }
    pub fn report(&self) -> Value {
        serde_json::to_value(crate::report::Report {
            machine: self,
            metadata: &crate::report::RunMetadata::default(),
        })
        .expect("in-memory report")
    }
}
pub struct RunSummary<'a> {
    pub reason: &'a str,
    pub pc: u16,
    pub steps: u64,
    pub disk_reads: u64,
    pub disk_writes: u64,
    pub overlay_blocks: usize,
    pub screen: &'a crate::screen::Screen,
}
pub fn cpu_state(cpu: &Cpu, extended: bool) -> Value {
    let mut v = json!({"pc":cpu.pc,"ac":cpu.ac,"carry":cpu.carry});
    if extended {
        v["bank"] = if cpu.memory.global_only() {
            Value::Null
        } else {
            json!(cpu.memory.bank())
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
