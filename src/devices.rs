#[derive(Debug, Clone, Copy)]
enum DiskCompletion {
    Idle,
    Control { end: Option<u16> },
    Transfer { end: u16 },
}
impl DiskCompletion {
    fn end(self) -> Option<u16> {
        match self {
            Self::Idle => None,
            Self::Control { end } => end,
            Self::Transfer { end } => Some(end),
        }
    }
    fn without_control(self) -> Self {
        match self.end() {
            Some(end) => Self::Transfer { end },
            None => Self::Idle,
        }
    }
}
// Inferred controllers with strict validation of unsupported operations.
use crate::{memory::Memory, terminal::Terminal, Fault, IoResult};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub struct Disk {
    pub(crate) base: Vec<u8>,
    pub(crate) overlay: BTreeMap<usize, Vec<u8>>,
    journal: Option<crate::overlay::OverlayFile>,
    pub a: Option<u16>,
    pub b: Option<u16>,
    pub c: Option<u16>,
    pub busy: bool,
    pub done: bool,
    pub reads: u64,
    pub writes: u64,
    pub controls: u64,
    completion: DiskCompletion,
}
impl Disk {
    pub(crate) fn overlay_identity(&self) -> Result<crate::persistence::FileIdentity, String> {
        self.journal
            .as_ref()
            .ok_or_else(|| "memory persistence requires an attached disk overlay".to_owned())?
            .identity()
    }
    pub fn new(base: Vec<u8>) -> Self {
        Self {
            base,
            overlay: BTreeMap::new(),
            journal: None,
            a: None,
            b: None,
            c: None,
            busy: false,
            done: false,
            reads: 0,
            writes: 0,
            controls: 0,
            completion: DiskCompletion::Idle,
        }
    }
    pub(crate) fn attach_overlay(
        &mut self,
        path: &std::path::Path,
        image: &std::fs::File,
    ) -> Result<(), String> {
        let (journal, blocks) = crate::overlay::OverlayFile::open(path, image, &self.base)?;
        self.overlay = blocks;
        self.journal = Some(journal);
        Ok(())
    }
    pub fn reset(&mut self) {
        self.a = None;
        self.b = None;
        self.c = None;
        self.busy = false;
        self.done = false;
        self.completion = DiskCompletion::Idle;
    }
    fn transfer(&mut self, memory: &mut Memory, extended: bool) -> Result<IoResult, Fault> {
        if extended && self.a == Some(0x2000) && self.c == Some(0x2000) {
            self.controls += 1;
            self.completion = DiskCompletion::Control {
                end: self.completion.end(),
            };
            self.busy = false;
            self.done = true;
            return Ok(IoResult::default());
        }
        self.completion = self.completion.without_control();
        if extended
            && memory.global_only()
            && self
                .b
                .is_some_and(|b| b as usize + 128 > memory.global_words())
        {
            return Err(Fault::with(
                crate::FaultKind::UnmappedMemoryBank,
                crate::FaultContext::Address {
                    address: self.b.unwrap_or_default() as usize,
                    selector: None,
                },
            ));
        }
        let a = self
            .a
            .ok_or_else(|| Fault::new(crate::FaultKind::DiskRegisterUninitialized))?;
        let c = self
            .c
            .ok_or_else(|| Fault::new(crate::FaultKind::DiskRegisterUninitialized))?;
        if a & !0x103f != 0 {
            return Err(Fault::with(
                crate::FaultKind::UnsupportedDiskCommand,
                crate::FaultContext::DiskCommand { command: a },
            ));
        }
        let b = self
            .b
            .ok_or_else(|| Fault::new(crate::FaultKind::DiskRegisterUninitialized))?
            as usize;
        let head = (c as usize >> 10) * 2 + ((a as usize >> 5) & 1);
        let offset = ((c as usize & 1023) * 20 + head) * 8192 + (a as usize & 31) * 256;
        if head >= 20 || offset + 256 > self.base.len() {
            return Err(Fault::with(
                crate::FaultKind::DiskAddressBounds,
                crate::FaultContext::DiskAddress { offset, head },
            ));
        }
        if b + 128 > memory.logical_words() {
            return Err(Fault::with(
                crate::FaultKind::DiskDmaBounds,
                crate::FaultContext::Dma { address: b },
            ));
        }
        memory.validate_range(b, 128, memory.mapping())?;
        if a & 0x1000 != 0 {
            let mut raw = Vec::with_capacity(256);
            for n in 0..128 {
                raw.extend_from_slice(&memory.read(b + n)?.to_be_bytes());
            }
            if let Some(journal) = &mut self.journal {
                journal.write_block(offset, &raw).map_err(|error| {
                    Fault::with(
                        crate::FaultKind::DiskOverlayWrite,
                        crate::FaultContext::Persistence { offset, error },
                    )
                })?;
            }
            self.overlay.insert(offset, raw);
            self.writes += 1;
        } else {
            let raw = self
                .overlay
                .get(&offset)
                .map(Vec::as_slice)
                .unwrap_or(&self.base[offset..offset + 256]);
            for n in 0..128 {
                memory.write(b + n, u16::from_be_bytes([raw[n * 2], raw[n * 2 + 1]]))?;
            }
            self.reads += 1;
        }
        self.busy = false;
        self.done = true;
        if extended {
            self.completion = DiskCompletion::Transfer {
                end: (b + 128) as u16,
            };
        }
        Ok(IoResult::default())
    }
    pub fn io(
        &mut self,
        memory: &mut Memory,
        extended: bool,
        op: u8,
        p: u8,
        v: u16,
    ) -> Result<IoResult, Fault> {
        if op > 7 || p > 3 {
            return Err(Fault::new(crate::FaultKind::UnsupportedIo));
        }
        let mut r = IoResult::default();
        if extended
            && (op, p) == (0, 3)
            && matches!(self.completion, DiskCompletion::Control { .. })
        {
            self.completion = self.completion.without_control();
            return Ok(r);
        }
        if extended && (op, p) == (3, 0) {
            r.value = Some(
                self.completion
                    .end()
                    .ok_or_else(|| Fault::new(crate::FaultKind::DiskDmaResultUninitialized))?,
            );
            return Ok(r);
        }
        match (op, p) {
            (7, _) => r.skip = [self.busy, !self.busy, self.done, !self.done][p as usize],
            (0, 2) => self.reset(),
            (2, 0 | 2) => {
                self.a = Some(v);
                if p == 2 {
                    self.busy = false;
                    self.done = false;
                }
            }
            (4, 0) => self.b = Some(v),
            (6, 1) => {
                self.c = Some(v);
                return self.transfer(memory, extended);
            }
            (1, 0 | 2) => {
                if p == 2 {
                    self.busy = false;
                    self.done = false;
                }
                r.value = Some(0);
            }
            _ => {
                return Err(Fault::with(
                    crate::FaultKind::UnsupportedDiskOperation,
                    crate::FaultContext::Protocol {
                        operation: op,
                        pulse: p,
                        value: None,
                        device: None,
                    },
                ))
            }
        }
        Ok(r)
    }
}

