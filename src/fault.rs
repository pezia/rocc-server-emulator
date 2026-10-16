use serde_json::Value;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    BlockMoveBounds,
    DiskAddressBounds,
    DiskDmaBounds,
    DiskDmaResultUninitialized,
    DiskOverlayWrite,
    DiskRegisterUninitialized,
    IndirectLimit,
    MemoryAddressBounds,
    MissingDeviceInput,
    TerminalIsNotConnected,
    TerminalCaptureLimit,
    TerminalDmaOutsideSharedMemory,
    TerminalDmaUninitialized,
    TerminalEventLimit,
    TerminalInputWithoutEvent,
    UninitializedMemory,
    UnmappedMemoryBank,
    UnsupportedAuxiliaryOperation,
    UnsupportedBankOperation,
    UnsupportedByteDescriptor,
    UnsupportedCpuControl,
    UnsupportedCpuPulse,
    UnsupportedDiskCommand,
    UnsupportedDiskOperation,
    UnsupportedIo,
    UnsupportedMemoryBank,
    UnsupportedMoveSourceBank,
    UnsupportedOptional51Operation,
    UnsupportedOptionalDeviceOperation,
    UnsupportedRoccInstruction,
    UnsupportedSetupOperation,
    UnsupportedTerminalCommand,
    UnsupportedTerminalDmaMode,
    UnsupportedTerminalOperation,
    UnsupportedTerminalParameter,
}
impl FaultKind {
    pub fn reason(self) -> &'static str {
        match self {
            Self::BlockMoveBounds => "block_move_bounds",
            Self::DiskAddressBounds => "disk_address_bounds",
            Self::DiskDmaBounds => "disk_dma_bounds",
            Self::DiskDmaResultUninitialized => "disk_dma_result_uninitialized",
            Self::DiskOverlayWrite => "disk_overlay_write",
            Self::DiskRegisterUninitialized => "disk_register_uninitialized",
            Self::IndirectLimit => "indirect_limit",
            Self::MemoryAddressBounds => "memory_address_bounds",
            Self::MissingDeviceInput => "missing_device_input",
            Self::TerminalIsNotConnected => "terminal is not connected",
            Self::TerminalCaptureLimit => "terminal_capture_limit",
            Self::TerminalDmaOutsideSharedMemory => "terminal_dma_outside_shared_memory",
            Self::TerminalDmaUninitialized => "terminal_dma_uninitialized",
            Self::TerminalEventLimit => "terminal_event_limit",
            Self::TerminalInputWithoutEvent => "terminal_input_without_event",
            Self::UninitializedMemory => "uninitialized_memory",
            Self::UnmappedMemoryBank => "unmapped_memory_bank",
            Self::UnsupportedAuxiliaryOperation => "unsupported_auxiliary_operation",
            Self::UnsupportedBankOperation => "unsupported_bank_operation",
            Self::UnsupportedByteDescriptor => "unsupported_byte_descriptor",
            Self::UnsupportedCpuControl => "unsupported_cpu_control",
            Self::UnsupportedCpuPulse => "unsupported_cpu_pulse",
            Self::UnsupportedDiskCommand => "unsupported_disk_command",
            Self::UnsupportedDiskOperation => "unsupported_disk_operation",
            Self::UnsupportedIo => "unsupported_io",
            Self::UnsupportedMemoryBank => "unsupported_memory_bank",
            Self::UnsupportedMoveSourceBank => "unsupported_move_source_bank",
            Self::UnsupportedOptional51Operation => "unsupported_optional51_operation",
            Self::UnsupportedOptionalDeviceOperation => "unsupported_optional_device_operation",
            Self::UnsupportedRoccInstruction => "unsupported_rocc_instruction",
            Self::UnsupportedSetupOperation => "unsupported_setup_operation",
            Self::UnsupportedTerminalCommand => "unsupported_terminal_command",
            Self::UnsupportedTerminalDmaMode => "unsupported_terminal_dma_mode",
            Self::UnsupportedTerminalOperation => "unsupported_terminal_operation",
            Self::UnsupportedTerminalParameter => "unsupported_terminal_parameter",
        }
    }
}
#[derive(Debug, Clone)]
pub struct Fault {
    pub kind: FaultKind,
    context: FaultContext,
}
impl Fault {
    pub fn new(kind: FaultKind) -> Self {
        Self::with(kind, serde_json::json!({}))
    }
    pub fn with(kind: FaultKind, details: Value) -> Self {
        Self {
            kind,
            context: serde_json::from_value(details).expect("typed fault context"),
        }
    }
    pub fn details(&self) -> Value {
        serde_json::to_value(&self.context).expect("fault serialization")
    }
    pub fn reason(&self) -> &'static str {
        self.kind.reason()
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct FaultContext {
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "address")]
    address: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "selector")]
    selector: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "limit")]
    limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "source")]
    source: Option<usize>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "destination"
    )]
    destination: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "words")]
    words: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "bank")]
    bank: Option<usize>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "physical_word"
    )]
    physical_word: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "low")]
    low: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "high")]
    high: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "device")]
    device: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "offset")]
    offset: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "head")]
    head: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "RAM_word")]
    ram_word: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "A")]
    a: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "error")]
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "operation")]
    operation: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "pulse")]
    pulse: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "value")]
    value: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "command")]
    command: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "mapping")]
    mapping: Option<MemoryMapping>,
}
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct MemoryMapping {
    region: String,
    bank: Option<usize>,
    physical_word: usize,
}
