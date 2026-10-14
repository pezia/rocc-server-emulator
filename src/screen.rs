use serde_json::{json, Value};
use std::collections::VecDeque;

/// Recovered/provisional ROCC controls; never passed through as host ANSI.
pub struct Screen {
    pub cells: [[u8; 80]; 24],
    pub attributes: [[u8; 80]; 24],
    pub row: usize,
    pub column: usize,
    pub attribute: u8,
    pub unknown_count: u64,
    pub revision: u64,
    saved: Option<(usize, usize, u8)>,
    field_end: Option<(usize, usize)>,
    pending: Vec<u8>,
    offset: u64,
    controls: VecDeque<Value>,
    unknown: VecDeque<Value>,
}
impl Default for Screen {
    fn default() -> Self {
        Self::new()
    }
}
impl Screen {
    pub fn new() -> Self {
        Self {
            cells: [[b' '; 80]; 24],
            attributes: [[0; 80]; 24],
            row: 0,
            column: 0,
            attribute: 0,
            unknown_count: 0,
            revision: 0,
            saved: None,
            field_end: None,
            pending: Vec::new(),
            offset: 0,
            controls: VecDeque::new(),
            unknown: VecDeque::new(),
        }
    }
    fn record(queue: &mut VecDeque<Value>, value: Value) {
        if queue.len() == 256 {
            queue.pop_front();
        }
        queue.push_back(value);
    }
    pub fn feed(&mut self, raw: &[u8]) {
        self.pending.extend_from_slice(raw);
        let mut i = 0;
        while i < self.pending.len() {
            // Application-area clear. Recognize the complete sequence:
            // treating 90 as a two-byte
            // setup command swallowed the second A8 and left old menu text.
            // Rows 0..2 hold status/header text and are cleared separately.
            // This is a bounded model of the observed sequence, not a claim
            // about arbitrary A8 modes or standalone 90 controls.
            const APPLICATION_CLEAR: &[u8] = b"\xa8\x01\x90\xa8\x00";
            let remaining = &self.pending[i..];
            if remaining.len() < APPLICATION_CLEAR.len() && APPLICATION_CLEAR.starts_with(remaining)
            {
                break;
            }
            if remaining.starts_with(APPLICATION_CLEAR) {
                self.cells[3..].fill([b' '; 80]);
                self.attributes[3..].fill([0; 80]);
                Self::record(&mut self.controls, json!(APPLICATION_CLEAR));
                i += APPLICATION_CLEAR.len();
                self.revision += 1;
                continue;
            }
            // Input teardown uses this exact sequence after clearing
            // the guest's input state (F8CB..F8DD in the H/1/1 trace). Decode
            // the complete operation; standalone B4/B2/95 remain unresolved.
            const FIELD_COMPLETE: &[u8] = b"\xb4\xb2\xb8\xa2\x00\x95";
            if remaining.len() < FIELD_COMPLETE.len() && FIELD_COMPLETE.starts_with(remaining) {
                break;
            }
            if remaining.starts_with(FIELD_COMPLETE) {
                self.field_end = None;
                if let Some((row, column, _)) = self.saved.take() {
                    self.row = row;
                    self.column = column;
                }
                // B8 restores any overlay, then A2 00 resets the rendition.
                self.attribute = 0;
                Self::record(&mut self.controls, json!(FIELD_COMPLETE));
                i += FIELD_COMPLETE.len();
                self.revision += 1;
                continue;
            }
            let value = self.pending[i];
            let size = match value {
                0xa0 | 0xa3 => 3,
                0xa8 | 0xa2 | 0x90 | 0x80 | 0xae | 0xaa | 0xab | 0xb6 | 0xb9 => 2,
                _ => 1,
            };
            if i + size > self.pending.len() {
                break;
            }
            match value {
                0xa0 => {
                    let r = self.pending[i + 1];
                    let c = self.pending[i + 2];
                    self.row = (r & 127) as usize;
                    self.column = (c & 127) as usize;
                    if (r | c) & 128 != 0 {
                        Self::record(&mut self.controls, json!([value, r, c]));
                    }
                }
                0xa3 => {
                    let r = self.pending[i + 1];
                    let c = self.pending[i + 2];
                    self.field_end = Some(((r & 127) as usize, (c & 127) as usize));
                    Self::record(&mut self.controls, json!([value, r, c]));
                }
                0xb6 => {
                    let fill = self.pending[i + 1];
                    Self::record(&mut self.controls, json!([value, fill]));
                    // Observed single-row input fields: inclusive endpoint and
                    // no cursor movement, so immediate keyboard echo replaces
                    // the first fill character. Other region shapes stay opaque.
                    if let Some((row, end)) = self.field_end {
                        if row == self.row
                            && row < 24
                            && self.column <= end
                            && end < 80
                            && (32..=126).contains(&fill)
                        {
                            self.cells[row][self.column..=end].fill(fill);
                            self.attributes[row][self.column..=end].fill(self.attribute);
                        }
                    }
                }
                0xa8 | 0xa2 | 0x90 | 0x80 | 0xae | 0xaa | 0xab | 0xb9 => {
                    // Consume one parameter; unmodeled mode meanings stay opaque.
                    let arg = self.pending[i + 1];
                    Self::record(&mut self.controls, json!([value, arg]));
                    if value == 0xa8 && arg == 0 {
                        self.cells = [[b' '; 80]; 24];
                        self.attributes = [[0; 80]; 24];
                        self.row = 0;
                        self.column = 0;
                    } else if value == 0xa2 {
                        self.attribute = arg;
                    }
                }
                0xb7 => {
                    self.saved = Some((self.row, self.column, self.attribute));
                    Self::record(&mut self.controls, json!([value]));
                }
                0xb8 => {
                    // Only restore an outstanding status-overlay save. Field
                    // completion also emits B8 without a preceding B7; restoring
                    // an invented (0,0) or stale save moved the input cursor.
                    if let Some(saved) = self.saved.take() {
                        (self.row, self.column, self.attribute) = saved;
                    }
                    Self::record(&mut self.controls, json!([value]));
                }
                32..=126 => {
                    if self.row < 24 && self.column < 80 {
                        self.cells[self.row][self.column] = value;
                        self.attributes[self.row][self.column] = self.attribute;
                    }
                    self.column += 1;
                }
                8 => self.column = self.column.saturating_sub(1),
                13 => self.column = 0,
                10 => self.row = (self.row + 1).min(23),
                9 => self.column = ((self.column / 8 + 1) * 8).min(79),
                7 => Self::record(&mut self.controls, json!([value])),
                _ => {
                    Self::record(
                        &mut self.unknown,
                        json!({"offset":self.offset+i as u64,"byte":value}),
                    );
                    self.unknown_count += 1;
                }
            }
            i += size;
            self.revision += 1;
        }
        self.pending.drain(..i);
        self.offset += i as u64;
    }
    pub fn snapshot(&self) -> Value {
        let lines: Vec<String> = self
            .cells
            .iter()
            .map(|r| String::from_utf8_lossy(r).trim_end().to_owned())
            .collect();
        json!({"lines":lines,"cursor":[self.row,self.column],"setup_controls":self.controls,"unknown_bytes":self.unknown,"unknown_count":self.unknown_count,"pending_hex":hex(&self.pending)})
    }
}
pub(crate) fn hex(raw: &[u8]) -> String {
    const DIGITS: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(raw.len() * 2);
    for b in raw {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 15) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn login_setup_preserves_position_command_across_splits() {
        let raw = b"\xb9\x80\xa0\x06\x19Password : \xa3\x06\x29\xb6_";
        for split in 0..=raw.len() {
            let mut s = Screen::new();
            s.feed(&raw[..split]);
            s.feed(&raw[split..]);
            assert_eq!(
                s.snapshot()["lines"][6],
                format!("{}Password : ______", " ".repeat(25))
            );
            assert_eq!((s.row, s.column), (6, 36));
            assert_eq!(s.unknown_count, 0);
        }
    }
    #[test]
    fn completion_sequence_ends_field_and_preserves_text_across_splits() {
        let completion = b"\xb4\xb2\xb8\xa2\x00\x95";
        for split in 0..=completion.len() {
            let mut s = Screen::new();
            s.feed(b"\xa0\x13\x27\xa3\x13\x27\xb6_1");
            s.attribute = 8;
            s.feed(&completion[..split]);
            s.feed(&completion[split..]);
            assert_eq!(s.unknown_count, 0, "split {split}");
            assert_eq!(s.field_end, None);
            assert_eq!((s.row, s.column, s.attribute), (19, 40, 0));
            assert_eq!(s.cells[19][39], b'1');
            // A subsequent fill must not reuse the completed field bounds.
            s.feed(b"\xa0\x13\x27\xb6_");
            assert_eq!(s.cells[19][39], b'1');
        }
    }
    #[test]
    fn completion_with_overlay_restores_cursor_and_resets_rendition() {
        let mut s = Screen::new();
        s.feed(b"\xa0\x13\x27\xa2\x04\xb7\xa0\x00\x00\xa2\x08NOTICE");
        s.feed(b"\xb4\xb2\xb8\xa2\x00\x95");
        assert_eq!((s.row, s.column, s.attribute), (19, 39, 0));
        assert_eq!(s.snapshot()["lines"][0], "NOTICE");
        assert_eq!(s.attributes[0][0], 8);
        assert_eq!(s.unknown_count, 0);
    }
    #[test]
    fn incomplete_completion_pattern_keeps_unknown_warnings() {
        let mut s = Screen::new();
        s.feed(b"\xb4\xb2\xb8\xa2\x00\x94");
        assert_eq!(s.unknown_count, 3);
        assert_eq!(s.snapshot()["unknown_bytes"][0]["byte"], 0xb4);
        assert_eq!(s.snapshot()["unknown_bytes"][1]["byte"], 0xb2);
        assert_eq!(s.snapshot()["unknown_bytes"][2]["byte"], 0x94);
        s.feed(b"\xb4X\xb2\x95");
        assert_eq!(s.unknown_count, 6);
        assert_eq!(s.cells[0][0], b'X');
    }
    #[test]
    fn restore_without_outstanding_save_does_not_jump_to_origin() {
        let mut s = Screen::new();
        s.feed(b"\xa0\x13\x27\xb8");
        assert_eq!((s.row, s.column), (19, 39));
        s.feed(b"\xb7\xa0\x00\x00ERROR\xb8");
        assert_eq!((s.row, s.column), (19, 39));
        s.feed(b"1\xb8");
        assert_eq!((s.row, s.column), (19, 40));
    }
    #[test]
    fn input_field_endpoint_and_fill_do_not_print_parameters() {
        for split in 0..=8 {
            let raw = b"\xa0\x13\x27\xa3\x13\x27\xb6_";
            let mut s = Screen::new();
            s.feed(&raw[..split]);
            s.feed(&raw[split..]);
            assert_eq!(s.cells[19][39], b'_');
            assert_eq!(s.cells[19][40], b' ');
            assert_eq!((s.row, s.column), (19, 39));
            assert_eq!(s.unknown_count, 0);
            s.feed(b"1");
            assert_eq!(s.snapshot()["lines"][19], format!("{}1", " ".repeat(39)));
        }
    }
    #[test]
    fn application_clear_preserves_status_area_across_every_packet_split() {
        let raw = b"\xa8\x01\x90\xa8\x00\xa0\x03\x00NEW";
        for split in 0..=raw.len() {
            let mut s = Screen::new();
            s.feed(b"\xa0\x01\x00HEADER\xa0\x03\x00OLD MENU\xa0\x17\x00OLD FOOTER");
            s.attributes[23][0] = 8;
            s.feed(&raw[..split]);
            s.feed(&raw[split..]);
            assert_eq!(s.snapshot()["lines"][1], "HEADER", "split {split}");
            assert_eq!(s.snapshot()["lines"][3], "NEW", "split {split}");
            assert_eq!(s.snapshot()["lines"][23], "", "split {split}");
            assert_eq!(s.attributes[23][0], 0);
            assert_eq!(s.unknown_count, 0);
        }
    }
    #[test]
    fn fragmented_overlay_preserves_cursor_and_attribute() {
        let mut s = Screen::new();
        for bytes in [
            &b"\xa0\x16\x27\xb7\xa0\x80"[..],
            &b"\0\xa2\x08INVALID"[..],
            &b"\xb8\xae"[..],
            &b"\x40"[..],
        ] {
            s.feed(bytes);
        }
        assert_eq!(s.snapshot()["lines"][0], "INVALID");
        assert_eq!((s.row, s.column), (22, 39));
        assert_eq!(s.attributes[0][0], 8);
        assert_eq!(s.snapshot()["pending_hex"], "");
    }
    #[test]
    fn unknown_controls_are_safe_and_history_is_bounded() {
        let mut s = Screen::new();
        s.feed(&[27; 300]);
        s.feed(b"[31m");
        assert_eq!(s.unknown_count, 300);
        assert_eq!(s.snapshot()["unknown_bytes"].as_array().unwrap().len(), 256);
        assert_eq!(s.snapshot()["unknown_bytes"][0]["offset"], 44);
        assert_eq!(s.snapshot()["lines"][0], "[31m");
    }
}
