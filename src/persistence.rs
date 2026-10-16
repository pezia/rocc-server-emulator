//! Retained physical RAM, paired with an effective disk-overlay state.
use crate::{config::CpuProfile, memory::Memory};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const MAGIC: &[u8; 8] = b"ROCCRAM1";
const HEADER: usize = 160;

#[derive(Clone, Debug, Default, Serialize)]
pub struct MemoryStatus {
    pub enabled: bool,
    pub path: Option<PathBuf>,
    pub restored: bool,
    pub bypassed_for_cold_boot: bool,
    pub committed: bool,
    pub loaded_sha256: Option<String>,
}

#[derive(Clone)]
pub(crate) struct SnapshotIdentity {
    pub profile: CpuProfile,
    pub address_bits: u8,
    pub image_bytes: u64,
    pub image_sha256: String,
    pub overlay_sha256: String,
}

fn hash_bytes(s: &str) -> Result<Vec<u8>, String> {
    if s.len() != 64 {
        return Err("invalid SHA-256 length".into());
    }
    s.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |c: u8| (c as char).to_digit(16).ok_or("invalid SHA-256 digit");
            Ok(((digit(pair[0])? << 4) | digit(pair[1])?) as u8)
        })
        .collect()
}

fn header(memory: &Memory, identity: &SnapshotIdentity, status: u8) -> Result<Vec<u8>, String> {
    let mut raw = vec![0; HEADER];
    raw[..8].copy_from_slice(MAGIC);
    raw[8..10].copy_from_slice(&1u16.to_be_bytes());
    raw[10] = status;
    raw[11] = u8::from(identity.profile == CpuProfile::Extended);
    raw[12] = identity.address_bits;
    for (start, value) in [
        (16, memory.global_words()),
        (20, memory.bank_words()),
        (24, memory.banks()),
        (
            28,
            memory.global_words() + memory.bank_words() * memory.banks(),
        ),
    ] {
        raw[start..start + 4].copy_from_slice(&(value as u32).to_be_bytes());
    }
    raw[32..40].copy_from_slice(&identity.image_bytes.to_be_bytes());
    raw[40..72].copy_from_slice(&hash_bytes(&identity.image_sha256)?);
    raw[72..104].copy_from_slice(&hash_bytes(&identity.overlay_sha256)?);
    Ok(raw)
}

pub(crate) fn encode_snapshot(
    memory: &Memory,
    identity: &SnapshotIdentity,
) -> Result<Vec<u8>, String> {
    let mut raw = header(memory, identity, 0)?;
    raw.extend_from_slice(&memory.snapshot_bytes());
    let mut hash = Sha256::new();
    hash.update(&raw[..128]);
    hash.update(&raw[HEADER..]);
    raw[128..HEADER].copy_from_slice(&hash.finalize());
    Ok(raw)
}

fn recognize(raw: &[u8]) -> Result<(), String> {
    if raw.len() < HEADER || &raw[..8] != MAGIC {
        return Err(
            "unrecognized memory snapshot; preserve or move this file before retrying".into(),
        );
    }
    if raw[8..10] != 1u16.to_be_bytes() {
        return Err("unsupported memory snapshot version".into());
    }
    Ok(())
}

