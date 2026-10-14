#[derive(Debug, Clone)]
pub struct Memory {
    pub storage: Vec<Option<u16>>,
    pub bank: usize,
    pub bank_words: usize,
    pub global_words: usize,
    pub logical_words: usize,
    pub mask: u16,
    pub banks: usize,
}
impl Memory {
    pub fn new(address_bits: u8, extended: bool) -> Self {
        assert!(matches!(address_bits, 15 | 16));
        assert!(!extended || address_bits == 16);
        let logical_words = 1usize << address_bits;
        let bank_words = if extended { 4096 } else { 0 };
        let banks = if extended { 128 } else { 1 };
        let global_words = logical_words - bank_words;
        Self {
            storage: vec![None; global_words + bank_words * banks],
            bank: 0,
            bank_words,
            global_words,
            logical_words,
            mask: (logical_words - 1) as u16,
            banks,
        }
    }
    pub fn physical(&self, address: usize) -> usize {
        assert!(
            address < self.logical_words,
            "logical address outside configured memory"
        );
        address
            + if address >= self.global_words {
                self.bank * self.bank_words
            } else {
                0
            }
    }
    pub fn get(&self, address: usize) -> Option<u16> {
        self.storage[self.physical(address)]
    }
    pub fn set(&mut self, address: usize, value: u16) {
        let p = self.physical(address);
        self.storage[p] = Some(value);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bank_switch_preserves_global_and_isolates_local() {
        let mut m = Memory::new(16, true);
        assert_eq!(m.get(0), None);
        m.set(0, 123);
        m.set(0xf000, 456);
        m.bank = 127;
        assert_eq!(m.get(0), Some(123));
        assert_eq!(m.get(0xf000), None);
        m.set(0xf000, 789);
        m.bank = 0;
        assert_eq!(m.get(0xf000), Some(456));
        assert_eq!(m.storage.len(), 61440 + 4096 * 128);
    }
    #[test]
    fn flat_memory_has_full_selected_address_space() {
        let mut m = Memory::new(15, false);
        assert_eq!(m.mask, 32767);
        assert_eq!(m.logical_words, 32768);
        m.set(32767, 65535);
        assert_eq!(m.get(32767), Some(65535));
    }
}
