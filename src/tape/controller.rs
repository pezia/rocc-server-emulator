use super::{
    medium::{Direction, TapeObject, TapeRead, TapeStep},
    session::TapeSession,
    TapeEvent, TapeStatus,
};
use crate::{memory::Memory, Fault, FaultContext, FaultKind, IoResult};

pub(crate) struct TapeController {
    session: TapeSession,
    command: u16,
    address: Option<u16>,
    count: Option<u16>,
    pending: bool,
    pub(crate) busy: bool,
    pub(crate) done: bool,
    result: u16,
    flags: u16,
    flags2: u16,
    status: TapeStatus,
    activity: (u64, &'static str),
}
impl TapeController {
    pub(crate) fn new(session: TapeSession) -> Self {
        let marks = session
            .medium
            .objects
            .iter()
            .filter(|x| matches!(x, TapeObject::Mark))
            .count();
        let status = TapeStatus {
            mode: if session.writable {
                "new_writable"
            } else {
                "read_only"
            }
            .into(),
            unit: 1,
            initial_sha256: session.initial_sha256.clone(),
            final_sha256: None,
            records: session.medium.objects.len() - marks,
            marks,
            position: 0,
            reads: 0,
            writes: 0,
            error: None,
            recent: Default::default(),
        };
        Self {
            session,
            command: 0,
            address: None,
            count: None,
            pending: false,
            busy: false,
            done: false,
            result: 0,
            flags: 0,
            flags2: 0,
            status,
            activity: (0, "IDLE"),
        }
    }
    /// Host activity telemetry survives the bounded polling-event history.
    pub(crate) fn activity(&self) -> (u64, &'static str) {
        self.activity
    }
    fn protocol(op: u8, pulse: u8, value: u16) -> Fault {
        Fault::with(
            FaultKind::UnsupportedTapeOperation,
            FaultContext::Protocol {
                operation: op,
                pulse,
                value: Some(value),
                device: Some(0o20),
            },
        )
    }
    fn status_word(&self) -> u16 {
        let mut v = 0x0840 | self.flags;
        if self.command & 7 == 0 {
            v |= 1;
            if self.session.medium.position() == 0 {
                v |= 0x80;
            }
            if !self.session.writable {
                v |= 4;
            }
        }
        v
    }
    pub(crate) fn reset(&mut self) {
        self.command = 0;
        self.address = None;
        self.count = None;
        self.pending = false;
        self.busy = false;
        self.done = false;
        self.flags = 0;
        self.flags2 = 0;
        self.refresh();
    }
    pub(crate) fn io(
        &mut self,
        memory: &mut Memory,
        op: u8,
        pulse: u8,
        value: u16,
    ) -> Result<IoResult, Fault> {
        let _ = memory;
        if op > 7 || pulse > 3 || pulse == 3 && op != 7 {
            return Err(Self::protocol(op, pulse, value));
        }
        let mut r = IoResult::default();
        if op == 7 {
            r.skip = [self.busy, !self.busy, self.done, !self.done][pulse as usize];
        } else {
            match op {
                0 => {}
                1 => r.value = Some(self.status_word()),
                2 => {
                    if pulse != 2
                        && (value & !0x083f != 0 || value & 0x0800 != 0 && value & 0x38 != 0x10)
                    {
                        return Err(Self::protocol(op, pulse, value));
                    }
                    self.command = value;
                }
                3 => r.value = Some(self.result),
                4 => {
                    self.address = Some(value);
                    self.result = value;
                }
                5 => r.value = Some(self.flags2),
                6 => self.count = Some(value),
                _ => return Err(Self::protocol(op, pulse, value)),
            }
            if pulse == 2 {
                self.reset();
            } else if pulse == 1 {
                if self.busy {
                    return Err(Self::protocol(op, pulse, value));
                }
                self.flags = 0;
                self.flags2 = 0;
                self.busy = true;
                self.done = false;
                self.pending = true;
                // DOAS spacing commands in the recovered retry routines mean one record.
                if op == 2 && matches!(self.command & 0x38, 0x18 | 0x20) {
                    self.count = Some(0xffff);
                }
            }
        }
        self.refresh();
        if self.status.recent.len() == 256 {
            self.status.recent.pop_front();
        }
        self.status.recent.push_back(TapeEvent {
            operation: op,
            pulse,
            value,
            input: r.value,
            skip: r.skip,
            busy: self.busy,
            done: self.done,
        });
        Ok(r)
    }
    fn words(&self) -> Result<usize, Fault> {
        let count = self
            .count
            .ok_or_else(|| Fault::new(FaultKind::TapeRegisterUninitialized))?;
        if count & 0x7fff == 0 && self.command & 0x38 == 0 {
            // Whole-record mode validates the indexed record before consuming it.
            let n = match self
                .session
                .medium
                .objects
                .get(self.session.medium.position())
            {
                Some(TapeObject::Data { length, .. }) => (*length as usize).div_ceil(2),
                Some(TapeObject::Mark) | None => 0,
            };
            if n > 4096 {
                return Err(Self::protocol(6, 1, count));
            }
            return Ok(n);
        }
        let n = (0x8000u32 - u32::from(count & 0x7fff)) as usize;
        if !(1..=4096).contains(&n) {
            return Err(Self::protocol(6, 1, count));
        }
        Ok(n)
    }
    fn media_fault(&mut self, error: String) -> Fault {
        self.status.error = Some(error.clone());
        Fault::with(
            FaultKind::TapeMedia,
            FaultContext::Persistence {
                offset: self.session.medium.end as usize,
                error,
            },
        )
    }
    pub(crate) fn tick(&mut self, memory: &mut Memory) -> Result<(), Fault> {
        if !self.pending {
            return Ok(());
        }
        self.pending = false;
        let result = self.complete(memory);
        self.busy = false;
        self.done = result.is_ok();
        self.activity = (
            self.activity.0.wrapping_add(1),
            if result.is_err() || self.flags & 0x1000 != 0 {
                "ERROR"
            } else {
                match self.command & 0x38 {
                    0 => "READ",
                    8 => "REWIND",
                    0x10 => "SETUP",
                    0x18 | 0x20 => "SPACE",
                    0x28 => "WRITE",
                    0x30 => "MARK",
                    0x38 => "ERASE",
                    _ => unreachable!(),
                }
            },
        );
        if let Err(ref e) = result {
            if self.status.error.is_none() {
                self.status.error = Some(e.reason().into());
            }
        }
        self.refresh();
        result
    }
    fn complete(&mut self, memory: &mut Memory) -> Result<(), Fault> {
        if self.command & 7 != 0 {
            self.flags = 0x9000;
            return Ok(());
        }
        match self.command & 0x38 {
            0 | 0x28 => {
                let n = self.words()?;
                let start = self
                    .address
                    .ok_or_else(|| Fault::new(FaultKind::TapeRegisterUninitialized))?
                    as usize;
                if n > 0 {
                    memory.validate_range(start, n, memory.mapping())?;
                }
                if self.command & 0x38 == 0x28 {
                    if !self.session.writable {
                        self.flags = 0x9000;
                        return Ok(());
                    }
                    let mut bytes = Vec::with_capacity(n * 2);
                    for i in 0..n {
                        bytes.extend_from_slice(&memory.read(start + i)?.to_be_bytes());
                    }
                    self.session
                        .write_record(&bytes)
                        .map_err(|e| self.media_fault(e))?;
                    self.result = (start + n) as u16;
                    self.count = Some(0);
                    self.status.writes += 1;
                    self.status.records += 1;
                } else {
                    match self.session.read().map_err(|e| self.media_fault(e))? {
                        TapeRead::Data(bytes) => {
                            let transferred = n.min(bytes.len().div_ceil(2));
                            if bytes.len() > n * 2 {
                                self.flags2 |= 0x80;
                            }
                            if bytes.len() & 1 != 0 {
                                self.flags |= 0x8002;
                            }
                            for i in 0..transferred {
                                memory.write(
                                    start + i,
                                    u16::from_be_bytes([
                                        bytes[2 * i],
                                        bytes.get(2 * i + 1).copied().unwrap_or(0),
                                    ]),
                                )?;
                            }
                            self.result = (start + transferred) as u16;
                            self.count = Some((-(n as i32 - transferred as i32)) as u16);
                        }
                        TapeRead::Mark => self.flags |= 0x0200,
                        TapeRead::End => self.flags |= 0x0200,
                    }
                    self.status.reads += 1;
                }
            }
            8 => {
                self.session.rewind();
            }
            0x10 => {}
            0x18 | 0x20 => {
                let direction = if self.command & 0x38 == 0x18 {
                    Direction::Forward
                } else {
                    Direction::Reverse
                };
                for _ in 0..self.words()? {
                    match self
                        .session
                        .space(direction)
                        .map_err(|e| self.media_fault(e))?
                    {
                        TapeStep::Record { .. } => {}
                        TapeStep::Mark => {
                            self.flags |= 0x0200;
                            break;
                        }
                        TapeStep::End => {
                            self.flags |= if matches!(direction, Direction::Forward) {
                                0x8200
                            } else {
                                0x8080
                            };
                            break;
                        }
                    }
                }
            }
            0x30 => {
                if !self.session.writable {
                    self.flags = 0x9000;
                    return Ok(());
                }
                self.session.write_mark().map_err(|e| self.media_fault(e))?;
                self.status.writes += 1;
                self.status.marks += 1;
            }
            0x38 => {
                if !self.session.writable {
                    self.flags = 0x9000;
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }
    fn refresh(&mut self) {
        self.status.unit = (self.command & 7) as u8 + 1;
        self.status.position = self.session.medium.position();
    }
    pub(crate) fn status(&self) -> &TapeStatus {
        &self.status
    }
    pub(crate) fn finish(&mut self) -> Result<(), String> {
        self.status.final_sha256 = Some(self.session.finish()?);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::session::TapeSession;
    use super::super::{session::TapeProtectedPaths, TapeMount};
    use super::*;
    use crate::{memory::Memory, IoResult};
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    struct TestTape {
        dir: PathBuf,
        controller: TapeController,
        memory: Memory,
    }
    impl TestTape {
        fn new() -> Self {
            Self::make(None)
        }
        fn make(raw: Option<&[u8]>) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "rocc-tape-controller-{}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&dir).unwrap();
            let paths = TapeProtectedPaths {
                image: dir.join("image"),
                overlay: dir.join("overlay"),
                ram: dir.join("ram"),
                report: None,
            };
            let mount = if let Some(raw) = raw {
                fs::write(dir.join("tape"), raw).unwrap();
                TapeMount::ReadOnly(dir.join("tape"))
            } else {
                TapeMount::CreateNew(dir.join("tape"))
            };
            let session = TapeSession::open(&mount, &paths).unwrap();
            Self {
                dir,
                controller: TapeController::new(session),
                memory: crate::cpu::Cpu::new(16, true).unwrap().memory,
            }
        }
        fn io(&mut self, op: u8, p: u8, v: u16) -> IoResult {
            self.controller.io(&mut self.memory, op, p, v).unwrap()
        }
        fn tick(&mut self) {
            self.controller.tick(&mut self.memory).unwrap();
        }
        fn write(&mut self) {
            self.io(2, 0, 0x28);
            self.io(4, 0, 300);
            self.io(6, 1, 0x7ffe);
            self.tick();
        }
    }
    impl Drop for TestTape {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.dir).unwrap();
        }
    }
    #[test]
    fn native_register_sequence_writes_words_and_restores_them() {
        let mut t = TestTape::new();
        t.memory.write(300, 0x1234).unwrap();
        t.memory.write(301, 0xabcd).unwrap();
        t.io(2, 0, 0x28);
        t.io(4, 0, 300);
        t.io(6, 1, 0x7ffe);
        assert!(t.controller.busy);
        assert!(!t.controller.done);
        t.tick();
        assert!(!t.controller.busy);
        assert!(t.controller.done);
        assert_eq!(t.io(3, 0, 0).value, Some(302));
        assert_eq!(
            fs::read(t.dir.join("tape")).unwrap(),
            [4, 0, 0, 0, 0x12, 0x34, 0xab, 0xcd, 4, 0, 0, 0]
        );
        t.io(2, 1, 8);
        t.tick();
        t.io(2, 0, 0);
        t.io(4, 0, 500);
        t.io(6, 1, 0xfffe);
        t.tick();
        assert_eq!(t.memory.read(500).unwrap(), 0x1234);
        assert_eq!(t.memory.read(501).unwrap(), 0xabcd);
        assert_eq!(t.io(3, 2, 0).value, Some(502));
        assert!(!t.controller.done);
    }
    #[test]
    fn transfer_validation_is_atomic() {
        let mut t = TestTape::new();
        t.memory.write(300, 0x1234).unwrap();
        t.io(2, 0, 0x28);
        t.io(4, 0, 300);
        t.io(6, 1, 0x7ffe);
        assert!(t.controller.tick(&mut t.memory).is_err());
        assert_eq!(fs::metadata(t.dir.join("tape")).unwrap().len(), 0);
        let mut t = TestTape::new();
        t.memory.write(300, 0x1234).unwrap();
        t.memory.write(301, 0xabcd).unwrap();
        t.write();
        t.io(2, 1, 8);
        t.tick();
        t.io(2, 0, 0);
        t.io(4, 0, 0xefff);
        t.io(6, 1, 0xfffe);
        t.memory
            .set_mapping(crate::memory::Mapping::GlobalOnly)
            .unwrap();
        assert!(t.controller.tick(&mut t.memory).is_err());
        assert_eq!(t.memory.peek(0xefff).unwrap(), None);
        assert_eq!(t.controller.session.medium.position(), 0);
    }
    #[test]
    fn marks_spacing_reset_and_bounded_diagnostics() {
        let mut t = TestTape::new();
        t.io(2, 1, 0x30);
        t.tick();
        // The offline WMS mark writer treats EOF in its completion mask as an error.
        assert_eq!(t.io(1, 0, 0).value.unwrap() & 0x100, 0);
        t.io(2, 1, 0x20);
        t.tick();
        assert_eq!(t.controller.session.medium.position(), 0);
        t.io(2, 1, 0x18);
        t.tick();
        assert_eq!(t.controller.session.medium.position(), 1);
        t.io(2, 1, 8);
        t.controller.reset();
        t.tick();
        assert_eq!(t.controller.session.medium.position(), 1);
        assert!(!t.controller.busy);
        assert!(!t.controller.done);
        for _ in 0..300 {
            t.io(7, 1, 0);
        }
        assert_eq!(t.controller.status().recent.len(), 256);
    }
    #[test]
    fn fixture_status_and_unsupported_context() {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/tape-controller.json"))
                .unwrap();
        for case in fixtures.as_array().unwrap() {
            let mut t = TestTape::new();
            for e in case["events"].as_array().unwrap() {
                let r = t.io(
                    e["operation"].as_u64().unwrap() as u8,
                    e["pulse"].as_u64().unwrap() as u8,
                    e["value"].as_u64().unwrap() as u16,
                );
                assert_eq!(r.value, e["input"].as_u64().map(|v| v as u16));
                assert_eq!(r.skip, e["skip"].as_bool().unwrap());
                assert_eq!(t.controller.busy, e["busy"].as_bool().unwrap());
                assert_eq!(t.controller.done, e["done"].as_bool().unwrap());
            }
        }
        let mut t = TestTape::new();
        let f = t.controller.io(&mut t.memory, 2, 3, 0x4000).unwrap_err();
        assert_eq!(f.details()["device"], 16);
        assert_eq!(f.details()["operation"], 2);
        assert_eq!(f.details()["pulse"], 3);
        assert_eq!(f.details()["value"], 0x4000);
    }
    #[test]
    fn odd_short_overflow_and_write_protection() {
        let mut t = TestTape::make(Some(&[3, 0, 0, 0, 0x12, 0x34, 0x56, 0, 3, 0, 0, 0]));
        t.io(2, 0, 0);
        t.io(4, 0, 300);
        t.io(6, 1, 0x7ffd);
        t.tick();
        assert_eq!(t.memory.read(300).unwrap(), 0x1234);
        assert_eq!(t.memory.read(301).unwrap(), 0x5600);
        assert_eq!(t.memory.peek(302).unwrap(), None);
        assert_eq!(t.io(3, 0, 0).value, Some(302));
        assert_ne!(t.io(1, 0, 0).value.unwrap() & 2, 0);
        t.io(2, 1, 8);
        t.tick();
        t.io(2, 0, 0);
        t.io(4, 0, 400);
        t.io(6, 1, 0xffff);
        t.tick();
        assert_eq!(t.memory.read(400).unwrap(), 0x1234);
        assert_eq!(t.memory.peek(401).unwrap(), None);
        assert_eq!(t.io(5, 0, 0).value.unwrap() & 0x80, 0x80);
        t.io(2, 0, 0x28);
        t.io(4, 0, 300);
        t.io(6, 1, 0xffff);
        t.tick();
        assert_ne!(t.io(1, 0, 0).value.unwrap() & 4, 0);
        assert_eq!(fs::metadata(t.dir.join("tape")).unwrap().len(), 12);
    }
    #[test]
    fn zero_count_read_loads_one_bounded_record_for_original_loader() {
        let mut t = TestTape::make(Some(&[2, 0, 0, 0, 0x12, 0x34, 2, 0, 0, 0]));
        t.io(2, 0, 0);
        t.io(4, 0, 300);
        t.io(6, 1, 0);
        t.tick();
        assert_eq!(t.memory.read(300).unwrap(), 0x1234);
        assert_eq!(t.memory.peek(301).unwrap(), None);
        assert_eq!(t.io(3, 0, 0).value, Some(301));
    }
    #[test]
    fn zero_count_read_validates_actual_record_at_memory_boundaries() {
        for (address, mapping) in [
            (0xffff, crate::memory::Mapping::SelectedBank(0)),
            (0xefff, crate::memory::Mapping::GlobalOnly),
        ] {
            let mut t = TestTape::make(Some(&[2, 0, 0, 0, 0x12, 0x34, 2, 0, 0, 0]));
            t.memory.set_mapping(mapping).unwrap();
            t.io(2, 0, 0);
            t.io(4, 0, address);
            t.io(6, 1, 0);
            t.tick();
            assert_eq!(t.memory.read(address as usize).unwrap(), 0x1234);
            assert_eq!(t.controller.status().position, 1);
            assert_eq!(t.io(3, 0, 0).value, Some(address.wrapping_add(1)));
        }
    }
    #[test]
    fn zero_count_read_rejects_oversized_record_without_consuming_it() {
        let length = 8194u32;
        let mut raw = length.to_le_bytes().to_vec();
        raw.resize(4 + length as usize, 0x12);
        raw.extend_from_slice(&length.to_le_bytes());
        let mut t = TestTape::make(Some(&raw));
        t.memory.write(300, 0xabcd).unwrap();
        t.io(2, 0, 0);
        t.io(4, 0, 300);
        t.io(6, 1, 0);
        assert!(t.controller.tick(&mut t.memory).is_err());
        assert_eq!(t.controller.status().position, 0);
        assert_eq!(t.controller.status().reads, 0);
        assert!(!t.controller.done);
        assert_eq!(t.memory.read(300).unwrap(), 0xabcd);
    }
    #[test]
    fn zero_count_mark_and_end_do_not_require_dma_memory() {
        let mut t = TestTape::make(Some(&[0; 4]));
        t.memory
            .set_mapping(crate::memory::Mapping::GlobalOnly)
            .unwrap();
        for position in [1, 1] {
            t.io(2, 0, 0);
            t.io(4, 0, 0xffff);
            t.io(6, 1, 0);
            t.tick();
            assert_eq!(t.controller.status().position, position);
            assert_ne!(t.io(1, 0, 0).value.unwrap() & 0x200, 0);
            assert_eq!(t.io(3, 0, 0).value, Some(0xffff));
        }
    }
    #[test]
    fn rocc_file_mark_is_eof_not_the_hard_error_status() {
        let mut t = TestTape::make(Some(&[0; 4]));
        t.io(2, 0, 0);
        t.io(4, 0, 300);
        t.io(6, 1, 0x7000);
        t.tick();
        let status = t.io(1, 0, 0).value.unwrap();
        assert_eq!(status & 0x200, 0x200);
        assert_eq!(status & 0x100, 0);
        assert_eq!(t.io(3, 0, 0).value, Some(300));
        assert_eq!(t.memory.peek(300).unwrap(), None);
    }
    #[test]
    fn bus_mount_and_cpu_reset_preserve_media_and_absent_behavior() {
        let t = TestTape::new();
        let mut bus = crate::devices::Bus::new(vec![0; 256], true, false, None);
        let mut memory = crate::cpu::Cpu::new(16, true).unwrap().memory;
        assert_eq!(bus.io(&mut memory, 0o20, 1, 2, 0).unwrap().value, Some(0));
        assert_eq!(
            bus.io(&mut memory, 0o20, 1, 0, 0).unwrap_err().reason(),
            "unsupported_optional_device_operation"
        );
        let paths = TapeProtectedPaths {
            image: t.dir.join("image"),
            overlay: t.dir.join("overlay"),
            ram: t.dir.join("ram"),
            report: None,
        };
        bus.tape = Some(TapeController::new(
            TapeSession::open(&TapeMount::CreateNew(t.dir.join("other-tape")), &paths).unwrap(),
        ));
        assert_ne!(
            bus.io(&mut memory, 0o20, 1, 0, 0).unwrap().value.unwrap() & 1,
            0
        );
        bus.io(&mut memory, 0o20, 2, 1, 0x30).unwrap();
        bus.tape.as_mut().unwrap().tick(&mut memory).unwrap();
        assert_eq!(bus.pending(), None);
        bus.io(&mut memory, 0o77, 5, 2, 0).unwrap();
        assert!(!bus.tape.as_ref().unwrap().done);
        assert_eq!(fs::read(t.dir.join("other-tape")).unwrap(), [0; 4]);
    }
}
