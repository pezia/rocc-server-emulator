pub mod cpu;
pub mod devices;
pub mod interactive;
pub mod machine;
pub mod memory;
mod overlay;
#[allow(dead_code)] // Session wiring is added by the next implementation tasks.
mod persistence;
pub use persistence::MemoryStatus;
pub mod screen;
pub mod terminal;

pub mod config;
mod fault;
pub use fault::{Fault, FaultContext, FaultKind, MemoryMapping, MemoryRegion};

#[derive(Default, Debug)]
pub struct IoResult {
    pub value: Option<u16>,
    pub skip: bool,
}

mod signals;

pub mod artifact;

pub mod report;

pub use overlay::{compact_overlay, CompactionSummary};
