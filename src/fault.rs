use serde_json::Value;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    UnsupportedTapeOperation,
    TapeMedia,
    TapeRegisterUninitialized,
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
            Self::UnsupportedTapeOperation => "unsupported_tape_operation",
            Self::TapeMedia => "tape_media_error",
            Self::TapeRegisterUninitialized => "tape_register_uninitialized",
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
    context: Box<FaultContext>,
}
impl Fault {
    pub fn new(kind: FaultKind) -> Self {
        Self::with(kind, FaultContext::Empty {})
    }
    pub fn with(kind: FaultKind, context: FaultContext) -> Self {
        Self {
            kind,
            context: Box::new(context),
        }
    }
    pub fn reason(&self) -> &'static str {
        self.kind.reason()
    }
    pub fn details(&self) -> Value {
        serde_json::to_value(&self.context).expect("typed context serialization")
    }
}
#[derive(Debug, Clone, serde::Serialize)]
#[serde(untagged)]
pub enum FaultContext {
    Empty {},
    Address {
        address: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        selector: Option<u16>,
    },
    Memory {
        address: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        mapping: Option<MemoryMapping>,
    },
    CopySource {
        address: usize,
        bank: usize,
        physical_word: usize,
    },
    Indirect {
        address: usize,
        limit: usize,
    },
    BlockMove {
        source: usize,
        destination: usize,
        words: usize,
    },
    Bank {
        bank: usize,
    },
    Selector {
        selector: u16,
    },
    Descriptor {
        low: u16,
        high: u16,
    },
    Device {
        device: u8,
    },
    DiskCommand {
        #[serde(rename = "A")]
        command: u16,
    },
    DiskAddress {
        offset: usize,
        head: usize,
    },
    Dma {
        #[serde(rename = "RAM_word")]
        address: usize,
    },
    Persistence {
        offset: usize,
        error: String,
    },
    Protocol {
        operation: u8,
        pulse: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<u16>,
        #[serde(skip_serializing_if = "Option::is_none")]
        device: Option<u8>,
    },
    Parameter {
        value: u16,
    },
    Command {
        command: Option<u16>,
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<u16>,
    },
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct MemoryMapping {
    pub region: MemoryRegion,
    pub bank: Option<usize>,
    pub physical_word: usize,
}
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryRegion {
    Global,
    Local,
}
