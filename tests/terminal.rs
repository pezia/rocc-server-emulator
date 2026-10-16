use rocc_emulator::{config::*, memory::Memory, terminal::Terminal};
fn setup(
    events: usize,
    bytes: usize,
    packets: usize,
    mode: CaptureMode,
    connect: Option<u64>,
) -> (Terminal, Memory) {
    let c = MachineConfig::new(
        CpuProfile::Extended,
        16,
        TerminalProfile::Experimental,
        connect,
        ResourceLimits::new(events, bytes, packets).unwrap(),
        mode,
    )
    .unwrap();
    (Terminal::with_config(&c), Memory::new(&c))
}
#[test]
fn event_limit_rejects_command_atomically() {
    let (mut t, m) = setup(1, 4, 2, CaptureMode::Stop, None);
    t.io(&m, 2, 0, 0xe000).unwrap();
    let before = t.snapshot();
    assert_eq!(
        t.io(&m, 2, 0, 0xf001).unwrap_err().reason(),
        "terminal_event_limit"
    );
    assert_eq!(t.snapshot(), before);
}
#[test]
fn clock_events_reserve_capacity_together() {
    let (mut t, m) = setup(1, 4, 2, CaptureMode::Stop, Some(1000));
    t.io(&m, 2, 0, 0xe001).unwrap();
    t.io(&m, 6, 0, 1).unwrap();
    t.io(&m, 0, 2, 0).unwrap();
    for _ in 0..999 {
        t.tick().unwrap();
    }
    assert_eq!(t.tick().unwrap_err().reason(), "terminal_event_limit");
    assert_eq!(t.pending_events(), 0);
    assert_eq!(t.snapshot()["connected"], false);
    assert_eq!(t.snapshot()["timer_ticks"], 0);
}
fn output(t: &mut Terminal, m: &Memory, v: u16) {
    t.io(m, 2, 0, 0).unwrap();
    t.io(m, 4, 0, 0).unwrap();
    t.io(m, 6, 0, v).unwrap();
    t.io(m, 0, 2, 0).unwrap();
}
#[test]
fn batch_packets_and_bytes_have_independent_limits() {
    let (mut t, m) = setup(2, 4, 2, CaptureMode::Stop, None);
    output(&mut t, &m, 65);
    output(&mut t, &m, 66);
    t.io(&m, 2, 0, 0).unwrap();
    t.io(&m, 4, 0, 0).unwrap();
    let before = t.display_report();
    assert_eq!(
        t.io(&m, 6, 0, 67).unwrap_err().reason(),
        "terminal_capture_limit"
    );
    assert_eq!(t.display_report(), before);
    assert_eq!(t.pending_events(), 0);
}
#[test]
fn live_history_evicts_and_keeps_full_screen() {
    let (mut t, m) = setup(2, 4, 2, CaptureMode::Tail, None);
    for b in b"ABCDEF" {
        output(&mut t, &m, *b as u16);
    }
    let d = t.display_report();
    assert_eq!(d["screen"]["lines"][0], "ABCDEF");
    assert_eq!(d["packets"].as_array().unwrap().len(), 2);
    assert_eq!(
        d["full_stream_sha256"],
        "e9c0f8b575cbfcb42ab3b78ecc87efa3b011d9a5d10b09fa4e96f240bf6a82f5"
    );
    assert_eq!(
        d["retained_sha256"],
        "90cff9317110dfcbfa1b6bfe5e62ca07e9defb7094035fe03a7752d3e18cc51c"
    );
    assert_eq!(d["byte_history_truncated"], true);
    assert_eq!(d["packet_history_truncated"], true);
    assert_eq!(t.snapshot()["display_byte_count"], 4);
}
