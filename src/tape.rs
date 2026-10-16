//! File-backed record media and the ROCC tape controller.
mod medium;
mod session;

use std::path::PathBuf;
#[derive(Debug, Clone)]
pub enum TapeMount {
    ReadOnly(PathBuf),
    CreateNew(PathBuf),
}