pub(crate) fn decode_snapshot(
    raw: &[u8],
    memory: &Memory,
    identity: &SnapshotIdentity,
) -> Result<Vec<u8>, String> {
    recognize(raw)?;
    if raw[10] == 1 {
        return Err("interrupted memory session; use --cold-boot to recover".into());
    }
    let expected = header(memory, identity, 0)?;
    if raw[32..72] != expected[32..72] {
        return Err("memory snapshot base image mismatch; use --cold-boot to recover".into());
    }
    if raw[72..104] != expected[72..104] {
        return Err("memory snapshot disk state mismatch; use --cold-boot to recover".into());
    }
    if raw[..128] != expected[..128] {
        return Err(
            "invalid memory header or incompatible profile/geometry; use --cold-boot to recover"
                .into(),
        );
    }
    let words = memory.global_words() + memory.bank_words() * memory.banks();
    if raw.len() != HEADER + words * 3 {
        return Err("invalid memory snapshot length".into());
    }
    let mut hash = Sha256::new();
    hash.update(&raw[..128]);
    hash.update(&raw[HEADER..]);
    if hash.finalize().as_slice() != &raw[128..HEADER] {
        return Err("memory snapshot checksum mismatch; use --cold-boot to recover".into());
    }
    let mut check = memory.clone();
    check.restore_snapshot_bytes(&raw[HEADER..])?;
    Ok(raw[HEADER..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::*, memory::Memory};
    fn fixture() -> (Memory, SnapshotIdentity) {
        let config = MachineConfig::new(
            CpuProfile::Extended,
            16,
            TerminalProfile::None,
            None,
            ResourceLimits::default(),
            CaptureMode::Stop,
        )
        .unwrap();
        (
            Memory::new(&config),
            SnapshotIdentity {
                profile: CpuProfile::Extended,
                address_bits: 16,
                image_bytes: 6144,
                image_sha256: "11".repeat(32),
                overlay_sha256: "22".repeat(32),
            },
        )
    }
    #[test]
    fn snapshot_format_roundtrip_and_identity_validation() {
        let (mut memory, identity) = fixture();
        memory.write(10941, 0xabcd).unwrap();
        let raw = encode_snapshot(&memory, &identity).unwrap();
        assert_eq!(raw.len(), 160 + 1_757_184);
        assert_eq!(&raw[..8], b"ROCCRAM1");
        assert_eq!(&raw[8..11], &[0, 1, 0]);
        assert_eq!(
            decode_snapshot(&raw, &memory, &identity).unwrap(),
            memory.snapshot_bytes()
        );
        for offset in [11, 12, 16, 20, 24, 28, 32, 40, 72] {
            let mut bad = raw.clone();
            bad[offset] ^= 1;
            assert!(
                decode_snapshot(&bad, &memory, &identity).is_err(),
                "offset {offset}"
            );
        }
        let mut changed = identity.clone();
        changed.overlay_sha256 = "33".repeat(32);
        assert!(decode_snapshot(&raw, &memory, &changed)
            .unwrap_err()
            .contains("disk state"));
        changed = identity.clone();
        changed.image_sha256 = "44".repeat(32);
        assert!(decode_snapshot(&raw, &memory, &changed)
            .unwrap_err()
            .contains("base image"));
        changed = identity.clone();
        changed.profile = CpuProfile::Loader;
        assert!(decode_snapshot(&raw, &memory, &changed).is_err());
        let flat = crate::cpu::Cpu::new(16, false).unwrap().memory;
        assert!(decode_snapshot(&raw, &flat, &identity).is_err());
        let config = MachineConfig::new(
            CpuProfile::Extended,
            16,
            TerminalProfile::Experimental,
            Some(100),
            ResourceLimits::new(10, 20, 30).unwrap(),
            CaptureMode::Tail,
        )
        .unwrap();
        assert!(decode_snapshot(&raw, &Memory::new(&config), &identity).is_ok());
    }
    #[test]
    fn snapshot_format_rejects_corruption_and_active_markers() {
        let (memory, identity) = fixture();
        let raw = encode_snapshot(&memory, &identity).unwrap();
        for offset in [8, 10, 13, 104, 128, 160] {
            let mut bad = raw.clone();
            bad[offset] ^= 2;
            assert!(decode_snapshot(&bad, &memory, &identity).is_err());
        }
        assert!(decode_snapshot(&raw[..raw.len() - 1], &memory, &identity).is_err());
        let mut bad = raw.clone();
        bad.push(0);
        assert!(decode_snapshot(&bad, &memory, &identity).is_err());
        bad = raw[..160].to_vec();
        bad[10] = 1;
        assert!(decode_snapshot(&bad, &memory, &identity)
            .unwrap_err()
            .contains("interrupted"));
        let mut invalid = identity.clone();
        invalid.image_sha256 = "z".repeat(64);
        assert!(encode_snapshot(&memory, &invalid).is_err());
    }
}
