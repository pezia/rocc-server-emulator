#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configured_keys_consume_fragmented_sequences_and_preserve_modifiers() {
        let mut k = Keyboard::new();
        k.bindings = parse_keymap(r#"{"Up":"ab","F1":"97","Delete":"9b","Enter":"af"}"#).unwrap();
        for (i, b) in b"\x1b[A\x1bOA\x1bOP\x1b[3~\r\x1b[1;5AZ".iter().enumerate() {
            k.feed(*b, i as u64 * 100);
        }
        assert_eq!(
            k.pending,
            VecDeque::from([0xab, 0xab, 0x97, 0x9b, 0xaf, b'Z'])
        );
        assert_eq!(k.notices.len(), 1);
    }
    #[test]
    fn recovered_help_and_release_defaults() {
        for sequence in [b"\x1bOP".as_slice(), b"\x1b[P", b"\x1b[11~", b"\x1b[[A"] {
            let mut k = Keyboard::new();
            for (time, &byte) in sequence.iter().enumerate() {
                k.feed(byte, time as u64 * 100);
            }
            assert_eq!(k.pending.pop_front(), Some(0x93));
            k.feed(27, 500);
            k.expire(530);
            assert_eq!(k.pending.pop_front(), Some(0x95));
            assert!(k.pending.is_empty());
        }
    }
    #[test]
    fn f2_control_default_remains_overridable() {
        for sequence in [b"\x1bOQ".as_slice(), b"\x1b[Q", b"\x1b[12~", b"\x1b[[B"] {
            let mut k = Keyboard::new();
            for &byte in sequence {
                k.feed(byte, 0);
            }
            assert_eq!(k.pending, VecDeque::from([0x97]));
            k.pending.clear();
            k.bindings = parse_keymap(r#"{"F2":"93"}"#).unwrap();
            for &byte in sequence {
                k.feed(byte, 1);
            }
            assert_eq!(k.pending, VecDeque::from([0x93]));
        }
    }
    #[test]
    fn keymap_rejects_typoes_and_non_bytes() {
        for raw in [
            r#"{"Cler":"93"}"#,
            r#"{"Up":171}"#,
            r#"{"Up":"abc"}"#,
            r#"{"Up":"+a"}"#,
            "[]",
        ] {
            assert!(parse_keymap(raw).is_err(), "{raw}");
        }
    }
    #[test]
    fn fragmented_function_keys_never_leak_suffixes() {
        let mut k = Keyboard::new();
        for (time, bytes) in [
            (0, &b"\x1b["[..]),
            (100, &b"1;5"[..]),
            (200, &b"AZ\x1bOR"[..]),
        ] {
            for &b in bytes {
                assert!(!k.feed(b, time));
            }
        }
        assert_eq!(k.pending, VecDeque::from(*b"Z"));
    }
    #[test]
    fn bare_escape_times_out_and_raw_entry_is_atomic() {
        let mut k = Keyboard::new();
        k.feed(27, 0);
        k.expire(29);
        assert!(k.pending.is_empty());
        k.expire(30);
        assert_eq!(k.pending.pop_front(), Some(0x95));
        for b in b"\x16f" {
            k.feed(*b, 31);
        }
        assert!(k.pending.is_empty());
        k.feed(b'f', 32);
        assert_eq!(k.pending.pop_front(), Some(255));
        for b in b"\x16xA" {
            k.feed(*b, 33);
        }
        assert_eq!(k.pending.pop_front(), Some(b'A'));
    }
    #[test]
    fn exit_interrupts_every_parser_state_and_queue_is_bounded() {
        let mut k = Keyboard::new();
        k.feed(22, 0);
        assert!(k.feed(3, 0));
        let mut k = Keyboard::new();
        for _ in 0..300 {
            k.feed(b'A', 0);
        }
        assert_eq!(k.pending.len(), 256);
        k.feed(27, 0);
        k.feed(b'[', 0);
        assert!(k.feed(29, 0));
    }
    #[test]
    fn function_key_cancels_raw_prompt_without_leaking_suffix() {
        let mut k = Keyboard::new();
        for &b in b"\x16\x1b[1;5AZ" {
            k.feed(b, 0);
        }
        assert_eq!(k.pending, VecDeque::from(*b"Z"));
    }
    #[test]
    fn fragmented_linux_console_function_keys_never_leak_menu_letters() {
        for raw_prompt in [false, true] {
            for suffix in b'C'..=b'E' {
                let mut k = Keyboard::new();
                if raw_prompt {
                    k.feed(22, 0);
                    k.feed(b'f', 0);
                }
                for (time, byte) in [27, b'[', b'[', suffix].into_iter().enumerate() {
                    assert!(!k.feed(byte, time as u64 * 100));
                    k.expire(time as u64 * 100);
                    assert!(
                        k.pending.is_empty(),
                        "leaked suffix {suffix} (raw prompt: {raw_prompt})"
                    );
                }
                k.feed(b'Z', 500);
                assert_eq!(k.pending, VecDeque::from(*b"Z"));
            }
        }
    }
    #[test]
    fn interrupted_escape_prefix_does_not_leak_function_key_suffix() {
        for bytes in [&b"\x1b\x1b[A"[..], &b"\x1b[1;\x1bOR"[..]] {
            let mut k = Keyboard::new();
            for &byte in bytes {
                k.feed(byte, 0);
            }
            assert!(k.pending.is_empty());
            k.feed(b'Z', 1);
            assert_eq!(k.pending, VecDeque::from(*b"Z"));
        }
    }
    #[test]
    fn enter_backspace_and_non_ascii_mapping() {
        let mut k = Keyboard::new();
        for b in [13, 10, 127, 200] {
            k.feed(b, 0);
        }
        assert_eq!(k.pending, VecDeque::from([13, 13, 8]));
        assert_eq!(k.notices.len(), 1);
    }
}

