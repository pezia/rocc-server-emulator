//! Borrowed report views serialize large collections incrementally.
use crate::{interactive::InteractiveOutcome, machine::Machine};
use serde::{
    ser::{SerializeMap, SerializeSeq},
    Serialize, Serializer,
};
use std::{
    io::{self, Write},
    path::PathBuf,
};
#[derive(Debug, Clone, Serialize)]
pub struct KeyInput {
    pub step: u64,
    pub byte: u8,
}
#[derive(Default, Serialize)]
pub struct RunMetadata {
    pub input: Option<PathBuf>,
    pub overlay_path: Option<PathBuf>,
    pub elapsed_seconds: f64,
    pub step_budget: Option<u64>,
    pub batch_keys: Vec<KeyInput>,
    pub interactive: Option<InteractiveOutcome>,
}
pub fn write_report<W: Write>(
    mut writer: W,
    machine: &Machine,
    metadata: &RunMetadata,
) -> io::Result<()> {
    serde_json::to_writer_pretty(&mut writer, &Report { machine, metadata })
        .map_err(io::Error::other)?;
    writer.write_all(b"\n")
}
pub(crate) struct Report<'a> {
    pub machine: &'a Machine,
    pub metadata: &'a RunMetadata,
}
impl Serialize for Report<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let m = self.machine;
        let mut map = s.serialize_map(None)?;
        macro_rules! field {
            ($key:expr,$value:expr) => {
                map.serialize_entry($key, &$value)?;
            };
        }
        field!("implementation", "rust");
        field!(
            "profile",
            if m.config().extended() {
                "rocc-startup-v2"
            } else {
                "rocc-startup-v1"
            }
        );
        field!(
            "warning",
            "Experimental inferred hardware; not complete vendor hardware."
        );
        field!("trace_format", "bounded recent-instruction trace");
        field!("report_schema_version", 2);
        field!("emulator_version", env!("CARGO_PKG_VERSION"));
        field!("build_revision", env!("ROCC_BUILD_REVISION"));
        field!("configuration", m.config());
        field!("cold_boot_completed", m.cold_boot_completed());
        field!("front_panel_switches", m.bus.switches);
        field!("image_sha256", m.image_sha256);
        field!("initial_overlay_sha256", m.initial_overlay_sha256());
        field!("final_overlay_sha256", overlay_digest(&m.bus.disk.overlay));
        field!("steps", m.steps);
        field!("reason", m.summary().reason);
        field!("state", m.state());
        field!("disk_reads", m.bus.disk.reads);
        field!("disk_writes", m.bus.disk.writes);
        field!("disk_controls", m.bus.disk.controls);
        field!("block_moves", m.block_moves);
        field!("interrupts", m.bus.interrupts);
        field!("bank_selectors", m.bus.bank_selectors);
        field!("terminal", m.bus.terminal.snapshot());
        field!("display", m.bus.terminal.display_export());
        field!("memory_sha256", memory_digest(m));
        field!("overlay", OverlayReport(m));
        field!("recent_instructions", Recent(m));
        field!("fault", m.fault.as_ref().map(|f| f.details()));
        field!("elapsed_seconds", self.metadata.elapsed_seconds);
        field!("input", self.metadata.input);
        field!("overlay_path", self.metadata.overlay_path);
        field!("step_budget", self.metadata.step_budget);
        field!("batch_keys", self.metadata.batch_keys);
        if let Some(i) = &self.metadata.interactive {
            field!("interactive", i);
        }
        map.end()
    }
}
fn memory_digest(m: &Machine) -> String {
    #[cfg(test)]
    MEMORY_HASHES.with(|c| c.set(c.get() + 1));
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for word in m.cpu.memory.iter_physical() {
        match word {
            Some(w) => {
                let [a, b] = w.to_be_bytes();
                h.update([1, a, b]);
            }
            None => h.update([0, 0, 0]),
        }
    }
    crate::machine::hex(&h.finalize())
}
pub(crate) fn overlay_digest(blocks: &std::collections::BTreeMap<usize, Vec<u8>>) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"ROCC-OVERLAY-STATE-1");
    for (offset, raw) in blocks {
        h.update((*offset as u64).to_be_bytes());
        h.update(raw);
    }
    crate::machine::hex(&h.finalize())
}
struct Recent<'a>(&'a Machine);
impl Serialize for Recent<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Entry {
            pc: u16,
            word: u16,
            next_pc: u16,
        }
        let mut seq = s.serialize_seq(Some(self.0.recent.len()))?;
        for &(pc, word, next_pc) in &self.0.recent {
            seq.serialize_element(&Entry { pc, word, next_pc })?;
        }
        seq.end()
    }
}
struct OverlayReport<'a>(&'a Machine);
impl Serialize for OverlayReport<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(3))?;
        map.serialize_entry("base_sha256", &self.0.image_sha256)?;
        map.serialize_entry("block_bytes", &256)?;
        map.serialize_entry("blocks", &Blocks(&self.0.bus.disk.overlay))?;
        map.end()
    }
}
struct Blocks<'a>(&'a std::collections::BTreeMap<usize, Vec<u8>>);
impl Serialize for Blocks<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.0.len()))?;
        for (offset, raw) in self.0 {
            let mut hex = [0u8; 512];
            const H: &[u8] = b"0123456789abcdef";
            for (i, b) in raw.iter().enumerate() {
                hex[2 * i] = H[(b >> 4) as usize];
                hex[2 * i + 1] = H[(b & 15) as usize];
            }
            #[derive(Serialize)]
            struct Block<'a> {
                offset: usize,
                hex: &'a str,
            }
            seq.serialize_element(&Block {
                offset: *offset,
                hex: std::str::from_utf8(&hex).map_err(serde::ser::Error::custom)?,
            })?;
        }
        seq.end()
    }
}
#[cfg(test)]
thread_local! {static MEMORY_HASHES:std::cell::Cell<usize>=const{std::cell::Cell::new(0)};}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summary_does_not_visit_report_data() {
        let m = Machine::new(vec![0; 6144], true, 16, false, None).unwrap();
        MEMORY_HASHES.with(|c| c.set(0));
        assert_eq!(m.summary().steps, 0);
        assert_eq!(MEMORY_HASHES.with(|c| c.get()), 0);
        let mut out = Vec::new();
        write_report(&mut out, &m, &RunMetadata::default()).unwrap();
        assert_eq!(MEMORY_HASHES.with(|c| c.get()), 1);
    }
}

#[derive(Debug)]
pub struct ReplayHistory {
    entries: std::collections::VecDeque<KeyInput>,
    complete: bool,
}
impl Default for ReplayHistory {
    fn default() -> Self {
        Self {
            entries: std::collections::VecDeque::new(),
            complete: true,
        }
    }
}
impl ReplayHistory {
    pub fn record(&mut self, step: u64, byte: u8) {
        if self.entries.len() == 65536 {
            self.entries.pop_front();
            self.complete = false;
        }
        self.entries.push_back(KeyInput { step, byte });
    }
    pub fn entries(&self) -> &std::collections::VecDeque<KeyInput> {
        &self.entries
    }
    pub fn is_complete(&self) -> bool {
        self.complete
    }
    pub(crate) fn into_parts(self) -> (std::collections::VecDeque<KeyInput>, bool) {
        (self.entries, self.complete)
    }
}
