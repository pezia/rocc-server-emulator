use rocc_emulator::{config::*, memory::Memory, terminal::Terminal};
#[test]
fn unset_command_context_retains_explicit_null() {
    let config = MachineConfig::new(
        CpuProfile::Extended,
        16,
        TerminalProfile::Experimental,
        None,
        ResourceLimits::default(),
        CaptureMode::Stop,
    )
    .unwrap();
    let mut terminal = Terminal::with_config(&config);
    let fault = terminal.io(&Memory::new(&config), 6, 0, 1).unwrap_err();
    assert_eq!(fault.reason(), "unsupported_terminal_parameter");
    assert_eq!(
        fault.details().get("command"),
        Some(&serde_json::Value::Null)
    );
}
