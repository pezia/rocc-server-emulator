use crate::{config::MachineConfig, Fault, FaultKind};
use serde_json::json;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mapping {
    SelectedBank(u8),
    GlobalOnly,
}
#[derive(Debug, Clone)]
pub struct Memory {
    storage: Vec<Option<u16>>,
    bank: usize,
    mapping: Mapping,
    bank_words: usize,
    global_words: usize,
    logical_words: usize,
    mask: u16,
    banks: usize,
}
impl Memory {
    pub fn new(config: &MachineConfig) -> Self {
        let logical_words = 1usize << config.address_bits();
        let bank_words = if config.extended() { 4096 } else { 0 };
        let banks = if config.extended() { 128 } else { 1 };
        let global_words = logical_words - bank_words;
        Self {
            storage: vec![None; global_words + bank_words * banks],
            bank: 0,
            mapping: Mapping::SelectedBank(0),
            bank_words,
            global_words,
            logical_words,
            mask: (logical_words - 1) as u16,
            banks,
        }
    }
    pub fn mask(&self) -> u16 {
        self.mask
    }
    pub fn logical_words(&self) -> usize {
        self.logical_words
    }
    pub fn global_words(&self) -> usize {
        self.global_words
    }
    pub fn bank_words(&self) -> usize {
        self.bank_words
    }
    pub fn banks(&self) -> usize {
        self.banks
    }
    pub fn bank(&self) -> usize {
        self.bank
    }
    pub fn global_only(&self) -> bool {
        self.mapping == Mapping::GlobalOnly
    }
    pub fn mapping(&self) -> Mapping {
        self.mapping
    }
    pub fn set_mapping(&mut self, mapping: Mapping) -> Result<(), Fault> {
        if let Mapping::SelectedBank(b) = mapping {
            if b as usize >= self.banks {
                return Err(Fault::with(
                    FaultKind::UnsupportedMemoryBank,
                    json!({"selector":(b as u16)<<8}),
                ));
            }
            self.bank = b as usize;
        }
        self.mapping = mapping;
        Ok(())
    }
    pub fn physical(&self, address: usize, mapping: Mapping) -> Result<usize, Fault> {
        if address >= self.logical_words {
            return Err(Fault::with(
                FaultKind::MemoryAddressBounds,
                json!({"address":address}),
            ));
        }
        let bank = match mapping {
            Mapping::SelectedBank(b) if (b as usize) < self.banks => b as usize,
            Mapping::SelectedBank(b) => {
                return Err(Fault::with(
                    FaultKind::UnsupportedMemoryBank,
                    json!({"selector":(b as u16)<<8}),
                ))
            }
            Mapping::GlobalOnly if address >= self.global_words => {
                return Err(Fault::with(
                    FaultKind::UnmappedMemoryBank,
                    json!({"address":address,"selector":65535}),
                ))
            }
            Mapping::GlobalOnly => 0,
        };
        Ok(address
            + if address >= self.global_words {
                bank * self.bank_words
            } else {
                0
            })
    }
    pub fn validate_range(
        &self,
        start: usize,
        words: usize,
        mapping: Mapping,
    ) -> Result<(), Fault> {
        let end = start
            .checked_add(words)
            .filter(|e| *e <= self.logical_words)
            .ok_or_else(|| Fault::with(FaultKind::MemoryAddressBounds, json!({"address":start})))?;
        if words > 0 {
            self.physical(start, mapping)?;
            self.physical(end - 1, mapping)?;
        } else if let Mapping::SelectedBank(b) = mapping {
            if b as usize >= self.banks {
                return Err(Fault::new(FaultKind::UnsupportedMemoryBank));
            }
        }
        Ok(())
    }
    pub fn peek(&self, address: usize) -> Result<Option<u16>, Fault> {
        Ok(self.storage[self.physical(address, self.mapping)?])
    }
    pub fn read(&self, address: usize) -> Result<u16, Fault> {
        self.peek(address)?.ok_or_else(||{let mut details=json!({"address":address});if self.bank_words!=0{details["mapping"]=json!({"region":if address>=self.global_words{"local"}else{"global"},"bank":if address>=self.global_words{Some(self.bank)}else{None},"physical_word":self.physical(address,self.mapping).unwrap()});}Fault::with(FaultKind::UninitializedMemory,details)})
    }
    pub fn write(&mut self, address: usize, value: u16) -> Result<(), Fault> {
        let p = self.physical(address, self.mapping)?;
        self.storage[p] = Some(value);
        Ok(())
    }
    pub fn initialize_physical(&mut self, address: usize, value: u16) -> Result<(), Fault> {
        let cell = self.storage.get_mut(address).ok_or_else(|| {
            Fault::with(FaultKind::MemoryAddressBounds, json!({"address":address}))
        })?;
        *cell = Some(value);
        Ok(())
    }
    pub fn iter_physical(&self) -> impl Iterator<Item = Option<u16>> + '_ {
        self.storage.iter().copied()
    }
    pub(crate) fn physical_get(&self, p: usize) -> Option<u16> {
        self.storage.get(p).copied().flatten()
    }
    pub(crate) fn physical_set(&mut self, p: usize, v: u16) -> Result<(), Fault> {
        self.initialize_physical(p, v)
    }
    pub(crate) fn get(&self, address: usize) -> Option<u16> {
        self.peek(address).ok().flatten()
    }
    pub(crate) fn set(&mut self, address: usize, value: u16) {
        self.write(address, value)
            .expect("validated internal memory write");
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bank_switch_preserves_global_and_isolates_local() {
        let mut m = crate::cpu::Cpu::new(16, true).unwrap().memory;
        assert_eq!(m.get(0), None);
        m.set(0, 123);
        m.set(0xf000, 456);
        m.set_mapping(Mapping::SelectedBank(127)).unwrap();
        assert_eq!(m.get(0), Some(123));
        assert_eq!(m.get(0xf000), None);
        m.set(0xf000, 789);
        m.set_mapping(Mapping::SelectedBank(0)).unwrap();
        assert_eq!(m.get(0xf000), Some(456));
        assert_eq!(m.storage.len(), 61440 + 4096 * 128);
    }
    #[test]
    fn flat_memory_has_full_selected_address_space() {
        let mut m = crate::cpu::Cpu::new(15, false).unwrap().memory;
        assert_eq!(m.mask, 32767);
        assert_eq!(m.logical_words, 32768);
        m.set(32767, 65535);
        assert_eq!(m.get(32767), Some(65535));
    }
}