use crate::machine::Machine;

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::time::{Duration, Instant};

/// Host names are deliberately separate from unverified physical key labels.
pub fn parse_keymap(raw: &str) -> Result<BTreeMap<String, u8>, String> {
    let entries: BTreeMap<String, String> =
        serde_json::from_str(raw).map_err(|e| format!("invalid keymap: {e}"))?;
    let mut result = BTreeMap::new();
    for (name, byte) in entries {
        if ![
            "Enter",
            "Backspace",
            "Tab",
            "Escape",
            "Up",
            "Down",
            "Left",
            "Right",
            "Home",
            "End",
            "Insert",
            "Delete",
            "PageUp",
            "PageDown",
            "BackTab",
            "F1",
            "F2",
            "F3",
            "F4",
            "F5",
            "F6",
            "F7",
            "F8",
            "F9",
            "F10",
            "F11",
            "F12",
        ]
        .contains(&name.as_str())
        {
            return Err(format!("unknown host key: {name}"));
        }
        if byte.len() != 2 {
            return Err(format!("{name}: expected two hexadecimal digits"));
        }
        if !byte.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("{name}: invalid byte"));
        }
        let code = u8::from_str_radix(&byte, 16).map_err(|_| format!("{name}: invalid byte"))?;
        result.insert(name, code);
    }
    Ok(result)
}
fn host_sequence(bytes: &[u8]) -> Option<&'static str> {
    Some(match bytes {
        b"[A" | b"OA" => "Up",
        b"[B" | b"OB" => "Down",
        b"[C" | b"OC" => "Right",
        b"[D" | b"OD" => "Left",
        b"[H" | b"OH" | b"[1~" | b"[7~" => "Home",
        b"[F" | b"OF" | b"[4~" | b"[8~" => "End",
        b"[2~" => "Insert",
        b"[3~" => "Delete",
        b"[5~" => "PageUp",
        b"[6~" => "PageDown",
        b"[Z" => "BackTab",
        b"OP" | b"[P" | b"[11~" | b"[[A" => "F1",
        b"OQ" | b"[Q" | b"[12~" | b"[[B" => "F2",
        b"OR" | b"[R" | b"[13~" | b"[[C" => "F3",
        b"OS" | b"[S" | b"[14~" | b"[[D" => "F4",
        b"[15~" | b"[[E" => "F5",
        b"[17~" => "F6",
        b"[18~" => "F7",
        b"[19~" => "F8",
        b"[20~" => "F9",
        b"[21~" => "F10",
        b"[23~" => "F11",
        b"[24~" => "F12",
        _ => return None,
    })
}

