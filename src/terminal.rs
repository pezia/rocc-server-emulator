#[derive(Debug, Clone, Copy)]
enum TerminalCommand {
    Unset,
    Other(u16),
    Output { dma: Option<u16> },
}
#[derive(Debug, Clone, Copy)]
enum TerminalEvent {
    Connection,
    Key(u8),
    Completion(u16),
    Timer,
}
impl TerminalEvent {
    fn words(self) -> [u16; 3] {
        match self {
            Self::Connection => [0x2000, 0, 0],
            Self::Key(b) => [0x1000, b as u16, 0],
            Self::Completion(s) => [s, 0, 0],
            Self::Timer => [0x6000, 0, 0],
        }
    }
}
#[derive(Debug, Clone, serde::Serialize)]
struct RegisterOutput {
    register: &'static str,
    value: u16,
}
#[derive(Debug, Clone)]
pub(crate) struct DisplayPacket {
    start: Option<usize>,
    parameter: u16,
    raw: Vec<u8>,
}
impl serde::Serialize for DisplayPacket {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(None)?;
        m.serialize_entry(
            "mode",
            if self.start.is_some() {
                "dma"
            } else {
                "immediate"
            },
        )?;
        if let Some(start) = self.start {
            m.serialize_entry("buffer_word", &start)?;
        }
        m.serialize_entry("parameter", &self.parameter)?;
        m.serialize_entry("hex", &hex(&self.raw))?;
        m.end()
    }
}
use crate::config::{CaptureMode, MachineConfig};
use crate::{
    memory::Memory,
    screen::{hex, Screen},
    Fault, IoResult,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};

