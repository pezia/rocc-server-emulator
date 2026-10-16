//! File-backed record media and the ROCC tape controller.
mod controller;
mod medium;
mod session;

pub(crate) use controller::TapeController;
pub(crate) use session::{TapeProtectedPaths, TapeSession};
use std::collections::VecDeque;
use std::path::PathBuf;

#[derive(Debug, Clone, serde::Serialize)]
pub struct TapeEvent {
    pub operation: u8,
    pub pulse: u8,
    pub value: u16,
    pub input: Option<u16>,
    pub skip: bool,
    pub busy: bool,
    pub done: bool,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct TapeStatus {
    pub mode: String,
    pub unit: u8,
    pub initial_sha256: String,
    pub final_sha256: Option<String>,
    pub records: usize,
    pub marks: usize,
    pub position: usize,
    pub reads: u64,
    pub writes: u64,
    pub error: Option<String>,
    pub recent: VecDeque<TapeEvent>,
}
#[derive(Debug, Clone)]
pub enum TapeMount {
    ReadOnly(PathBuf),
    CreateNew(PathBuf),
}
