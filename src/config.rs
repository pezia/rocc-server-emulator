use serde::Serialize;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CpuProfile {
    Loader,
    Extended,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalProfile {
    None,
    Experimental,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    Stop,
    Tail,
}
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ResourceLimits {
    events: usize,
    bytes: usize,
    packets: usize,
}
pub type ConfigError = String;
impl ResourceLimits {
    pub fn new(events: usize, bytes: usize, packets: usize) -> Result<Self, ConfigError> {
        if events == 0
            || events > 65536
            || bytes == 0
            || bytes > 64 * 1024 * 1024
            || packets == 0
            || packets > 65536
        {
            return Err(
                "invalid resource limits (events/packets: 1..65536; bytes: 1..67108864)".into(),
            );
        }
        Ok(Self {
            events,
            bytes,
            packets,
        })
    }
    pub fn events(self) -> usize {
        self.events
    }
    pub fn bytes(self) -> usize {
        self.bytes
    }
    pub fn packets(self) -> usize {
        self.packets
    }
}
impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            events: 4096,
            bytes: 1048576,
            packets: 4096,
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct MachineConfig {
    profile: CpuProfile,
    address_bits: u8,
    terminal: TerminalProfile,
    connect_after: Option<u64>,
    limits: ResourceLimits,
    capture: CaptureMode,
}
impl MachineConfig {
    pub fn new(
        profile: CpuProfile,
        address_bits: u8,
        terminal: TerminalProfile,
        connect_after: Option<u64>,
        limits: ResourceLimits,
        capture: CaptureMode,
    ) -> Result<Self, ConfigError> {
        if !matches!(address_bits, 15 | 16)
            || (profile == CpuProfile::Extended && address_bits != 16)
        {
            return Err("invalid address width for profile".into());
        }
        if (terminal == TerminalProfile::Experimental && profile != CpuProfile::Extended)
            || (connect_after.is_some() && terminal != TerminalProfile::Experimental)
            || connect_after == Some(0)
        {
            return Err("invalid terminal connection/profile".into());
        }
        Ok(Self {
            profile,
            address_bits,
            terminal,
            connect_after,
            limits,
            capture,
        })
    }
    pub fn profile(&self) -> CpuProfile {
        self.profile
    }
    pub fn extended(&self) -> bool {
        self.profile == CpuProfile::Extended
    }
    pub fn address_bits(&self) -> u8 {
        self.address_bits
    }
    pub fn experimental(&self) -> bool {
        self.terminal == TerminalProfile::Experimental
    }
    pub fn connect_after(&self) -> Option<u64> {
        self.connect_after
    }
    pub fn limits(&self) -> ResourceLimits {
        self.limits
    }
    pub fn capture(&self) -> CaptureMode {
        self.capture
    }
}
