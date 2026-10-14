use crate::{devices::Bus, memory::Memory, Fault};
use serde_json::json;
#[derive(Debug)]
pub struct Cpu {
    pub memory: Memory,
    pub ac: [u16; 4],
    pub carry: u16,
    pub pc: u16,
    pub global_only: bool,
    /// Last fetched instruction, including one that faults after interrupt entry.
    pub last_instruction: Option<(u16, u16)>,
}
#[derive(Debug)]
pub struct StepInfo {
    pub pc: u16,
    pub word: u16,
    pub block_move: bool,
    pub halt: bool,
}
impl Cpu {
    pub fn new(address_bits: u8, extended: bool) -> Self {
        Self {
            memory: Memory::new(address_bits, extended),
            ac: [0; 4],
            carry: 0,
            pc: 0,
            global_only: false,
            last_instruction: None,
        }
    }
    fn guard(&self, address: usize) -> Result<(), Fault> {
        if self.global_only && address >= self.memory.global_words {
            return Err(Fault::with(
                "unmapped_memory_bank",
                json!({"address":address,"selector":65535}),
            ));
        }
        Ok(())
    }
    pub fn read(&self, address: usize) -> Result<u16, Fault> {
        self.guard(address)?;
        self.memory.get(address).ok_or_else(|| {
            let mut details = json!({"address":address});
            if self.memory.bank_words != 0 {
                details["mapping"] = json!({"region":if address >= self.memory.global_words {"local"} else {"global"},
                    "bank":if address >= self.memory.global_words {Some(self.memory.bank)} else {None},
                    "physical_word":self.memory.physical(address)});
            }
            Fault::with("uninitialized_memory",details)
        })
    }
    pub fn write(&mut self, address: usize, value: u16) -> Result<(), Fault> {
        self.guard(address)?;
        self.memory.set(address, value);
        Ok(())
    }
    fn effective_address(&mut self, word: u16) -> Result<u16, Fault> {
        let mode = (word >> 8) & 3;
        let base = match mode {
            0 => 0,
            1 => self.pc,
            2 => self.ac[2],
            _ => self.ac[3],
        };
        let displacement = if mode == 0 {
            word & 255
        } else {
            (word as u8 as i8 as i16) as u16
        };
        let mut address = base.wrapping_add(displacement) & self.memory.mask;
        if word & 0x400 != 0 {
            for _ in 0..256 {
                let pointer = self.read(address as usize)?;
                let target = if (16..32).contains(&address) {
                    let target = if address < 24 {
                        pointer.wrapping_add(1)
                    } else {
                        pointer.wrapping_sub(1)
                    };
                    self.write(address as usize, target)?;
                    target
                } else {
                    pointer
                };
                address = target & self.memory.mask;
                if self.memory.mask == 65535 || pointer & 0x8000 == 0 {
                    return Ok(address);
                }
            }
            return Err(Fault::with(
                "indirect_limit",
                json!({"address":address,"limit":256}),
            ));
        }
        Ok(address)
    }
    fn block_move(&mut self, extended: bool) -> Result<(), Fault> {
        let destination = self.ac[0] as usize;
        let source = self.ac[1] as usize;
        let count = self.ac[3] as usize;
        if destination + count > self.memory.logical_words
            || source + count > self.memory.logical_words
        {
            return Err(Fault::with(
                "block_move_bounds",
                json!({"source":source,"destination":destination,"words":count}),
            ));
        }
        if extended && count > 0 {
            self.guard(destination + count - 1)?;
        }
        let mut bank = self.memory.bank;
        if extended
            && count > 0
            && self.memory.bank_words > 0
            && source + count > self.memory.global_words
        {
            bank = (self.read(2)? >> 8) as usize;
            if bank >= self.memory.banks {
                return Err(Fault::with(
                    "unsupported_move_source_bank",
                    json!({"bank":bank}),
                ));
            }
        }
        let mut pending = std::collections::HashMap::with_capacity(count);
        for n in 0..count {
            let address = source + n;
            let physical = address
                + if address >= self.memory.global_words {
                    bank * self.memory.bank_words
                } else {
                    0
                };
            let value = match pending.get(&physical) {
                Some(v) => *v,
                None if !extended => self.read(address)?,
                None => self.memory.storage[physical].ok_or_else(|| {
                    Fault::with(
                        "uninitialized_memory",
                        json!({"address":address,"bank":bank,"physical_word":physical}),
                    )
                })?,
            };
            pending.insert(self.memory.physical(destination + n), value);
        }
        for (physical, value) in pending {
            self.memory.storage[physical] = Some(value);
        }
        if extended {
            self.ac[0] = self.ac[0].wrapping_add(count as u16);
            self.ac[1] = self.ac[1].wrapping_add(count as u16);
            self.ac[3] = 0;
        }
        Ok(())
    }
    fn byte_field(&mut self, word: u16) -> Result<(), Fault> {
        let low = self.read((self.ac[2].wrapping_sub(68) & self.memory.mask) as usize)?;
        let high = self.read((self.ac[2].wrapping_sub(67) & self.memory.mask) as usize)?;
        if high & !1 != 0 {
            return Err(Fault::with(
                "unsupported_byte_descriptor",
                json!({"low":low,"high":high}),
            ));
        }
        let pointer = low as usize | ((high as usize) << 16);
        let address = pointer >> 1;
        let shift = if pointer & 1 == 0 { 8 } else { 0 };
        let value = self.read(address)?;
        if word == 0xfbc9 {
            self.write(
                address,
                (value & !(255 << shift)) | ((self.ac[1] & 255) << shift),
            )?;
        } else {
            self.ac[if word == 0xabc9 { 0 } else { 1 }] = (value >> shift) & 255;
        }
        Ok(())
    }
    pub fn step(&mut self, bus: &mut Bus) -> Result<StepInfo, Fault> {
        self.last_instruction = None;
        if bus.extended {
            bus.terminal.tick();
            if bus.delay > 0 {
                bus.delay -= 1;
            } else if bus.enabled && bus.pending().is_some() {
                let vector = self.read(1)?;
                self.memory.set(0, self.pc);
                self.pc = vector;
                bus.enabled = false;
                bus.interrupts += 1;
            }
        }
        let pc = self.pc;
        let word = self.read(pc as usize)?;
        self.last_instruction = Some((pc, word));
        let mut info = StepInfo {
            pc,
            word,
            block_move: false,
            halt: false,
        };
        let mut next_pc = pc.wrapping_add(1) & self.memory.mask;
        if word == 0x8208 {
            info.block_move = true;
            self.block_move(bus.extended)?;
        } else if bus.extended && matches!(word, 0xabc9 | 0xbbc9 | 0xfbc9) {
            self.byte_field(word)?;
        } else if word & 0x8000 != 0 {
            if bus.extended && word & 15 == 8 {
                return Err(Fault::new("unsupported_rocc_instruction"));
            }
            let source = ((word >> 13) & 3) as usize;
            let dest = ((word >> 11) & 3) as usize;
            let carry = match (word >> 4) & 3 {
                0 => self.carry,
                1 => 0,
                2 => 1,
                _ => 1 - self.carry,
            };
            let mut value = self.ac[source] as u32 | ((carry as u32) << 16);
            let destination = self.ac[dest] as u32;
            match (word >> 8) & 7 {
                0 => value ^= 65535,
                1 => value = (value ^ 65535) + 1,
                2 => {}
                3 => value += 1,
                4 => value = (value ^ 65535) + destination,
                5 => value = (value ^ 65535) + destination + 1,
                6 => value += destination,
                _ => value &= destination | 0x10000,
            }
            value &= 0x1ffff;
            value = match (word >> 6) & 3 {
                1 => ((value << 1) | (value >> 16)) & 0x1ffff,
                2 => (value >> 1) | ((value & 1) << 16),
                3 => (value & 0x10000) | ((value & 255) << 8) | ((value >> 8) & 255),
                _ => value,
            };
            let result = value as u16;
            let carry = (value >> 16) as u16;
            let skip = match word & 7 {
                0 => false,
                1 => true,
                2 => carry == 0,
                3 => carry == 1,
                4 => result == 0,
                5 => result != 0,
                6 => carry == 0 || result == 0,
                _ => carry == 1 && result != 0,
            };
            if skip {
                next_pc = next_pc.wrapping_add(1) & self.memory.mask;
            }
            if word & 8 == 0 {
                self.ac[dest] = result;
                self.carry = carry;
            }
        } else if word < 0x6000 {
            let address = self.effective_address(word)?;
            let op = word >> 11;
            match op {
                0 | 1 => {
                    if op == 1 {
                        self.ac[3] = next_pc;
                    }
                    next_pc = address;
                }
                2 | 3 => {
                    let old = self.read(address as usize)?;
                    let value = if op == 2 {
                        old.wrapping_add(1)
                    } else {
                        old.wrapping_sub(1)
                    };
                    self.write(address as usize, value)?;
                    if value == 0 {
                        next_pc = next_pc.wrapping_add(1) & self.memory.mask;
                    }
                }
                4..=7 => self.ac[(op & 3) as usize] = self.read(address as usize)?,
                _ => self.write(address as usize, self.ac[(op & 3) as usize])?,
            }
        } else if word & 0xe7ff == 0x663f {
            info.halt = true;
        } else {
            let accumulator = ((word >> 11) & 3) as usize;
            let operation = ((word >> 8) & 7) as u8;
            let number = (word & 63) as u8;
            let result = bus.io(
                self,
                number,
                operation,
                ((word >> 6) & 3) as u8,
                self.ac[accumulator],
            )?;
            if matches!(operation, 1 | 3 | 5) {
                self.ac[accumulator] = result
                    .value
                    .ok_or_else(|| Fault::with("missing_device_input", json!({"device":number})))?;
            }
            if result.skip {
                next_pc = next_pc.wrapping_add(1) & self.memory.mask;
            }
        }
        self.pc = next_pc;
        Ok(info)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn setup(extended: bool, word: u16) -> (Cpu, Bus) {
        let mut c = Cpu::new(16, extended);
        c.pc = 100;
        c.memory.set(100, word);
        (c, Bus::new(vec![0; 6144], extended, false, None))
    }
    #[test]
    fn alu_carry_rotate_and_no_load_skip() {
        let (mut c, mut b) = setup(false, 0x8340); // INC left
        c.ac[0] = 0xffff;
        c.step(&mut b).unwrap();
        assert_eq!((c.ac[0], c.carry), (1, 0));
        c.memory.set(101, 0x8209);
        c.step(&mut b).unwrap();
        assert_eq!((c.pc, c.ac[0], c.carry), (103, 1, 0));
    }
    #[test]
    fn indirect_autoincrement_uses_original_high_bit() {
        let mut c = Cpu::new(15, false);
        let mut b = Bus::new(vec![0; 6144], false, false, None);
        c.memory.set(0, 0x2410);
        c.memory.set(16, 0xffff);
        c.memory.set(0x7fff, 7);
        // Original pointer high bit continues after increment wraps to zero.
        c.memory.set(0x2410, 0x7777);
        c.step(&mut b).unwrap();
        assert_eq!(c.ac[0], 0x7777);
        assert_eq!(c.memory.get(16), Some(0));
    }
    #[test]
    fn forward_overlap_and_loader_register_preservation() {
        for extended in [false, true] {
            let (mut c, mut b) = setup(extended, 0x8208);
            c.ac = [201, 200, 9, 3];
            c.memory.set(200, 42);
            c.carry = 1;
            c.step(&mut b).unwrap();
            assert_eq!((c.memory.get(201), c.memory.get(203)), (Some(42), Some(42)));
            assert_eq!(
                c.ac,
                if extended {
                    [204, 203, 9, 0]
                } else {
                    [201, 200, 9, 3]
                }
            );
            assert_eq!(c.carry, 1);
        }
    }
    #[test]
    fn failed_move_is_atomic_and_source_bank_is_independent() {
        let (mut c, mut b) = setup(true, 0x8208);
        c.ac = [200, 300, 0, 2];
        c.memory.set(300, 7);
        assert_eq!(c.step(&mut b).unwrap_err().reason, "uninitialized_memory");
        assert_eq!(c.memory.get(200), None);
        assert_eq!(c.pc, 100);
        c.ac = [0xf000, 0xf000, 0, 1];
        c.memory.set(2, 0x100);
        c.memory.bank = 1;
        c.memory.set(0xf000, 77);
        c.memory.bank = 0;
        c.step(&mut b).unwrap();
        assert_eq!(c.memory.get(0xf000), Some(77));
    }
    #[test]
    fn byte_fields_and_global_guard_and_unknown_extension() {
        let (mut c, mut b) = setup(true, 0xabc9);
        c.ac[2] = 500;
        c.memory.set(432, 600);
        c.memory.set(433, 0);
        c.memory.set(300, 0xabcd);
        c.step(&mut b).unwrap();
        assert_eq!(c.ac[0], 0xab);
        c.memory.set(101, 0xfbc9);
        c.memory.set(432, 601);
        c.ac[1] = 0x12;
        c.step(&mut b).unwrap();
        assert_eq!(c.memory.get(300), Some(0xab12));
        c.global_only = true;
        assert_eq!(c.read(0xf000).unwrap_err().reason, "unmapped_memory_bank");
        c.memory.set(102, 0x8008);
        assert_eq!(
            c.step(&mut b).unwrap_err().reason,
            "unsupported_rocc_instruction"
        );
    }
    #[test]
    fn interrupt_delay_and_halt_advance_pc() {
        let (mut c, mut b) = setup(true, 0x8200);
        c.memory.set(1, 200);
        c.memory.set(200, 0x663f);
        b.terminal.events.push_back([1, 2, 3]);
        b.enabled = true;
        b.delay = 1;
        c.step(&mut b).unwrap();
        assert_eq!(c.pc, 101);
        assert_eq!(b.interrupts, 0);
        let info = c.step(&mut b).unwrap();
        assert!(info.halt);
        assert_eq!(c.pc, 201);
        assert_eq!(c.memory.get(0), Some(101));
        assert_eq!(b.interrupts, 1);
        assert!(!b.enabled);
    }
}