#[derive(Clone, Copy)]
enum Escape {
    None,
    Waiting(u64),
    Sequence { first_csi_byte: bool },
}
struct Keyboard {
    bindings: BTreeMap<String, u8>,
    sequence: Vec<u8>,
    pending: VecDeque<u8>,
    notices: VecDeque<String>,
    notice: String,
    notice_time: u64,
    escape: Escape,
    literal: Option<Option<u8>>,
    delivered: u64,
    next_step: u64,
}
impl Keyboard {
    fn new() -> Self {
        Self {
            bindings: BTreeMap::new(),
            sequence: Vec::new(),
            pending: VecDeque::new(),
            notices: VecDeque::new(),
            notice: String::new(),
            notice_time: 0,
            escape: Escape::None,
            literal: None,
            delivered: 0,
            next_step: 0,
        }
    }
    fn notify(&mut self, text: &str, now: u64) {
        self.notice = text.into();
        self.notice_time = now;
        if self.notices.len() == 32 {
            self.notices.pop_front();
        }
        self.notices.push_back(text.into());
    }
    fn enqueue(&mut self, b: u8, now: u64) {
        if self.pending.len() == 256 {
            self.notify("Keyboard queue full; remaining paste ignored.", now);
        } else {
            self.pending.push_back(b);
        }
    }
    fn mapped(&mut self, name: &str, fallback: Option<u8>, now: u64) {
        if let Some(byte) = self.bindings.get(name).copied().or(fallback) {
            self.enqueue(byte, now);
        } else {
            self.notify(
                &format!("{name}: no recovered binding; configure --keymap or use Ctrl-V + hex."),
                now,
            );
        }
    }
    fn expire(&mut self, now: u64) {
        if matches!(self.escape,Escape::Waiting(start) if now.saturating_sub(start)>=30) {
            self.escape = Escape::None;
            self.mapped("Escape", Some(0x95), now);
        }
    }
    fn feed(&mut self, b: u8, now: u64) -> bool {
        if b == 3 || b == 29 {
            return true;
        }
        match self.escape {
            Escape::Waiting(_) => {
                if b == 27 {
                    self.escape = Escape::Waiting(now);
                } else if b == b'[' || b == b'O' {
                    self.sequence.clear();
                    self.sequence.push(b);
                    self.escape = Escape::Sequence {
                        first_csi_byte: b == b'[',
                    };
                } else {
                    self.escape = Escape::None;
                    self.notify(
                        "Key has no recovered ROCC mapping; use ASCII/control keys.",
                        now,
                    );
                }
                return false;
            }
            Escape::Sequence { first_csi_byte } => {
                if b == 27 {
                    self.escape = Escape::Waiting(now);
                    return false;
                }
                if self.sequence.len() < 32 {
                    self.sequence.push(b);
                }
                // Linux console F1-F5 use ESC [[ A through ESC [[ E.
                // The second '[' is a prefix, not the final function-key byte.
                self.escape = Escape::Sequence {
                    first_csi_byte: false,
                };
                if first_csi_byte && b == b'[' {
                    return false;
                }
                if (0x40..=0x7e).contains(&b) {
                    self.escape = Escape::None;
                    if let Some(name) = host_sequence(&self.sequence) {
                        // Physical key identities are unresolved. Guest software
                        // determines each raw byte's action; bindings take priority.
                        let fallback = match name {
                            "F1" => Some(0x93),
                            // Additional guest control; physical keycap unresolved.
                            "F2" => Some(0x97),
                            _ => None,
                        };
                        self.mapped(name, fallback, now);
                    } else {
                        self.notify("Unsupported host key sequence (including modifiers).", now);
                    }
                }
                return false;
            }
            Escape::None => {}
        }
        if let Some(first) = self.literal {
            let digit = match b {
                b'0'..=b'9' => Some(b - b'0'),
                b'a'..=b'f' => Some(b - b'a' + 10),
                b'A'..=b'F' => Some(b - b'A' + 10),
                _ => None,
            };
            if let Some(digit) = digit {
                if let Some(first) = first {
                    self.literal = None;
                    let value = first * 16 + digit;
                    self.notify(&format!("Queued raw ROCC byte {value:02X}."), now);
                    self.enqueue(value, now);
                } else {
                    self.literal = Some(Some(digit));
                }
            } else {
                self.literal = None;
                if b == 27 {
                    self.escape = Escape::Waiting(now);
                }
                self.notify(
                    "Raw byte cancelled: enter exactly two hexadecimal digits.",
                    now,
                );
            }
            return false;
        }
        match b {
            22 => {
                self.literal = Some(None);
                self.notify("Raw ROCC byte: type two hexadecimal digits.", now);
            }
            27 => self.escape = Escape::Waiting(now),
            10 | 13 => self.mapped("Enter", Some(13), now),
            8 | 127 => self.mapped("Backspace", Some(8), now),
            9 => self.mapped("Tab", Some(9), now),
            0..=126 => self.enqueue(b, now),
            _ => self.notify(
                "Key has no recovered ROCC mapping; use ASCII/control keys.",
                now,
            ),
        }
        false
    }
}

