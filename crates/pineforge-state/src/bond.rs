//! Length-framed, CRC32-checked container for an opaque BLE bond blob.
//!
//! The BLE stack owns the bond's byte layout; this module only frames those
//! bytes for flash storage so a corrupt record falls back to re-pairing.
//!
//! Because the payload's layout belongs to the BLE stack rather than to this
//! container, the record also carries a tag naming the layout whoever wrote it
//! used. Nothing here interprets the tag - the caller compares it - but without
//! it, a firmware update that changes the payload layout would read the old
//! bytes as a valid record and install keys that decode into nonsense. Failing
//! the comparison costs one re-pairing; failing to notice costs a bond the
//! phone believes in and the watch cannot honour.

use heapless::Vec;

use crate::crc32;

pub const BOND_RECORD_LEN: usize = 128;
pub const BOND_PAYLOAD_MAX: usize = 96;
/// Fixed width of the payload-layout tag, zero-padded.
pub const BOND_SCHEMA_LEN: usize = 16;

const BOND_MAGIC: [u8; 4] = *b"PFBD";
/// Container version. 1 predates the payload tag and stores the payload at
/// [`LEGACY_HEADER_LEN`]; 2 carries the tag and stores it at [`HEADER_LEN`].
const BOND_VERSION: u16 = 2;
const LEGACY_BOND_VERSION: u16 = 1;
const LEGACY_HEADER_LEN: usize = 8;
const SCHEMA_OFFSET: usize = 8;
const HEADER_LEN: usize = SCHEMA_OFFSET + BOND_SCHEMA_LEN;
const CRC_OFFSET: usize = BOND_RECORD_LEN - 4;

/// A bond record as it was stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BondRecord {
    /// The bytes the BLE stack serialized.
    pub payload: Vec<u8, BOND_PAYLOAD_MAX>,
    /// The payload layout the writing firmware used, or `None` for a record
    /// written before this container carried the tag.
    pub schema: Option<[u8; BOND_SCHEMA_LEN]>,
}

/// Pads a layout name into the fixed-width tag, or `None` when it is too long.
///
/// Both writing and comparing go through this, so a tag can never match by
/// accident of padding.
#[must_use]
pub fn bond_schema_tag(schema: &str) -> Option<[u8; BOND_SCHEMA_LEN]> {
    let bytes = schema.as_bytes();
    if bytes.is_empty() || bytes.len() > BOND_SCHEMA_LEN {
        return None;
    }
    let mut tag = [0_u8; BOND_SCHEMA_LEN];
    tag[..bytes.len()].copy_from_slice(bytes);
    Some(tag)
}

/// Frames a bond payload into a fixed-size, checksummed record.
#[must_use]
pub fn frame_bond(schema: [u8; BOND_SCHEMA_LEN], payload: &[u8]) -> Option<[u8; BOND_RECORD_LEN]> {
    if payload.len() > BOND_PAYLOAD_MAX {
        return None;
    }
    let mut record = [0_u8; BOND_RECORD_LEN];
    // The length fits a u16: it is bounded by BOND_PAYLOAD_MAX above.
    let len = u16::try_from(payload.len()).ok()?;
    record[0..4].copy_from_slice(&BOND_MAGIC);
    record[4..6].copy_from_slice(&BOND_VERSION.to_le_bytes());
    record[6..8].copy_from_slice(&len.to_le_bytes());
    record[SCHEMA_OFFSET..HEADER_LEN].copy_from_slice(&schema);
    record[HEADER_LEN..HEADER_LEN + payload.len()].copy_from_slice(payload);
    let crc = crc32(&record[..CRC_OFFSET]);
    record[CRC_OFFSET..].copy_from_slice(&crc.to_le_bytes());
    Some(record)
}