pub struct Terminal {
    experimental: bool,
    events: VecDeque<TerminalEvent>,
    event_limit: usize,
    packet_limit: usize,
    timer_pending: bool,
    byte_history_truncated: bool,
    packet_history_truncated: bool,
    full_digest: Sha256,
    pub screen: Screen,
    pub capture_tail: bool,
    pub total_display_bytes: u64,
    pub connected: bool,
    pub connect_after: Option<u64>,
    elapsed: u64,
    state: TerminalCommand,
    values: Vec<u16>,
    display_bytes: VecDeque<u8>,
    pub(crate) packets: VecDeque<DisplayPacket>,
    capture_limit: usize,
    capture_truncated: bool,
    outputs: VecDeque<RegisterOutput>,
    timer_period: u64,
    timer_remaining: u64,
    timer_enabled: bool,
    timer_ticks: u64,
    mapping: BTreeMap<u16, u16>,
}
impl Terminal {
    pub fn new(experimental: bool, connect_after: Option<u64>) -> Self {
        Self {
            experimental,
            events: VecDeque::new(),
            event_limit: 4096,
            packet_limit: 4096,
            timer_pending: false,
            byte_history_truncated: false,
            packet_history_truncated: false,
            full_digest: Sha256::new(),
            screen: Screen::new(),
            capture_tail: false,
            total_display_bytes: 0,
            connected: false,
            connect_after,
            elapsed: 0,
            state: TerminalCommand::Unset,
            values: Vec::new(),
            display_bytes: VecDeque::new(),
            packets: VecDeque::new(),
            capture_limit: 1048576,
            capture_truncated: false,
            outputs: VecDeque::new(),
            timer_period: 200000,
            timer_remaining: 0,
            timer_enabled: false,
            timer_ticks: 0,
            mapping: BTreeMap::new(),
        }
    }
    fn command(&self) -> Option<u16> {
        match self.state {
            TerminalCommand::Unset => None,
            TerminalCommand::Other(v) => Some(v),
            TerminalCommand::Output { .. } => Some(0),
        }
    }
    fn dma(&self) -> Option<u16> {
        match self.state {
            TerminalCommand::Output { dma } => dma,
            _ => None,
        }
    }
    fn select_command(&mut self, value: u16) {
        self.state = if value == 0 {
            TerminalCommand::Output { dma: None }
        } else {
            TerminalCommand::Other(value)
        };
    }
    fn clear_dma(&mut self) {
        if let TerminalCommand::Output { dma } = &mut self.state {
            *dma = None;
        }
    }
    pub fn with_config(config: &MachineConfig) -> Self {
        let mut t = Self::new(config.experimental(), config.connect_after());
        let limits = config.limits();
        t.event_limit = limits.events();
        t.capture_limit = limits.bytes();
        t.packet_limit = limits.packets();
        t.capture_tail = config.capture() == CaptureMode::Tail;
        t
    }
    pub fn pending_events(&self) -> usize {
        self.events.len()
    }
    pub fn next_event(&self) -> Option<[u16; 3]> {
        self.events.front().map(|e| e.words())
    }
    fn reserve_events(&self, n: usize) -> Result<(), Fault> {
        if self.events.len() + n > self.event_limit {
            Err(Fault::new(crate::FaultKind::TerminalEventLimit))
        } else {
            Ok(())
        }
    }
    fn pop_event(&mut self) -> Option<TerminalEvent> {
        let e = self.events.pop_front()?;
        if matches!(e, TerminalEvent::Timer) {
            self.timer_pending = false;
        }
        Some(e)
    }
    pub fn tick(&mut self) -> Result<(), Fault> {
        if !self.experimental {
            return Ok(());
        }
        self.elapsed += 1;
        let connect = !self.connected && self.connect_after.is_some_and(|n| self.elapsed >= n);
        let timer = self.timer_enabled && self.timer_remaining == 1;
        self.reserve_events(usize::from(connect) + usize::from(timer && !self.timer_pending))?;
        if connect {
            self.events.push_back(TerminalEvent::Connection);
            self.connected = true;
        }
        if self.timer_enabled {
            self.timer_remaining -= 1;
            if timer {
                self.timer_remaining = self.timer_period;
                self.timer_ticks += 1;
                if !self.timer_pending {
                    self.events.push_back(TerminalEvent::Timer);
                    self.timer_pending = true;
                }
            }
        }
        Ok(())
    }
    pub fn connect(&mut self) -> Result<(), Fault> {
        if !self.connected {
            self.reserve_events(1)?;
            self.events.push_back(TerminalEvent::Connection);
            self.connected = true;
        }
        Ok(())
    }
    pub fn send_key(&mut self, value: u8) -> Result<(), Fault> {
        if !self.connected {
            return Err(Fault::new(crate::FaultKind::TerminalIsNotConnected));
        }
        self.reserve_events(1)?;
        self.events.push_back(TerminalEvent::Key(value));
        Ok(())
    }
    fn acknowledge(&mut self, status: u16) {
        self.events.push_back(TerminalEvent::Completion(status));
    }
    fn output(&mut self, register: &'static str, value: u16) {
        if self.outputs.len() == 256 {
            self.outputs.pop_front();
        }
        self.outputs.push_back(RegisterOutput { register, value });
    }
    fn capture(
        &mut self,
        raw: &[u8],
        start: Option<usize>,
        parameter: u16,
    ) -> Result<IoResult, Fault> {
        self.reserve_events(1)?;
        let bytes_over = self.display_bytes.len() + raw.len() > self.capture_limit;
        let packets_over = self.packets.len() >= self.packet_limit;
        if !self.capture_tail && (bytes_over || packets_over) {
            return Err(Fault::new(crate::FaultKind::TerminalCaptureLimit));
        }
        if bytes_over {
            self.byte_history_truncated = true;
        }
        if packets_over {
            self.packet_history_truncated = true;
            self.packets.pop_front();
        }
        // Drain from VecDeque advances the head without moving retained bytes.
        let remove = (self.display_bytes.len() + raw.len())
            .saturating_sub(self.capture_limit)
            .min(self.display_bytes.len());
        self.display_bytes.drain(..remove);
        self.display_bytes.extend(
            raw[raw.len().saturating_sub(self.capture_limit)..]
                .iter()
                .copied(),
        );
        self.capture_truncated = self.byte_history_truncated || self.packet_history_truncated;
        self.full_digest.update(raw);
        self.total_display_bytes += raw.len() as u64;
        self.screen.feed(raw);
        self.packets.push_back(DisplayPacket {
            start,
            parameter,
            raw: raw.to_vec(),
        });
        self.clear_dma();
        self.acknowledge(0);
        Ok(IoResult::default())
    }
    pub fn full_stream_sha256(&self) -> String {
        crate::machine::hex(&self.full_digest.clone().finalize())
    }
    pub fn retained_sha256(&self) -> String {
        let mut d = Sha256::new();
        let (a, b) = self.display_bytes.as_slices();
        d.update(a);
        d.update(b);
        crate::machine::hex(&d.finalize())
    }
    fn transfer(&mut self, memory: &Memory, value: u16) -> Result<IoResult, Fault> {
        self.reserve_events(1)?;
        // The original echo routine leaves a stale B; mode zero is immediate.
        if value < 256 {
            let raw = [value as u8];
            return self.capture(&raw, None, value);
        }
        if value & 0xf800 != 0xe000 || value & 0x7ff == 0 {
            return Err(Fault::with(
                crate::FaultKind::UnsupportedTerminalDmaMode,
                json!({"value":value}),
            ));
        }
        let count = (value & 0x7ff) as usize;
        let start = self
            .dma()
            .ok_or_else(|| Fault::new(crate::FaultKind::UnsupportedTerminalDmaMode))?
            as usize;
        let end = start + count.div_ceil(2);
        if end > memory.global_words() {
            return Err(Fault::new(crate::FaultKind::TerminalDmaOutsideSharedMemory));
        }
        let mut raw = Vec::with_capacity(count + 1);
        for address in start..end {
            let word = memory
                .get(address)
                .ok_or_else(|| Fault::new(crate::FaultKind::TerminalDmaUninitialized))?;
            raw.extend_from_slice(&word.to_be_bytes());
        }
        raw.truncate(count);
        self.capture(&raw, Some(start), value)
    }
    pub fn io(
        &mut self,
        memory: &Memory,
        operation: u8,
        pulse: u8,
        value: u16,
    ) -> Result<IoResult, Fault> {
        let details = || json!({"operation":operation,"pulse":pulse,"value":value});
        let needs_ack = match (operation, pulse) {
            (2, 0) => {
                value == 0xe000 || value == 0xf001 || value == 0xe804 || value & 0xfc0f == 0xc001
            }
            (0, 1) => self.command() == Some(0x400),
            (6, 0) => {
                self.command() == Some(0xe001) && value != 0
                    || self.command().is_some_and(|c| c & 0xfc0f == 0xc000)
            }
            _ => false,
        };
        if self.experimental && needs_ack {
            self.reserve_events(1)?;
        }
        if (operation, pulse) == (2, 3) {
            if self.values.len() >= 8 || value != self.values.len() as u16 * 128 {
                return Err(Fault::with(
                    crate::FaultKind::UnsupportedSetupOperation,
                    details(),
                ));
            }
            self.values.push(value);
            return Ok(IoResult::default());
        }
        if !self.experimental {
            if (operation, pulse) == (2, 0)
                && (0x400..0x800).contains(&value)
                && value.is_multiple_of(128)
            {
                self.select_command(value);
                return Ok(IoResult::default());
            }
            if (operation, pulse) == (0, 1) && self.command().is_some() {
                return Ok(IoResult::default());
            }
            if operation == 7 && pulse < 4 {
                return Ok(IoResult {
                    skip: matches!(pulse, 1 | 3),
                    value: None,
                });
            }
            return Err(Fault::with(
                crate::FaultKind::UnsupportedTerminalOperation,
                details(),
            ));
        }
        if operation == 7 && pulse < 4 {
            return Ok(IoResult {
                skip: match pulse {
                    0 => false,
                    1 => true,
                    2 => !self.events.is_empty(),
                    _ => self.events.is_empty(),
                },
                value: None,
            });
        }
        if matches!(operation, 1 | 3 | 5) && matches!(pulse, 0 | 2) {
            let event = self
                .events
                .front()
                .ok_or_else(|| Fault::new(crate::FaultKind::TerminalInputWithoutEvent))?;
            let result = event.words()[((operation - 1) / 2) as usize];
            if pulse == 2 {
                self.pop_event();
            }
            return Ok(IoResult {
                value: Some(result),
                skip: false,
            });
        }
        if (operation, pulse) == (0, 2) {
            self.pop_event();
            return Ok(IoResult::default());
        }
        if (operation, pulse) == (0, 1)
            && self
                .command()
                .is_some_and(|c| (0x400..0x800).contains(&c) && c % 128 == 0)
        {
            if self.command() == Some(0x400) {
                self.acknowledge(0);
            }
            return Ok(IoResult::default());
        }
        if (operation, pulse) == (2, 0) {
            let known = value < 0x800
                || matches!(value, 0xe000 | 0xe001 | 0xe804 | 0xf001)
                || matches!(value & 0xfc0f, 0xc000 | 0xc001)
                || matches!(value & !0x400, 0xe000 | 0xe001 | 0xe804 | 0xf001)
                || matches!(value & 0xf80f, 0xc000 | 0xc001);
            if !known {
                return Err(Fault::with(
                    crate::FaultKind::UnsupportedTerminalCommand,
                    json!({"command":value}),
                ));
            }
            self.clear_dma();
            self.select_command(value);
            self.output("A", value);
            if value == 0xe000 {
                self.timer_enabled = false;
                self.acknowledge(0x4000);
            } else if value == 0xf001 {
                self.timer_enabled = true;
                self.timer_remaining = self.timer_period;
                self.acknowledge(0x4001);
            } else if value == 0xe804 || value & 0xfc0f == 0xc001 {
                self.acknowledge(0x4000 | (value & 0x3ff));
            }
            return Ok(IoResult::default());
        }
        if (operation, pulse) == (4, 0) && self.command() == Some(0) {
            if let TerminalCommand::Output { dma } = &mut self.state {
                *dma = Some(value);
            }
            return Ok(IoResult::default());
        }
        if (operation, pulse) == (6, 0) && self.command() == Some(0) && self.dma().is_some() {
            return self.transfer(memory, value);
        }
        if (operation, pulse) == (6, 0) {
            if self.command() == Some(0xe001) && value != 0 {
                self.timer_period = value as u64 * 1000;
                self.timer_remaining = self.timer_period;
                self.timer_enabled = true;
                self.acknowledge(0x4001);
            } else if self.command().is_some_and(|c| c & 0xfc0f == 0xc000) {
                self.acknowledge(0x4000 | (self.command().unwrap_or_default() & 0x3ff));
            } else if self.command().is_some_and(|c| c < 0x400) && value == 0x800 {
                self.mapping
                    .insert(self.command().unwrap_or_default(), value);
            } else {
                return Err(Fault::with(
                    crate::FaultKind::UnsupportedTerminalParameter,
                    json!({"command":self.command(),"value":value}),
                ));
            }
            self.output("C", value);
            return Ok(IoResult::default());
        }
        Err(Fault::with(
            crate::FaultKind::UnsupportedTerminalOperation,
            details(),
        ))
    }
    pub fn snapshot(&self) -> Value {
        if !self.experimental {
            return json!({"values":self.values,"command":self.command(),"present_channels":[]});
        }
        json!({"connected":self.connected,"display_byte_count":self.display_bytes.len(),"display_packets":self.packets.len(),"command":self.command(),"pending_events":self.events.len(),"next_event":self.next_event(),"timer_enabled":self.timer_enabled,"timer_period":self.timer_period,"timer_ticks":self.timer_ticks,"present_channels":[0]})
    }
    pub fn display_export(&self) -> DisplayExport<'_> {
        DisplayExport(self)
    }
    pub fn display_report(&self) -> Value {
        serde_json::to_value(self.display_export()).expect("display report")
    }
}

