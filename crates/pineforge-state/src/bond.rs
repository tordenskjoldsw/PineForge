//! Length-framed, CRC32-checked container for an opaque BLE bond blob.
//!
//! The BLE stack owns the bond's byte layout; this module only frames those
//! bytes for flash storage so a corrupt record falls back to re-pairing.

use heapless::Vec;

use crate::crc32;

pub const BOND_RECORD_LEN: usize = 128;
pub const BOND_PAYLOAD_MAX: usize = 96;

const BOND_MAGIC: [u8; 4] = *b"PFBD";
const BOND_VERSION: u16 = 1;
const HEADER_LEN: usize = 8;
const CRC_OFFSET: usize = BOND_RECORD_LEN - 4;

/// Frames a bond payload into a fixed-size, checksummed record.
#[must_use]
pub fn frame_bond(payload: &[u8]) -> Option<[u8; BOND_RECORD_LEN]> {
    if payload.len() > BOND_PAYLOAD_MAX {
        return None;
    }
    let mut record = [0_u8; BOND_RECORD_LEN];
    // The length fits a u16: it is bounded by BOND_PAYLOAD_MAX above.
    let len = u16::try_from(payload.len()).ok()?;
    record[0..4].copy_from_slice(&BOND_MAGIC);
    record[4..6].copy_from_slice(&BOND_VERSION.to_le_bytes());
    record[6..8].copy_from_slice(&len.to_le_bytes());
    record[HEADER_LEN..HEADER_LEN + payload.len()].copy_from_slice(payload);
    let crc = crc32(&record[..CRC_OFFSET]);
    record[CRC_OFFSET..].copy_from_slice(&crc.to_le_bytes());
    Some(record)
}

/// Recovers the bond payload from a stored record, or `None` if it is blank,
/// corrupt, or a version this build does not understand.
#[must_use]
pub fn parse_bond(record: &[u8; BOND_RECORD_LEN]) -> Option<Vec<u8, BOND_PAYLOAD_MAX>> {
    if record[0..4] != BOND_MAGIC {
        return None;
    }
    let stored_crc = u32::from_le_bytes([
        record[CRC_OFFSET],
        record[CRC_OFFSET + 1],
        record[CRC_OFFSET + 2],
        record[CRC_OFFSET + 3],
    ]);
    if crc32(&record[..CRC_OFFSET]) != stored_crc {
        return None;
    }
    if u16::from_le_bytes([record[4], record[5]]) != BOND_VERSION {
        return None;
    }
    let len = u16::from_le_bytes([record[6], record[7]]) as usize;
    if len > BOND_PAYLOAD_MAX {
        return None;
    }
    Vec::from_slice(&record[HEADER_LEN..HEADER_LEN + len]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_payload() {
        let payload = [0xa5_u8; 43];
        let record = frame_bond(&payload).unwrap();
        assert_eq!(parse_bond(&record).unwrap(), payload);
    }

    #[test]
    fn rejects_blank_and_corrupt_records() {
        assert_eq!(parse_bond(&[0xFF; BOND_RECORD_LEN]), None);
        assert_eq!(parse_bond(&[0x00; BOND_RECORD_LEN]), None);

        let mut record = frame_bond(&[1, 2, 3, 4]).unwrap();
        record[HEADER_LEN] ^= 0x01;
        assert_eq!(parse_bond(&record), None);
    }

    #[test]
    fn rejects_oversized_payloads() {
        assert_eq!(frame_bond(&[0; BOND_PAYLOAD_MAX + 1]), None);
    }

    #[test]
    fn rejects_future_versions() {
        let mut record = frame_bond(&[7; 8]).unwrap();
        record[4..6].copy_from_slice(&2_u16.to_le_bytes());
        let crc = crc32(&record[..CRC_OFFSET]);
        record[CRC_OFFSET..].copy_from_slice(&crc.to_le_bytes());
        assert_eq!(parse_bond(&record), None);
    }
}