/// Recovers a stored record, or `None` if it is blank, corrupt, or a container
/// version this build does not understand.
///
/// An intact record is returned whatever its payload tag says; deciding whether
/// that tag is one this firmware can read is the caller's job.
#[must_use]
pub fn parse_bond(record: &[u8; BOND_RECORD_LEN]) -> Option<BondRecord> {
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

    let (header_len, schema) = match u16::from_le_bytes([record[4], record[5]]) {
        BOND_VERSION => (
            HEADER_LEN,
            Some(record[SCHEMA_OFFSET..HEADER_LEN].try_into().ok()?),
        ),
        LEGACY_BOND_VERSION => (LEGACY_HEADER_LEN, None),
        _ => return None,
    };
    let len = u16::from_le_bytes([record[6], record[7]]) as usize;
    if len > BOND_PAYLOAD_MAX {
        return None;
    }
    Some(BondRecord {
        payload: Vec::from_slice(&record[header_len..header_len + len]).ok()?,
        schema,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(schema: &str) -> [u8; BOND_SCHEMA_LEN] {
        bond_schema_tag(schema).unwrap()
    }

    /// Frames a record the way the firmware did before payload tagging, so the
    /// migration path is exercised against the real layout rather than a guess.
    fn frame_legacy(payload: &[u8]) -> [u8; BOND_RECORD_LEN] {
        let mut record = [0_u8; BOND_RECORD_LEN];
        record[0..4].copy_from_slice(&BOND_MAGIC);
        record[4..6].copy_from_slice(&LEGACY_BOND_VERSION.to_le_bytes());
        record[6..8].copy_from_slice(&(payload.len() as u16).to_le_bytes());
        record[LEGACY_HEADER_LEN..LEGACY_HEADER_LEN + payload.len()].copy_from_slice(payload);
        let crc = crc32(&record[..CRC_OFFSET]);
        record[CRC_OFFSET..].copy_from_slice(&crc.to_le_bytes());
        record
    }

    #[test]
    fn round_trips_a_payload_with_its_layout_tag() {
        let payload = [0xa5_u8; 43];
        let record = frame_bond(tag("0.7.0"), &payload).unwrap();
        let parsed = parse_bond(&record).unwrap();

        assert_eq!(parsed.payload, payload);
        assert_eq!(parsed.schema, Some(tag("0.7.0")));
    }

    #[test]
    fn a_record_written_by_another_layout_is_readable_but_names_that_layout() {
        // Reading it must not fail here - the caller compares the tag - but the
        // tag must survive intact, because that comparison is the whole point.
        let record = frame_bond(tag("0.8.0"), &[1, 2, 3, 4]).unwrap();
        let parsed = parse_bond(&record).unwrap();

        assert_eq!(parsed.schema, Some(tag("0.8.0")));
        assert_ne!(parsed.schema, Some(tag("0.7.0")));
    }

    #[test]
    fn an_untagged_record_still_parses_and_reports_no_layout() {
        let parsed = parse_bond(&frame_legacy(&[9; 16])).unwrap();

        assert_eq!(parsed.payload, [9; 16]);
        assert_eq!(parsed.schema, None);
    }

    #[test]
    fn rejects_blank_and_corrupt_records() {
        assert_eq!(parse_bond(&[0xFF; BOND_RECORD_LEN]), None);
        assert_eq!(parse_bond(&[0x00; BOND_RECORD_LEN]), None);

        let mut record = frame_bond(tag("0.7.0"), &[1, 2, 3, 4]).unwrap();
        record[HEADER_LEN] ^= 0x01;
        assert_eq!(parse_bond(&record), None);
    }

    #[test]
    fn a_tampered_tag_fails_the_checksum_rather_than_matching() {
        let mut record = frame_bond(tag("0.7.0"), &[1, 2, 3, 4]).unwrap();
        record[SCHEMA_OFFSET] ^= 0x01;
        assert_eq!(parse_bond(&record), None);
    }

    #[test]
    fn rejects_oversized_payloads_and_tags() {
        assert_eq!(frame_bond(tag("0.7.0"), &[0; BOND_PAYLOAD_MAX + 1]), None);
        assert_eq!(bond_schema_tag(""), None);
        assert_eq!(bond_schema_tag("0.7.0-with-a-very-long-suffix"), None);
        // The longest tag still has to leave room for a full payload.
        assert!(HEADER_LEN + BOND_PAYLOAD_MAX <= CRC_OFFSET);
    }

    #[test]
    fn rejects_future_versions() {
        let mut record = frame_bond(tag("0.7.0"), &[7; 8]).unwrap();
        record[4..6].copy_from_slice(&(BOND_VERSION + 1).to_le_bytes());
        let crc = crc32(&record[..CRC_OFFSET]);
        record[CRC_OFFSET..].copy_from_slice(&crc.to_le_bytes());
        assert_eq!(parse_bond(&record), None);
    }
}
