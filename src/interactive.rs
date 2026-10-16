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
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Write};
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
            let session = Self { original };
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
        let _ = write_output(b"\x1b[0m\x1b[?1049l\x1b8\x1b[?25h");
    }
}
fn write_output(bytes: &[u8]) -> Result<(), String> {
    let mut out = io::stdout().lock();
    out.write_all(bytes)
        .and_then(|_| out.flush())
        .map_err(|e| e.to_string())
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
fn render_frame(machine: &Machine, k: &Keyboard, now: u64, height: usize, width: usize) -> String {
    let terminal = &machine.bus.terminal;
    let screen = &terminal.screen;
    let usable = height >= 24 && width >= 80;
    let mut out = String::from("\x1b[?25l\x1b[0m\x1b[?7l");
    if !usable {
        put(
            &mut out,
            0,
            0,
            "Resize to at least 80 columns x 24 rows. Ctrl-] exits.",
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
    // Guest repaint covers this row; boot/resize messages otherwise leave an
    // expired notice behind now that normal frames no longer clear the display.
    if height == 24 && (!usable || terminal.total_display_bytes == 0) {
        put(&mut out, 23, 0, "", height, width);
    }
    if height == 24 && !k.notice.is_empty() && now.saturating_sub(k.notice_time) < 3000 {
        out.push_str("\x1b[7m");
        put(
            &mut out,
            23,
            0,
            &format!("{:<79}", k.notice),
            height,
            width.min(79),
        );
        out.push_str("\x1b[0m");
    } else if height > 24 {
        let status = if k.notice.is_empty() || now.saturating_sub(k.notice_time) >= 3000 {
            "Ctrl-] exits | Original disk unchanged; session writes are private"
        } else {
            &k.notice
        };
        put(&mut out, 24, 0, status, height, width.saturating_sub(1));
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

/// Run to an absolute guest step limit, or until host exit/guest fault.
pub fn run(
    machine: &mut Machine,
    limit: Option<u64>,
    bindings: BTreeMap<String, u8>,
) -> Result<Value, String> {
    let _session = RawSession::enter()?;
    machine.bus.terminal.capture_tail = true;
    let start = Instant::now();
    let mut k = Keyboard::new();
    k.bindings = bindings;
    let mut unknown = 0;
    let (mut height, mut width) = size()?;
    let mut usable = height >= 24 && width >= 80;
    let mut renderer = FrameRenderer::default();
    renderer.draw(machine, &k, 0, height, width)?;
    let mut last_draw = Instant::now();
    let end = limit;
    let mut next_poll = machine.steps;
    while machine.reason.is_none() && end.is_none_or(|end| machine.steps < end) {
        if machine.steps >= next_poll {
            next_poll = machine.steps.saturating_add(2048);
            (height, width) = size()?;
            usable = height >= 24 && width >= 80;
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
            if let Some(key) = k.pending.pop_front() {
                machine
                    .bus
                    .terminal
                    .send_key(key)
                    .map_err(|f| f.reason().to_string())?;
                k.delivered += 1;
                k.next_step = machine.steps.saturating_add(2000);
            }
        }
        machine.step();
    }
    if machine.reason.is_none() {
        machine.reason = Some("step_limit".into());
    }
    Ok(
        json!({"keys_delivered":k.delivered,"keys_pending":k.pending.len(),"notices":k.notices,"exit_keys":["Ctrl-]","Ctrl-C"]}),
    )
}

#[derive(Default)]
struct FrameRenderer {
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
        let frame = render_frame(machine, keyboard, now, height, width);
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
    fn machine() -> Machine {
        let mut machine = Machine::new(vec![0; 6144], true, 16, true, None).unwrap();
        machine.bus.terminal.total_display_bytes = 1;
        machine
    }
    #[test]
    fn unchanged_screen_emits_nothing_and_changed_screen_does_not_clear() {
        let mut machine = machine();
        let keyboard = Keyboard::new();
        let mut renderer = FrameRenderer::default();
        assert!(renderer
            .frame(&machine, &keyboard, 0, 24, 80)
            .contains("\x1b[2J"));
        assert_eq!(renderer.frame(&machine, &keyboard, 50, 24, 80), "");
        machine.bus.terminal.screen.cells[0][0] = b'X';
        let changed = renderer.frame(&machine, &keyboard, 100, 24, 80);
        assert!(!changed.is_empty());
        assert!(!changed.contains("\x1b[2J"));
        assert!(renderer
            .frame(&machine, &keyboard, 150, 25, 81)
            .contains("\x1b[2J"));
    }
    #[test]
    fn bottom_right_is_painted_with_wrapping_disabled_and_guest_cursor_restored() {
        let mut machine = machine();
        machine.bus.terminal.screen.cells[23][79] = b'Z';
        machine.bus.terminal.screen.row = 3;
        machine.bus.terminal.screen.column = 4;
        let frame = FrameRenderer::default().frame(&machine, &Keyboard::new(), 0, 24, 80);
        let start = frame
            .find("\x1b[?7l")
            .expect("disable wrap before painting");
        let end = frame.find("\x1b[?7h").expect("restore wrap after painting");
        let bottom = frame.find('Z').unwrap();
        assert!(start < bottom && bottom < end);
        assert!(frame.contains("\x1b[4;5H\x1b[?25h"));
    }
    #[test]
    fn expired_notice_is_erased_even_when_host_is_too_narrow() {
        let machine = machine();
        let mut keyboard = Keyboard::new();
        keyboard.notify("Temporary notice", 0);
        let mut renderer = FrameRenderer::default();
        renderer.frame(&machine, &keyboard, 0, 24, 70);
        let expired = renderer.frame(&machine, &keyboard, 3000, 24, 70);
        assert!(expired.contains(&format!("\x1b[24;1H{}", " ".repeat(70))));
    }
    #[test]
    fn tall_terminal_notice_expires_back_to_normal_status() {
        let machine = machine();
        let mut keyboard = Keyboard::new();
        let mut renderer = FrameRenderer::default();
        keyboard.notify("Temporary status", 5);
        assert!(renderer
            .frame(&machine, &keyboard, 3004, 25, 80)
            .contains("Temporary status"));
        let expired = renderer.frame(&machine, &keyboard, 3005, 25, 80);
        assert!(!expired.contains("Temporary status"));
        assert!(expired.contains("Ctrl-] exits | Original disk unchanged"));
    }
    #[test]
    fn shorter_status_and_expired_overlay_erase_previous_text_and_attributes() {
        let machine = machine();
        let mut keyboard = Keyboard::new();
        let mut renderer = FrameRenderer::default();
        keyboard.notify("A long notice to erase", 0);
        renderer.frame(&machine, &keyboard, 0, 25, 80);
        keyboard.notify("Short", 1);
        let short = renderer.frame(&machine, &keyboard, 1, 25, 80);
        assert!(short.contains(&format!("\x1b[25;1H{:<79}", "Short")));
        renderer.frame(&machine, &keyboard, 1, 24, 80);
        let expired = renderer.frame(&machine, &keyboard, 3001, 24, 80);
        assert!(!expired.contains("Short"));
        assert!(expired.contains("\x1b[24;1H"));
        assert!(!expired.contains("\x1b[7m"));
    }
}
