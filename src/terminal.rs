use crate::{
    memory::Memory,
    screen::{hex, Screen},
    Fault, IoResult,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};

pub struct Terminal {
    experimental: bool,
    pub events: VecDeque<[u16; 3]>,
    pub screen: Screen,
    pub capture_tail: bool,
    pub total_display_bytes: u64,
    pub connected: bool,
    pub connect_after: Option<u64>,
    elapsed: u64,
    command: Option<u16>,
    dma: Option<u16>,
    values: Vec<u16>,
    display_bytes: Vec<u8>,
    packets: VecDeque<Value>,
    capture_limit: usize,
    capture_truncated: bool,
    outputs: VecDeque<Value>,
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
            screen: Screen::new(),
            capture_tail: false,
            total_display_bytes: 0,
            connected: false,
            connect_after,
            elapsed: 0,
            command: None,
            dma: None,
            values: Vec::new(),
            display_bytes: Vec::new(),
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
    pub fn tick(&mut self) {
        if !self.experimental {
            return;
        }
        self.elapsed += 1;
        if !self.connected && self.connect_after.is_some_and(|n| self.elapsed >= n) {
            self.connect();
        }
        if !self.timer_enabled {
            return;
        }
        self.timer_remaining -= 1;
        if self.timer_remaining == 0 {
            self.timer_remaining = self.timer_period;
            self.timer_ticks += 1;
            if !self.events.iter().any(|event| event[0] == 0x6000) {
                self.events.push_back([0x6000, 0, 0]);
            }
        }
    }
    pub fn connect(&mut self) {
        if !self.connected {
            self.events.push_back([0x2000, 0, 0]);
            self.connected = true;
        }
    }
    pub fn send_key(&mut self, value: u8) -> Result<(), Fault> {
        if !self.connected {
            return Err(Fault::new(crate::FaultKind::TerminalIsNotConnected));
        }
        self.events.push_back([0x1000, value as u16, 0]);
        Ok(())
    }
    fn acknowledge(&mut self, status: u16) {
        self.events.push_back([status, 0, 0]);
    }
    fn output(&mut self, register: &str, value: u16) {
        if self.outputs.len() == 256 {
            self.outputs.pop_front();
        }
        self.outputs
            .push_back(json!({"register":register,"value":value}));
    }
    fn capture(&mut self, raw: &[u8], packet: Value) -> Result<IoResult, Fault> {
        if self.display_bytes.len() + raw.len() > self.capture_limit {
            if !self.capture_tail {
                return Err(Fault::new(crate::FaultKind::TerminalCaptureLimit));
            }
            let remove = (self.display_bytes.len() + raw.len() - self.capture_limit)
                .min(self.display_bytes.len());
            self.display_bytes.drain(..remove);
            self.capture_truncated = true;
        }
        self.display_bytes.extend_from_slice(raw);
        if self.display_bytes.len() > self.capture_limit {
            let remove = self.display_bytes.len() - self.capture_limit;
            self.display_bytes.drain(..remove);
        }
        if self.capture_tail && self.packets.len() >= 4096 {
            while self.packets.len() >= 4096 {
                self.packets.pop_front();
            }
            self.capture_truncated = true;
        }
        self.total_display_bytes += raw.len() as u64;
        self.screen.feed(raw);
        self.packets.push_back(packet);
        self.dma = None;
        self.acknowledge(0);
        Ok(IoResult::default())
    }
    fn transfer(&mut self, memory: &Memory, value: u16) -> Result<IoResult, Fault> {
        // The original echo routine leaves a stale B; mode zero is immediate.
        if value < 256 {
            let raw = [value as u8];
            return self.capture(
                &raw,
                json!({"mode":"immediate","parameter":value,"hex":hex(&raw)}),
            );
        }
        if value & 0xf800 != 0xe000 || value & 0x7ff == 0 {
            return Err(Fault::with(
                crate::FaultKind::UnsupportedTerminalDmaMode,
                json!({"value":value}),
            ));
        }
        let count = (value & 0x7ff) as usize;
        let start = self.dma.unwrap() as usize;
        let end = start + count.div_ceil(2);
        if end > memory.global_words {
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
        self.capture(
            &raw,
            json!({"mode":"dma","buffer_word":start,"parameter":value,"hex":hex(&raw)}),
        )
    }
    pub fn io(
        &mut self,
        memory: &Memory,
        operation: u8,
        pulse: u8,
        value: u16,
    ) -> Result<IoResult, Fault> {
        let details = json!({"operation":operation,"pulse":pulse,"value":value});
        if (operation, pulse) == (2, 3) {
            if self.values.len() >= 8 || value != self.values.len() as u16 * 128 {
                return Err(Fault::with(
                    crate::FaultKind::UnsupportedSetupOperation,
                    details,
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
                self.command = Some(value);
                return Ok(IoResult::default());
            }
            if (operation, pulse) == (0, 1) && self.command.is_some() {
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
                details,
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
            let result = event[((operation - 1) / 2) as usize];
            if pulse == 2 {
                self.events.pop_front();
            }
            return Ok(IoResult {
                value: Some(result),
                skip: false,
            });
        }
        if (operation, pulse) == (0, 2) {
            self.events.pop_front();
            return Ok(IoResult::default());
        }
        if (operation, pulse) == (0, 1)
            && self
                .command
                .is_some_and(|c| (0x400..0x800).contains(&c) && c % 128 == 0)
        {
            if self.command == Some(0x400) {
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
            self.dma = None;
            self.command = Some(value);
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
        if (operation, pulse) == (4, 0) && self.command == Some(0) {
            self.dma = Some(value);
            return Ok(IoResult::default());
        }
        if (operation, pulse) == (6, 0) && self.command == Some(0) && self.dma.is_some() {
            return self.transfer(memory, value);
        }
        if (operation, pulse) == (6, 0) {
            if self.command == Some(0xe001) && value != 0 {
                self.timer_period = value as u64 * 1000;
                self.timer_remaining = self.timer_period;
                self.timer_enabled = true;
                self.acknowledge(0x4001);
            } else if self.command.is_some_and(|c| c & 0xfc0f == 0xc000) {
                self.acknowledge(0x4000 | (self.command.unwrap() & 0x3ff));
            } else if self.command.is_some_and(|c| c < 0x400) && value == 0x800 {
                self.mapping.insert(self.command.unwrap(), value);
            } else {
                return Err(Fault::with(
                    crate::FaultKind::UnsupportedTerminalParameter,
                    json!({"command":self.command,"value":value}),
                ));
            }
            self.output("C", value);
            return Ok(IoResult::default());
        }
        Err(Fault::with(
            crate::FaultKind::UnsupportedTerminalOperation,
            details,
        ))
    }
    pub fn snapshot(&self) -> Value {
        if !self.experimental {
            return json!({"values":self.values,"command":self.command,"present_channels":[]});
        }
        json!({"connected":self.connected,"display_byte_count":self.display_bytes.len(),"display_packets":self.packets.len(),"command":self.command,"pending_events":self.events.len(),"next_event":self.events.front(),"timer_enabled":self.timer_enabled,"timer_period":self.timer_period,"timer_ticks":self.timer_ticks,"present_channels":[0]})
    }
    pub fn display_report(&self) -> Value {
        json!({"packets":self.packets,"screen":self.screen.snapshot(),"capture_truncated":self.capture_truncated,"total_bytes":self.total_display_bytes,"connect_after":self.connect_after,"sha256":crate::machine::digest(&self.display_bytes)})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn immediate_echo_ignores_stale_dma() {
        let m = Memory::new(16, true);
        let mut t = Terminal::new(true, None);
        t.io(&m, 2, 0, 0).unwrap();
        t.io(&m, 4, 0, 0xed07).unwrap();
        t.io(&m, 6, 0, 65).unwrap();
        assert_eq!(t.display_report()["packets"][0]["hex"], "41");
        assert_eq!(t.events.pop_front(), Some([0, 0, 0]));
    }
    #[test]
    fn connection_precedes_coalesced_timer_event() {
        let m = Memory::new(16, true);
        let mut t = Terminal::new(true, Some(1000));
        t.io(&m, 2, 0, 0xe001).unwrap();
        t.io(&m, 6, 0, 1).unwrap();
        t.events.clear();
        for _ in 0..2000 {
            t.tick();
        }
        assert_eq!(
            t.events.iter().copied().collect::<Vec<_>>(),
            vec![[0x2000, 0, 0], [0x6000, 0, 0]]
        );
        assert_eq!(t.snapshot()["timer_ticks"], 2);
    }
    #[test]
    fn capture_failure_is_atomic_and_live_tail_keeps_screen() {
        let mut t = Terminal::new(true, None);
        t.capture_limit = 4;
        t.capture(b"ABCD", json!({})).unwrap();
        t.events.clear();
        assert_eq!(
            t.capture(b"EF", json!({})).unwrap_err().reason(),
            "terminal_capture_limit"
        );
        assert_eq!(t.total_display_bytes, 4);
        assert!(t.events.is_empty());
        t.capture_tail = true;
        t.capture(b"EF", json!({})).unwrap();
        assert_eq!(t.display_bytes, b"CDEF");
        assert_eq!(t.screen.snapshot()["lines"][0], "ABCDEF");
    }
    #[test]
    fn rejects_bad_dma_without_ack_and_busy_queries_do_not_execute() {
        let m = Memory::new(16, true);
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
            t.capture(b"X", json!({"hex":"58"})).unwrap();
        }
        assert_eq!(t.packets.len(), 4096);
        assert!(t.capture_truncated);
        assert_eq!(t.total_display_bytes, 4097);
        let raw = vec![b'Y'; 1048577];
        t.capture(&raw, json!({})).unwrap();
        assert_eq!(t.display_bytes.len(), 1048576);
        assert_eq!(t.total_display_bytes, 1052674);
        assert_eq!(t.screen.column, 1052674);
    }
    #[test]
    fn dma_is_big_endian_and_event_reads_preserve_tuple_until_clear() {
        let mut m = Memory::new(16, true);
        m.set(200, 0x4142);
        m.set(201, 0x4300);
        let mut t = Terminal::new(true, None);
        t.io(&m, 2, 0, 0).unwrap();
        t.io(&m, 4, 0, 200).unwrap();
        t.io(&m, 6, 0, 0xe003).unwrap();
        assert_eq!(t.display_bytes, b"ABC");
        t.events.clear();
        assert!(t.send_key(65).is_err());
        t.connect();
        t.events.clear();
        t.send_key(65).unwrap();
        assert_eq!(t.io(&m, 1, 0, 0).unwrap().value, Some(0x1000));
        assert_eq!(t.io(&m, 3, 0, 0).unwrap().value, Some(65));
        assert_eq!(t.io(&m, 5, 2, 0).unwrap().value, Some(0));
        assert!(t.events.is_empty());
    }
    #[test]
    fn probe_only_accepts_eight_setup_values_and_absent_channels() {
        let m = Memory::new(16, true);
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