struct RawSession {
    original: libc::termios,
    output_flags: i32,
}
impl RawSession {
    fn enter() -> Result<Self, String> {
        unsafe {
            if libc::isatty(0) != 1 || libc::isatty(1) != 1 {
                return Err("interactive mode requires a terminal on stdin and stdout".into());
            }
            let mut original = std::mem::zeroed();
            if libc::tcgetattr(0, &mut original) != 0 {
                return Err(io::Error::last_os_error().to_string());
            }
            let mut raw = original;
            libc::cfmakeraw(&mut raw);
            raw.c_cc[libc::VMIN] = 0;
            raw.c_cc[libc::VTIME] = 0;
            if libc::tcsetattr(0, libc::TCSANOW, &raw) != 0 {
                return Err(io::Error::last_os_error().to_string());
            }
            let output_flags = libc::fcntl(1, libc::F_GETFL);
            let session = Self {
                original,
                output_flags,
            };
            if output_flags < 0
                || libc::fcntl(1, libc::F_SETFL, output_flags | libc::O_NONBLOCK) < 0
            {
                return Err(io::Error::last_os_error().to_string());
            }
            // Save the host cursor before entering the separate screen buffer.
            write_output(b"\x1b7\x1b[?1049h\x1b[?25l\x1b[2J")?;
            Ok(session)
        }
    }
}
impl Drop for RawSession {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(0, libc::TCSANOW, &self.original);
        }
        let _ = write_output_inner(b"\x1b[0m\x1b[?1049l\x1b8\x1b[?25h", true);
        if self.output_flags >= 0 {
            unsafe {
                libc::fcntl(1, libc::F_SETFL, self.output_flags);
            }
        }
    }
}
fn write_output(bytes: &[u8]) -> Result<(), String> {
    write_output_inner(bytes, false)
}
fn write_output_inner(mut bytes: &[u8], cleanup: bool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_millis(if cleanup { 250 } else { 1000 });
    while !bytes.is_empty() {
        if !cleanup && crate::signals::pending().is_some() {
            return Ok(());
        }
        let n = unsafe { libc::write(1, bytes.as_ptr().cast(), bytes.len()) };
        if n > 0 {
            bytes = &bytes[n as usize..];
            continue;
        }
        let e = io::Error::last_os_error();
        if n == 0 {
            return Err("terminal output closed".into());
        }
        if !matches!(
            e.kind(),
            io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
        ) {
            return Err(e.to_string());
        }
        if Instant::now() >= deadline {
            return Err("terminal output timed out".into());
        }
        let mut descriptor = libc::pollfd {
            fd: 1,
            events: libc::POLLOUT,
            revents: 0,
        };
        unsafe {
            libc::poll(&mut descriptor, 1, 25);
        }
    }
    Ok(())
}
/// Best-effort final host output, preserving flags and avoiding a blocked PTY.
pub fn write_final_output(bytes: &[u8]) -> Result<(), String> {
    let flags = unsafe { libc::fcntl(1, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(1, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    let result = write_output_inner(bytes, true);
    unsafe {
        libc::fcntl(1, libc::F_SETFL, flags);
    }
    result
}
fn size() -> Result<(usize, usize), String> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) } < 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    Ok((ws.ws_row as usize, ws.ws_col as usize))
}
fn put(out: &mut String, row: usize, col: usize, text: &str, height: usize, width: usize) {
    if row >= height || col >= width {
        return;
    }
    out.push_str(&format!("\x1b[{};{}H", row + 1, col + 1));
    // Only local ASCII notices and sanitized emulator cells reach the host.
    let available = width - col;
    out.extend(text.bytes().take(available).map(|b| {
        if (32..=126).contains(&b) {
            b as char
        } else {
            ' '
        }
    }));
    out.extend(std::iter::repeat_n(
        ' ',
        available.saturating_sub(text.len()),
    ));
}
fn render_frame(
    machine: &Machine,
    k: &Keyboard,
    lights: &DeviceLights,
    now: u64,
    height: usize,
    width: usize,
) -> String {
    let terminal = &machine.bus.terminal;
    let screen = &terminal.screen;
    let usable = height >= 25 && width >= 80;
    let mut out = String::from("\x1b[?25l\x1b[0m\x1b[?7l");
    if !usable {
        put(
            &mut out,
            0,
            0,
            "Resize to at least 80 columns x 25 rows. Ctrl-] exits.",
            height,
            width,
        );
    } else if terminal.total_display_bytes == 0 {
        put(
            &mut out,
            10,
            16,
            "Booting guest from disk...",
            height,
            width,
        );
        let target = terminal.connect_after.unwrap_or(26000000).max(1);
        put(
            &mut out,
            12,
            16,
            &format!(
                "{}% to terminal connection   Ctrl-] exits",
                machine
                    .steps
                    .saturating_mul(100)
                    .checked_div(target)
                    .unwrap_or(100)
                    .min(100)
            ),
            height,
            width,
        );
    } else {
        for row in 0..24 {
            out.push_str(&format!("\x1b[{};1H", row + 1));
            let mut reverse = false;
            for col in 0..80 {
                let next = screen.attributes[row][col] & 8 != 0;
                if next != reverse {
                    out.push_str(if next { "\x1b[7m" } else { "\x1b[0m" });
                    reverse = next;
                }
                let b = screen.cells[row][col];
                out.push(if (32..=126).contains(&b) {
                    b as char
                } else {
                    ' '
                });
            }
            out.push_str("\x1b[0m");
        }
    }
    if usable {
        if !k.notice.is_empty() && now.saturating_sub(k.notice_time) < 3000 {
            put(&mut out, 24, 0, &k.notice, height, width.saturating_sub(1));
        } else {
            lights.paint(&mut out, machine, now, height, width);
        }
    }
    if usable && terminal.total_display_bytes > 0 {
        out.push_str(&format!(
            "\x1b[{};{}H\x1b[?25h",
            screen.row.min(23) + 1,
            screen.column.min(79) + 1
        ));
    }
    // Paint the last physical cell without a wrap or scroll. The surrounding
    // RawSession saves/restores the original host cursor, attributes and
    // autowrap on VT-compatible hosts via DECSC/DECRC.
    out.push_str("\x1b[?7h");
    out
}

