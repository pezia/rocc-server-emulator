pub mod cpu;
pub mod devices;
pub mod interactive;
pub mod machine;
pub mod memory;
pub mod screen;
pub mod terminal;

#[derive(Debug, Clone)]
pub struct Fault {
    pub reason: String,
    pub details: serde_json::Value,
}
impl Fault {
    pub fn new(reason: &str) -> Self {
        Self::with(reason, serde_json::json!({}))
    }
    pub fn with(reason: &str, details: serde_json::Value) -> Self {
        Self {
            reason: reason.into(),
            details,
        }
    }
}
#[derive(Default, Debug)]
pub struct IoResult {
    pub value: Option<u16>,
    pub skip: bool,
}
