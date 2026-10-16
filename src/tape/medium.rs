use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

pub(super) const MAX_BYTES: u64 = 512 * 1024 * 1024;
pub(super) const MAX_RECORD: u32 = 0xFF_FFFF;
pub(super) const MAX_OBJECTS: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TapeObject {
    Data { offset: u64, length: u32 },
    Mark,
}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum TapeRead {
    Data(Vec<u8>),
    Mark,
    End,
}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum TapeStep {
    Record { bytes: u32 },
    Mark,
    End,
}
#[derive(Clone, Copy)]
pub(super) enum Direction {
    Forward,
    Reverse,
}

pub(super) struct TapeMedium {
    pub(super) objects: Vec<TapeObject>,
    pub(super) offsets: Vec<u64>,
    pub(super) end: u64,
    cursor: usize,
}
impl TapeMedium {
    pub(super) fn scan(file: &mut File) -> Result<Self, String> {
        let bytes = file.metadata().map_err(|e| e.to_string())?.len();
        if bytes > MAX_BYTES {
            return Err("tape exceeds 512 MiB limit".into());
        }
        let mut result = Self {
            objects: Vec::new(),
            offsets: Vec::new(),
            end: bytes,
            cursor: 0,
        };
        let mut offset = 0;
        while offset < bytes {
            if result.objects.len() >= MAX_OBJECTS {
                return Err("tape exceeds object limit".into());
            }
            let length = read_word(file, offset)?;
            if length == u32::MAX {
                if offset + 4 != bytes {
                    return Err("unsupported data after tape end marker".into());
                }
                result.end = offset;
                break;
            }
            result.offsets.push(offset);
            if length == 0 {
                result.objects.push(TapeObject::Mark);
                offset += 4;
            } else {
                if length > MAX_RECORD {
                    return Err("unsupported tape object or record length".into());
                }
                let trailer = offset + 4 + u64::from(length) + u64::from(length & 1);
                if trailer + 4 > bytes {
                    return Err("truncated tape record".into());
                }
                if read_word(file, trailer)? != length {
                    return Err("tape record lengths differ".into());
                }
                result.objects.push(TapeObject::Data { offset, length });
                offset = trailer + 4;
            }
        }
        Ok(result)
    }
    pub(super) fn read(&mut self, file: &mut File) -> Result<TapeRead, String> {
        let value = match self.objects.get(self.cursor) {
            Some(TapeObject::Data { offset, length }) => {
                if read_word(file, *offset)? != *length
                    || read_word(
                        file,
                        offset + 4 + u64::from(*length) + u64::from(length & 1),
                    )? != *length
                {
                    return Err("tape record changed after mounting".into());
                }
                file.seek(SeekFrom::Start(offset + 4))
                    .map_err(|e| e.to_string())?;
                let mut data = vec![0; *length as usize];
                file.read_exact(&mut data).map_err(|e| e.to_string())?;
                TapeRead::Data(data)
            }
            Some(TapeObject::Mark) => {
                if read_word(file, self.offsets[self.cursor])? != 0 {
                    return Err("tape mark changed after mounting".into());
                }
                TapeRead::Mark
            }
            None => return Ok(TapeRead::End),
        };
        self.cursor += 1;
        Ok(value)
    }
    pub(super) fn space(&mut self, direction: Direction) -> Result<TapeStep, String> {
        let index = match direction {
            Direction::Forward if self.cursor < self.objects.len() => {
                let index = self.cursor;
                self.cursor += 1;
                index
            }
            Direction::Reverse if self.cursor > 0 => {
                self.cursor -= 1;
                self.cursor
            }
            _ => return Ok(TapeStep::End),
        };
        Ok(match self.objects[index] {
            TapeObject::Data { length, .. } => TapeStep::Record { bytes: length },
            TapeObject::Mark => TapeStep::Mark,
        })
    }
    pub(super) fn rewind(&mut self) {
        self.cursor = 0;
    }
    pub(super) fn position(&self) -> usize {
        self.cursor
    }
}
fn read_word(file: &mut File, offset: u64) -> Result<u32, String> {
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    let mut word = [0; 4];
    file.read_exact(&mut word)
        .map_err(|e| format!("tape metadata at {offset}: {e}"))?;
    Ok(u32::from_le_bytes(word))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, OpenOptions};
    use std::io::{Seek, SeekFrom, Write};
    use std::sync::atomic::{AtomicU64, Ordering};
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    fn file(raw: &[u8]) -> std::fs::File {
        let path = std::env::temp_dir().join(format!(
            "rocc-tape-medium-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        fs::remove_file(path).unwrap();
        file.write_all(raw).unwrap();
        file.rewind().unwrap();
        file
    }
    #[test]
    fn standard_frames_and_cursor() {
        let mut f = file(&[
            3, 0, 0, 0, b'a', b'b', b'c', 0, 3, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0x12, 0x34, 2, 0,
            0, 0,
        ]);
        let mut m = TapeMedium::scan(&mut f).unwrap();
        assert_eq!(m.position(), 0);
        assert_eq!(m.read(&mut f).unwrap(), TapeRead::Data(b"abc".to_vec()));
        assert_eq!(m.position(), 1);
        assert_eq!(m.read(&mut f).unwrap(), TapeRead::Mark);
        assert_eq!(m.space(Direction::Reverse).unwrap(), TapeStep::Mark);
        assert_eq!(m.position(), 1);
        assert_eq!(m.space(Direction::Forward).unwrap(), TapeStep::Mark);
        assert_eq!(m.read(&mut f).unwrap(), TapeRead::Data(vec![0x12, 0x34]));
        assert_eq!(m.read(&mut f).unwrap(), TapeRead::End);
        assert_eq!(m.read(&mut f).unwrap(), TapeRead::End);
        assert_eq!(m.position(), 3);
        m.rewind();
        assert_eq!(m.space(Direction::Reverse).unwrap(), TapeStep::End);
        assert_eq!(m.position(), 0);
        assert_eq!(
            m.space(Direction::Forward).unwrap(),
            TapeStep::Record { bytes: 3 }
        );
    }
    #[test]
    fn rejects_unsupported_or_malformed_objects() {
        let valid = [3, 0, 0, 0, b'a', b'b', b'c', 0xff, 3, 0, 0, 0];
        assert!(TapeMedium::scan(&mut file(&valid)).is_ok());
        for n in 1..valid.len() {
            assert!(
                TapeMedium::scan(&mut file(&valid[..n])).is_err(),
                "length {n}"
            );
        }
        for raw in [
            vec![1, 0, 0, 0, 0, 0, 2, 0, 0, 0],
            vec![1, 0, 0, 0x80],
            vec![1, 0, 0, 0x10],
            vec![0xfe, 0xff, 0xff, 0xff],
            vec![0xff, 0xff, 0xff, 0xff, 0],
        ] {
            assert!(TapeMedium::scan(&mut file(&raw)).is_err(), "{raw:?}");
        }
        for raw in [vec![], vec![0xff, 0xff, 0xff, 0xff]] {
            let mut f = file(&raw);
            let mut m = TapeMedium::scan(&mut f).unwrap();
            assert_eq!(m.read(&mut f).unwrap(), TapeRead::End);
        }
    }
    #[test]
    fn resource_ceilings_precede_allocation() {
        let mut f = file(&[]);
        f.set_len(512 * 1024 * 1024 + 1).unwrap();
        assert!(TapeMedium::scan(&mut f).is_err());
        assert!(TapeMedium::scan(&mut file(&0x1000000u32.to_le_bytes())).is_err());
        let mut f = file(&vec![0; 100_000 * 4]);
        assert!(TapeMedium::scan(&mut f).is_ok());
        f.seek(SeekFrom::End(0)).unwrap();
        f.write_all(&[0; 4]).unwrap();
        assert!(TapeMedium::scan(&mut f).is_err());
        let mut f = file(&[]);
        for n in 0..32 {
            let len: u32 = if n < 31 { 0xffffff } else { 0xffff00 };
            f.write_all(&len.to_le_bytes()).unwrap();
            f.seek(SeekFrom::Current(
                (len as u64 + u64::from(len & 1 != 0)) as i64,
            ))
            .unwrap();
            f.write_all(&len.to_le_bytes()).unwrap();
        }
        assert_eq!(f.metadata().unwrap().len(), 512 * 1024 * 1024);
        assert!(TapeMedium::scan(&mut f).is_ok());
    }
}