#[derive(Debug, serde::Serialize)]
pub struct InteractiveOutcome {
    pub signal: Option<i32>,
    pub keys_delivered: u64,
    pub keys_pending: usize,
    pub notices: Vec<String>,
    pub delivered_keys: std::collections::VecDeque<crate::report::KeyInput>,
    pub replay_complete: bool,
}
/// Run to an absolute guest step limit, or until host exit/guest fault.
pub fn run(
    machine: &mut Machine,
    limit: Option<u64>,
    bindings: BTreeMap<String, u8>,
) -> Result<InteractiveOutcome, String> {
    if !machine.config().extended()
        || !machine.config().experimental()
        || machine.config().capture() != crate::config::CaptureMode::Tail
        || machine.bus.terminal.capture_mode() != crate::config::CaptureMode::Tail
    {
        return Err("interactive requires extended experimental terminal with tail capture".into());
    }
    let signals = crate::signals::SignalSession::enter().map_err(|e| e.to_string())?;
    let _session = RawSession::enter()?;
    let start = Instant::now();
    let mut k = Keyboard::new();
    k.bindings = bindings;
    let mut unknown = 0;
    let (mut height, mut width) = size()?;
    let mut usable = height >= 25 && width >= 80;
    let mut renderer = FrameRenderer::default();
    renderer.draw(machine, &k, 0, height, width)?;
    let mut last_draw = Instant::now();
    let end = limit;
    let mut next_poll = machine.steps;
    let mut signal = None;
    let mut history = crate::report::ReplayHistory::default();
    while machine.reason.is_none() && end.is_none_or(|end| machine.steps < end) {
        if let Some(number) = signals.pending_signal() {
            signal = Some(number);
            machine.reason = Some("signal".into());
            break;
        }
        if machine.steps >= next_poll {
            next_poll = machine.steps.saturating_add(2048);
            (height, width) = size()?;
            usable = height >= 25 && width >= 80;
            let now = start.elapsed().as_millis() as u64;
            // Poll never blocks the guest, and VMIN=0 makes reads bounded.
            let mut descriptor = libc::pollfd {
                fd: 0,
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut descriptor, 1, 0) };
            if ready < 0 {
                let err = io::Error::last_os_error();
                if err.kind() != io::ErrorKind::Interrupted {
                    return Err(err.to_string());
                }
            }
            if descriptor.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
                return Err("interactive input disconnected".into());
            }
            if descriptor.revents & libc::POLLIN != 0 {
                let mut bytes = [0u8; 256];
                let n = unsafe { libc::read(0, bytes.as_mut_ptr().cast(), bytes.len()) };
                if n < 0 {
                    let err = io::Error::last_os_error();
                    if !matches!(
                        err.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                    ) {
                        return Err(err.to_string());
                    }
                } else if n == 0 {
                    machine.reason = Some("input_eof".into());
                    break;
                } else {
                    for &b in &bytes[..n as usize] {
                        if b == 3
                            || b == 29
                            || (usable
                                && machine.bus.terminal.total_display_bytes > 0
                                && k.feed(b, now))
                        {
                            machine.reason = Some("user_exit".into());
                            break;
                        }
                    }
                }
            }
            if machine.reason.is_some() {
                break;
            }
            if descriptor.revents & libc::POLLHUP != 0 {
                machine.reason = Some("input_eof".into());
                break;
            }
            k.expire(now);
            if machine.bus.terminal.screen.unknown_count != unknown {
                unknown = machine.bus.terminal.screen.unknown_count;
                k.notify(
                    &format!("{unknown} display bytes undecoded; retained in report."),
                    now,
                );
            }
            if last_draw.elapsed() >= Duration::from_millis(50) {
                renderer.draw(machine, &k, now, height, width)?;
                last_draw = Instant::now();
            }
        }
        if usable
            && machine.bus.terminal.total_display_bytes > 0
            && machine.bus.terminal.connected
            && machine.steps >= k.next_step
        {
            if let Some(key) = k.pending.front().copied() {
                if machine.send_key(key).is_err() {
                    break;
                }
                k.pending.pop_front();
                history.record(machine.steps, key);
                k.delivered += 1;
                k.next_step = machine.steps.saturating_add(2000);
            }
        }
        machine.step();
    }
    if machine.reason.is_none() {
        machine.reason = Some("step_limit".into());
    }
    let (delivered_keys, replay_complete) = history.into_parts();
    Ok(InteractiveOutcome {
        delivered_keys,
        replay_complete,
        signal,
        keys_delivered: k.delivered,
        keys_pending: k.pending.len(),
        notices: k.notices.into_iter().collect(),
    })
}