pub struct Bus {
    extended: bool,
    pub enabled: bool,
    pub delay: u8,
    pub mask: u16,
    pub interrupts: u64,
    /// Front-panel switch register, read by DIA to CPU device 0o77.
    pub switches: u16,
    pub disk: Disk,
    pub terminal: Terminal,
    pub bank_selectors: BTreeSet<u16>,
    selected: u16,
    setup53: usize,
    probe53: Option<u16>,
    aux40: Vec<u16>,
    aux41: Vec<u16>,
}
impl Bus {
    pub fn new(
        raw: Vec<u8>,
        extended: bool,
        experimental: bool,
        connect_after: Option<u64>,
    ) -> Self {
        Self {
            extended,
            enabled: false,
            delay: 0,
            mask: 0,
            interrupts: 0,
            switches: 0,
            disk: Disk::new(raw),
            terminal: Terminal::new(experimental, connect_after),
            bank_selectors: BTreeSet::new(),
            selected: 0x80,
            setup53: 0,
            probe53: None,
            aux40: vec![],
            aux41: vec![],
        }
    }
    pub fn extended(&self) -> bool {
        self.extended
    }
    pub fn pending(&self) -> Option<u8> {
        if self.disk.done && self.mask & 0x2000 == 0 {
            Some(0o33)
        } else if self.terminal.pending_events() > 0 && self.mask & 0x8000 == 0 {
            Some(0o50)
        } else {
            None
        }
    }
    pub fn io(
        &mut self,
        memory: &mut Memory,
        num: u8,
        op: u8,
        p: u8,
        v: u16,
    ) -> Result<IoResult, Fault> {
        if op > 7 || p > 3 {
            return Err(Fault::new(crate::FaultKind::UnsupportedIo));
        }
        let mut r = IoResult::default();
        match num {
            0o33 => return self.disk.io(memory, self.extended, op, p, v),
            0o50 => {
                if !self.extended && (op, p) != (2, 3) {
                    return Err(Fault::with(
                        crate::FaultKind::UnsupportedSetupOperation,
                        crate::FaultContext::Protocol {
                            operation: op,
                            pulse: p,
                            value: Some(v),
                            device: None,
                        },
                    ));
                }
                return self.terminal.io(memory, op, p, v);
            }
            0o53 => {
                if (op, p) == (2, 3) && self.setup53 < 4 && v == self.setup53 as u16 * 128 {
                    self.setup53 += 1;
                } else if self.extended
                    && (op, p) == (2, 0)
                    && (0x400..0x600).contains(&v)
                    && v.is_multiple_of(128)
                {
                    self.probe53 = Some(v);
                } else if self.extended && (op, p) == (0, 1) && self.probe53.is_some() {
                } else if self.extended && op == 7 {
                    r.skip = [false, true, false, true][p as usize];
                } else {
                    return Err(Fault::with(
                        if self.extended && (op, p) != (2, 3) {
                            crate::FaultKind::UnsupportedTerminalOperation
                        } else {
                            crate::FaultKind::UnsupportedSetupOperation
                        },
                        crate::FaultContext::Protocol {
                            operation: op,
                            pulse: p,
                            value: Some(v),
                            device: None,
                        },
                    ));
                }
            }
            0o77 => {
                if !self.extended {
                    if (op, p) == (5, 2) {
                        self.disk.reset();
                        r.value = Some(v);
                    } else {
                        return Err(Fault::with(
                            crate::FaultKind::UnsupportedIo,
                            crate::FaultContext::Protocol {
                                operation: op,
                                pulse: p,
                                value: None,
                                device: Some(num),
                            },
                        ));
                    }
                } else {
                    if op == 7 {
                        r.skip = [self.enabled, !self.enabled, false, true][p as usize];
                        return Ok(r);
                    }
                    if ![0, 1, 3, 4, 5].contains(&op) {
                        return Err(Fault::new(crate::FaultKind::UnsupportedCpuControl));
                    }
                    if p > 2 {
                        return Err(Fault::new(crate::FaultKind::UnsupportedCpuPulse));
                    }
                    if op == 1 {
                        r.value = Some(self.switches);
                    } else if op == 3 {
                        r.value = Some(self.pending().unwrap_or(0) as u16);
                    } else if op == 4 {
                        self.mask = v;
                    } else if op == 5 {
                        self.disk.reset();
                        self.enabled = false;
                        self.mask = 0;
                        r.value = Some(v);
                    }
                    if p == 1 {
                        self.enabled = true;
                        self.delay = 1;
                    } else if p == 2 {
                        self.enabled = false;
                        self.delay = 0;
                    }
                }
            }
            0o35 if self.extended => {
                if op == 2 && p <= 1 {
                    if v == 0xffff {
                        memory.set_mapping(crate::memory::Mapping::GlobalOnly)?;
                    } else {
                        if (v >> 8) as usize >= memory.banks() {
                            return Err(Fault::with(
                                crate::FaultKind::UnsupportedMemoryBank,
                                crate::FaultContext::Selector { selector: v },
                            ));
                        }
                        memory.set_mapping(crate::memory::Mapping::SelectedBank((v >> 8) as u8))?;
                    }
                    self.selected = v;
                    self.bank_selectors.insert(v);
                } else if (op, p) == (1, 0) {
                    r.value = Some(self.selected);
                } else if (op, p) == (0, 2) {
                } else if op == 7 {
                    r.skip = [false, true, false, true][p as usize];
                } else {
                    return Err(Fault::with(
                        crate::FaultKind::UnsupportedBankOperation,
                        crate::FaultContext::Protocol {
                            operation: op,
                            pulse: p,
                            value: None,
                            device: None,
                        },
                    ));
                }
            }
            0o20 if self.extended => {
                if op == 7 {
                    r.skip = [false, true, false, true][p as usize];
                } else if (op, p) == (2, 1) {
                } else if (op, p) == (1, 2) {
                    r.value = Some(0);
                } else {
                    return Err(Fault::new(
                        crate::FaultKind::UnsupportedOptionalDeviceOperation,
                    ));
                }
            }
            0o40 | 0o41 if self.extended => {
                let values = if num == 0o40 {
                    &mut self.aux40
                } else {
                    &mut self.aux41
                };
                let valid = if num == 0o41 {
                    values.is_empty() && (op, p, v) == (2, 2, 1)
                } else {
                    match values.len() {
                        0 => (op, p, v) == (2, 2, 0x3237),
                        1 => (op, p) == (4, 2),
                        2 => (op, p, v) == (6, 1, 0xffff),
                        _ => false,
                    }
                };
                if !valid {
                    return Err(Fault::new(crate::FaultKind::UnsupportedAuxiliaryOperation));
                }
                values.push(v);
            }
            0o51 if self.extended => {
                if op == 7 {
                    r.skip = [false, true, false, true][p as usize];
                } else if (op, p) == (2, 1)
                    && (0xd80e..0xdc0e).contains(&v)
                    && (v - 0xd80e).is_multiple_of(128)
                {
                } else {
                    return Err(Fault::new(crate::FaultKind::UnsupportedOptional51Operation));
                }
            }
            _ => {
                return Err(Fault::with(
                    crate::FaultKind::UnsupportedIo,
                    crate::FaultContext::Protocol {
                        operation: op,
                        pulse: p,
                        value: None,
                        device: Some(num),
                    },
                ))
            }
        }
        Ok(r)
    }
    pub fn overlay_report(&self, digest: &str) -> Value {
        json!({"base_sha256":digest,"block_bytes":256,"blocks":self.disk.overlay.iter()
            .map(|(offset,raw)|json!({"offset":offset,"hex":crate::machine::hex(raw)})).collect::<Vec<_>>()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlay_round_trip_never_changes_backing_image() {
        let mut cpu = crate::cpu::Cpu::new(16, true).unwrap();
        let mut disk = Disk::new(vec![0; 6144]);
        for n in 0..128 {
            cpu.memory.set(256 + n, n as u16);
        }
        disk.io(&mut cpu.memory, true, 2, 0, 0x1000).unwrap();
        disk.io(&mut cpu.memory, true, 4, 0, 256).unwrap();
        disk.io(&mut cpu.memory, true, 6, 1, 0).unwrap();
        assert!(disk.base.iter().all(|&x| x == 0));
        disk.io(&mut cpu.memory, true, 2, 0, 0).unwrap();
        disk.io(&mut cpu.memory, true, 4, 0, 512).unwrap();
        disk.io(&mut cpu.memory, true, 6, 1, 0).unwrap();
        assert_eq!(cpu.memory.get(639), Some(127));
        assert_eq!((disk.reads, disk.writes), (1, 1));
        assert_eq!(
            disk.io(&mut cpu.memory, true, 3, 0, 0).unwrap().value,
            Some(640)
        );
    }
    #[test]
    fn failed_write_is_atomic_and_control_needs_no_dma() {
        let mut cpu = crate::cpu::Cpu::new(16, true).unwrap();
        let mut disk = Disk::new(vec![0; 6144]);
        disk.a = Some(0x1000);
        disk.b = Some(1000);
        disk.c = Some(0);
        assert_eq!(
            disk.transfer(&mut cpu.memory, true).unwrap_err().reason(),
            "uninitialized_memory"
        );
        assert!(disk.overlay.is_empty());
        assert_eq!(disk.writes, 0);
        disk.a = Some(0x2000);
        disk.b = None;
        disk.c = Some(0x2000);
        disk.transfer(&mut cpu.memory, true).unwrap();
        assert_eq!(disk.controls, 1);
        assert!(disk.done);
    }
    #[test]
    fn strict_setup_and_global_dma_fault_precedence_match_reference() {
        let mut cpu = crate::cpu::Cpu::new(16, true).unwrap();
        let mut bus = Bus::new(vec![0; 6144], false, false, None);
        assert_eq!(
            bus.io(&mut cpu.memory, 0o50, 7, 0, 0).unwrap_err().reason(),
            "unsupported_setup_operation"
        );
        bus.extended = true;
        let f = bus.io(&mut cpu.memory, 0o53, 2, 3, 128).unwrap_err();
        assert_eq!(f.reason().to_string(), "unsupported_setup_operation");
        assert_eq!(f.details()["value"], 128);
        cpu.memory
            .set_mapping(crate::memory::Mapping::GlobalOnly)
            .unwrap();
        bus.disk.b = Some(0xf000);
        assert_eq!(
            bus.disk
                .transfer(&mut cpu.memory, true)
                .unwrap_err()
                .reason(),
            "unmapped_memory_bank"
        );
    }
}