pub struct DisplayExport<'a>(&'a Terminal);
impl serde::Serialize for DisplayExport<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let t = self.0;
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("packets", &t.packets)?;
        m.serialize_entry("screen", &t.screen.snapshot())?;
        m.serialize_entry("capture_truncated", &t.capture_truncated)?;
        m.serialize_entry("byte_history_truncated", &t.byte_history_truncated)?;
        m.serialize_entry("packet_history_truncated", &t.packet_history_truncated)?;
        m.serialize_entry("total_bytes", &t.total_display_bytes)?;
        m.serialize_entry("connect_after", &t.connect_after)?;
        m.serialize_entry("sha256", &t.retained_sha256())?;
        m.serialize_entry("retained_sha256", &t.retained_sha256())?;
        m.serialize_entry("full_stream_sha256", &t.full_stream_sha256())?;
        m.serialize_entry("digest_scope",&"sha256 and retained_sha256 cover retained bytes; full_stream_sha256 covers all output bytes")?;
        m.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn immediate_echo_ignores_stale_dma() {
        let m = crate::cpu::Cpu::new(16, true).unwrap().memory;
        let mut t = Terminal::new(true, None);
        t.io(&m, 2, 0, 0).unwrap();
        t.io(&m, 4, 0, 0xed07).unwrap();
        t.io(&m, 6, 0, 65).unwrap();
        assert_eq!(t.display_report()["packets"][0]["hex"], "41");
        assert_eq!(t.pop_event().map(|e| e.words()), Some([0, 0, 0]));
    }
    #[test]
    fn connection_precedes_coalesced_timer_event() {
        let m = crate::cpu::Cpu::new(16, true).unwrap().memory;
        let mut t = Terminal::new(true, Some(1000));
        t.io(&m, 2, 0, 0xe001).unwrap();
        t.io(&m, 6, 0, 1).unwrap();
        t.events.clear();
        for _ in 0..2000 {
            t.tick().unwrap();
        }
        assert_eq!(
            t.events.iter().map(|e| e.words()).collect::<Vec<_>>(),
            vec![[0x2000, 0, 0], [0x6000, 0, 0]]
        );
        assert_eq!(t.snapshot()["timer_ticks"], 2);
    }
    #[test]
    fn capture_failure_is_atomic_and_live_tail_keeps_screen() {
        let mut t = Terminal::new(true, None);
        t.capture_limit = 4;
        t.capture(b"ABCD", None, 0).unwrap();
        t.events.clear();
        assert_eq!(
            t.capture(b"EF", None, 0).unwrap_err().reason(),
            "terminal_capture_limit"
        );
        assert_eq!(t.total_display_bytes, 4);
        assert!(t.events.is_empty());
        t.capture_tail = true;
        t.capture(b"EF", None, 0).unwrap();
        assert_eq!(t.display_bytes.iter().copied().collect::<Vec<_>>(), b"CDEF");
        assert_eq!(t.screen.snapshot()["lines"][0], "ABCDEF");
    }
    #[test]
    fn rejects_bad_dma_without_ack_and_busy_queries_do_not_execute() {
        let m = crate::cpu::Cpu::new(16, true).unwrap().memory;
        let mut t = Terminal::new(true, None);
        for command in [0xe400, 0xe401, 0xec04, 0xf401, 0xc401] {
            t.io(&m, 2, 0, command).unwrap();
        }
        assert!(t.events.is_empty());
        assert!(!t.timer_enabled);
        t.io(&m, 2, 0, 0).unwrap();
        t.io(&m, 4, 0, 200).unwrap();
        assert_eq!(
            t.io(&m, 6, 0, 0xe004).unwrap_err().reason(),
            "terminal_dma_uninitialized"
        );
        assert!(t.events.is_empty());
        assert_eq!(t.total_display_bytes, 0);
    }
    #[test]
    fn live_packet_history_and_byte_history_have_independent_bounds() {
        let mut t = Terminal::new(true, None);
        t.capture_tail = true;
        for _ in 0..4097 {
            t.capture(b"X", None, 0).unwrap();
            t.pop_event();
        }
        assert_eq!(t.packets.len(), 4096);
        assert!(t.capture_truncated);
        assert_eq!(t.total_display_bytes, 4097);
        let raw = vec![b'Y'; 1048577];
        t.capture(&raw, None, 0).unwrap();
        assert_eq!(t.display_bytes.len(), 1048576);
        assert_eq!(t.total_display_bytes, 1052674);
        assert_eq!(t.screen.column, 1052674);
    }
    #[test]
    fn dma_is_big_endian_and_event_reads_preserve_tuple_until_clear() {
        let mut m = crate::cpu::Cpu::new(16, true).unwrap().memory;
        m.set(200, 0x4142);
        m.set(201, 0x4300);
        let mut t = Terminal::new(true, None);
        t.io(&m, 2, 0, 0).unwrap();
        t.io(&m, 4, 0, 200).unwrap();
        t.io(&m, 6, 0, 0xe003).unwrap();
        assert_eq!(t.display_bytes.iter().copied().collect::<Vec<_>>(), b"ABC");
        t.events.clear();
        assert!(t.send_key(65).is_err());
        t.connect().unwrap();
        t.events.clear();
        t.send_key(65).unwrap();
        assert_eq!(t.io(&m, 1, 0, 0).unwrap().value, Some(0x1000));
        assert_eq!(t.io(&m, 3, 0, 0).unwrap().value, Some(65));
        assert_eq!(t.io(&m, 5, 2, 0).unwrap().value, Some(0));
        assert!(t.events.is_empty());
    }
    #[test]
    fn probe_only_accepts_eight_setup_values_and_absent_channels() {
        let m = crate::cpu::Cpu::new(16, true).unwrap().memory;
        let mut t = Terminal::new(false, None);
        for n in 0..8 {
            t.io(&m, 2, 3, n * 128).unwrap();
        }
        assert_eq!(
            t.io(&m, 2, 3, 1024).unwrap_err().reason(),
            "unsupported_setup_operation"
        );
        t.io(&m, 2, 0, 0x780).unwrap();
        t.io(&m, 0, 1, 0).unwrap();
        assert!(!t.io(&m, 7, 2, 0).unwrap().skip);
        assert!(t.io(&m, 2, 0, 0xe000).is_err());
    }
}