#[derive(Default)]
struct ActivityLamp {
    label: Option<&'static str>,
    until: u64,
}
impl ActivityLamp {
    fn flash(&mut self, label: &'static str, now: u64) {
        self.label = Some(label);
        self.until = now.saturating_add(200);
    }
    fn label(&self, now: u64) -> &'static str {
        if now < self.until {
            self.label.unwrap_or("IDLE")
        } else {
            "IDLE"
        }
    }
}
#[derive(Default)]
struct DeviceLights {
    disk_totals: (u64, u64, u64),
    tape_generation: u64,
    disk: ActivityLamp,
    tape: ActivityLamp,
}
impl DeviceLights {
    fn update(&mut self, machine: &Machine, now: u64) {
        let disk = &machine.bus.disk;
        let totals = (disk.reads, disk.writes, disk.controls);
        let read = totals.0 != self.disk_totals.0;
        let write = totals.1 != self.disk_totals.1;
        if read || write || totals.2 != self.disk_totals.2 {
            self.disk.flash(
                match (read, write) {
                    (true, true) => "R/W",
                    (true, false) => "READ",
                    (false, true) => "WRITE",
                    _ => "CTRL",
                },
                now,
            );
        }
        self.disk_totals = totals;
        if let Some(tape) = &machine.bus.tape {
            let (generation, label) = tape.activity();
            if generation != self.tape_generation {
                self.tape.flash(label, now);
            }
            self.tape_generation = generation;
        }
    }
    fn paint(&self, out: &mut String, machine: &Machine, now: u64, height: usize, width: usize) {
        fn color(label: &str) -> u8 {
            match label {
                "READ" => 46,
                "WRITE" | "R/W" | "MARK" | "ERASE" => 208,
                "ERROR" => 196,
                "IDLE" | "OFF" => 244,
                _ => 45,
            }
        }
        fn count(value: u64) -> String {
            if value > 999_999 {
                "99999+".into()
            } else {
                format!("{value:>6}")
            }
        }
        let disk = &machine.bus.disk;
        let disk_label = if machine
            .reason
            .as_deref()
            .is_some_and(|r| r.contains("disk"))
        {
            "ERROR"
        } else if disk.busy {
            "BUSY"
        } else {
            self.disk.label(now)
        };
        out.push_str(&format!("\x1b[38;5;{}m", color(disk_label)));
        put(
            out,
            24,
            0,
            &format!(
                "DISK {disk_label:<6} R:{} W:{}",
                count(disk.reads),
                count(disk.writes)
            ),
            height,
            32,
        );
        let (mode, label, position, objects) = match &machine.bus.tape {
            Some(tape) => {
                let status = tape.status();
                let label = if status.error.is_some() {
                    "ERROR"
                } else if tape.busy {
                    "BUSY"
                } else {
                    self.tape.label(now)
                };
                (
                    if status.mode == "read_only" {
                        "RO"
                    } else {
                        "RW"
                    },
                    label,
                    status.position,
                    status.records + status.marks,
                )
            }
            None => ("---", "OFF", 0, 0),
        };
        out.push_str(&format!("\x1b[38;5;{}m", color(label)));
        put(
            out,
            24,
            32,
            &format!("TAPE {mode:<3} {label:<6} {position:>6}/{objects:>6}"),
            height,
            62,
        );
        out.push_str("\x1b[0m");
        put(
            out,
            24,
            62,
            " | Ctrl-] exits",
            height,
            width.saturating_sub(1),
        );
    }
}

