pub mod cpu;
pub mod devices;
pub mod interactive;
pub mod machine;
pub mod memory;
mod overlay;
pub mod screen;
pub mod terminal;

pub mod config;
mod fault;
pub use fault::{Fault, FaultKind};

#[derive(Default, Debug)]
pub struct IoResult {
    pub value: Option<u16>,
    pub skip: bool,
}