#[derive(Default)]
struct FrameRenderer {
    lights: DeviceLights,
    previous: String,
    size: Option<(usize, usize)>,
}
impl FrameRenderer {
    fn frame(
        &mut self,
        machine: &Machine,
        keyboard: &Keyboard,
        now: u64,
        height: usize,
        width: usize,
    ) -> String {
        self.lights.update(machine, now);
        let frame = render_frame(machine, keyboard, &self.lights, now, height, width);
        let resized = self.size != Some((height, width));
        if !resized && frame == self.previous {
            return String::new();
        }
        self.size = Some((height, width));
        self.previous = frame.clone();
        if resized {
            format!("\x1b[0m\x1b[2J{frame}")
        } else {
            frame
        }
    }
    fn draw(
        &mut self,
        machine: &Machine,
        keyboard: &Keyboard,
        now: u64,
        height: usize,
        width: usize,
    ) -> Result<(), String> {
        let frame = self.frame(machine, keyboard, now, height, width);
        if frame.is_empty() {
            Ok(())
        } else {
            write_output(frame.as_bytes())
        }
    }
}
#[cfg(test)]
mod renderer_tests {
    use super::*;
    mod fixture {
        use crate as rocc_emulator;
        include!("../tests/common/tape.rs");
    }
    fn machine() -> Machine {
        let mut machine = Machine::new(vec![0; 6144], true, 16, true, None).unwrap();
        machine.bus.terminal.total_display_bytes = 1;
        machine
    }
    fn plain(frame: &str) -> String {
        let mut text = String::new();
        let mut chars = frame.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' && chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            } else {
                text.push(c);
            }
        }
        text
    }
    #[test]
    fn unchanged_screen_emits_nothing_and_changed_screen_does_not_clear() {
        let mut machine = machine();
        let keyboard = Keyboard::new();
        let mut renderer = FrameRenderer::default();
        assert!(renderer
            .frame(&machine, &keyboard, 0, 25, 80)
            .contains("\x1b[2J"));
        assert_eq!(renderer.frame(&machine, &keyboard, 50, 25, 80), "");
        machine.bus.terminal.screen.cells[0][0] = b'X';
        let changed = renderer.frame(&machine, &keyboard, 100, 25, 80);
        assert!(!changed.is_empty());
        assert!(!changed.contains("\x1b[2J"));
        assert!(renderer
            .frame(&machine, &keyboard, 150, 26, 81)
            .contains("\x1b[2J"));
    }
    #[test]
    fn fixed_footer_preserves_guest_bottom_row_and_cursor() {
        let mut machine = machine();
        machine.bus.terminal.screen.cells[23][79] = b'Z';
        machine.bus.terminal.screen.row = 3;
        machine.bus.terminal.screen.column = 4;
        let frame = FrameRenderer::default().frame(&machine, &Keyboard::new(), 0, 25, 80);
        let start = frame.find("\x1b[?7l").unwrap();
        let end = frame.find("\x1b[?7h").unwrap();
        let bottom = frame.find('Z').unwrap();
        let footer = frame.find("\x1b[25;1H").unwrap();
        assert!(start < bottom && bottom < footer && footer < end);
        assert!(frame.contains("\x1b[4;5H\x1b[?25h"));
        assert!(plain(&frame).contains("DISK IDLE"));
        assert!(plain(&frame).contains("TAPE --- OFF"));
    }
    #[test]
    fn disk_activity_is_visible_for_200ms_and_fields_never_move() {
        let mut machine = machine();
        let keyboard = Keyboard::new();
        let mut renderer = FrameRenderer::default();
        renderer.frame(&machine, &keyboard, 0, 25, 80);
        machine.bus.disk.reads = 1;
        let read = renderer.frame(&machine, &keyboard, 50, 25, 80);
        assert!(plain(&read).contains("DISK READ"));
        assert!(read.contains("38;5;46m"));
        assert!(renderer.frame(&machine, &keyboard, 249, 25, 80).is_empty());
        let idle = renderer.frame(&machine, &keyboard, 250, 25, 80);
        assert!(plain(&idle).contains("DISK IDLE"));
        machine.bus.disk.writes = u64::MAX;
        let write = renderer.frame(&machine, &keyboard, 300, 25, 80);
        assert!(plain(&write).contains("DISK WRITE"));
        assert!(write.contains("38;5;208m"));
        // Even counter overflow cannot shift the tape field or guest cursor.
        assert_eq!(plain(&read).find("TAPE"), plain(&write).find("TAPE"));
        assert!(!write.contains("\x1b[2J"));
    }
    #[test]
    fn tape_motion_survives_polling_and_errors_are_red() {
        let f = fixture::Fixture::new();
        let (mut machine, _image) = f.machine();
        machine.bus.terminal.total_display_bytes = 1;
        machine
            .attach_tape(
                crate::tape::TapeMount::CreateNew(f.dir.join("reel")),
                &f.image,
                &f.overlay,
                None,
            )
            .unwrap();
        let keyboard = Keyboard::new();
        let mut renderer = FrameRenderer::default();
        renderer.frame(&machine, &keyboard, 0, 25, 80);
        machine
            .bus
            .io(&mut machine.cpu.memory, 0o20, 2, 1, 8)
            .unwrap();
        machine
            .bus
            .tape
            .as_mut()
            .unwrap()
            .tick(&mut machine.cpu.memory)
            .unwrap();
        for _ in 0..300 {
            machine
                .bus
                .io(&mut machine.cpu.memory, 0o20, 1, 0, 0)
                .unwrap();
        }
        let rewind = renderer.frame(&machine, &keyboard, 50, 25, 80);
        assert!(plain(&rewind).contains("TAPE RW  REWIND"));
        machine
            .bus
            .io(&mut machine.cpu.memory, 0o20, 2, 1, 0x18)
            .unwrap();
        machine
            .bus
            .tape
            .as_mut()
            .unwrap()
            .tick(&mut machine.cpu.memory)
            .unwrap();
        assert!(plain(&renderer.frame(&machine, &keyboard, 100, 25, 80)).contains("TAPE RW  SPACE"));
        machine
            .bus
            .io(&mut machine.cpu.memory, 0o20, 2, 0, 0x28)
            .unwrap();
        machine
            .bus
            .io(&mut machine.cpu.memory, 0o20, 4, 0, 300)
            .unwrap();
        machine
            .bus
            .io(&mut machine.cpu.memory, 0o20, 6, 1, 0xffff)
            .unwrap();
        assert!(machine
            .bus
            .tape
            .as_mut()
            .unwrap()
            .tick(&mut machine.cpu.memory)
            .is_err());
        let error = renderer.frame(&machine, &keyboard, 150, 25, 80);
        assert!(plain(&error).contains("TAPE RW  ERROR"));
        assert!(error.contains("38;5;196m"));
    }
    #[test]
    fn readonly_tape_read_updates_position_without_moving_footer() {
        let f = fixture::Fixture::new();
        let (mut machine, _image) = f.machine();
        let reel = f.dir.join("reel");
        std::fs::write(&reel, [2, 0, 0, 0, 0x12, 0x34, 2, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        machine
            .attach_tape(
                crate::tape::TapeMount::ReadOnly(reel),
                &f.image,
                &f.overlay,
                None,
            )
            .unwrap();
        let mut renderer = FrameRenderer::default();
        let keyboard = Keyboard::new();
        let boot = renderer.frame(&machine, &keyboard, 0, 25, 80);
        assert!(plain(&boot).contains("TAPE RO  IDLE"));
        machine
            .bus
            .io(&mut machine.cpu.memory, 0o20, 2, 0, 0)
            .unwrap();
        machine
            .bus
            .io(&mut machine.cpu.memory, 0o20, 4, 0, 300)
            .unwrap();
        machine
            .bus
            .io(&mut machine.cpu.memory, 0o20, 6, 1, 0xffff)
            .unwrap();
        machine
            .bus
            .tape
            .as_mut()
            .unwrap()
            .tick(&mut machine.cpu.memory)
            .unwrap();
        let read = renderer.frame(&machine, &keyboard, 50, 25, 80);
        assert!(plain(&read).contains("TAPE RO  READ"));
        assert!(plain(&read).contains("     1/     2"));
        assert!(boot.contains("\x1b[25;1H"));
        assert!(read.contains("\x1b[25;1H"));
    }
    #[test]
    fn undersized_terminal_pauses_guest_display_and_resize_restores_footer() {
        let machine = machine();
        let keyboard = Keyboard::new();
        let mut renderer = FrameRenderer::default();
        let small = renderer.frame(&machine, &keyboard, 0, 24, 80);
        assert!(small.contains("80 columns x 25 rows"));
        assert!(!small.contains("TAPE"));
        let normal = renderer.frame(&machine, &keyboard, 50, 25, 80);
        assert!(normal.contains("\x1b[2J"));
        assert!(plain(&normal).contains("DISK"));
    }
    #[test]
    fn notices_expire_back_to_device_footer_without_touching_guest_rows() {
        let machine = machine();
        let mut keyboard = Keyboard::new();
        let mut renderer = FrameRenderer::default();
        keyboard.notify("A long notice to erase", 0);
        assert!(renderer
            .frame(&machine, &keyboard, 0, 25, 80)
            .contains("A long notice to erase"));
        keyboard.notify("Short", 1);
        let short = renderer.frame(&machine, &keyboard, 1, 25, 80);
        assert!(short.contains(&format!("\x1b[25;1H{:<79}", "Short")));
        let expired = renderer.frame(&machine, &keyboard, 3001, 25, 80);
        assert!(!expired.contains("Short"));
        assert!(plain(&expired).contains("DISK IDLE"));
        assert!(!expired.contains("\x1b[2J"));
    }
}
