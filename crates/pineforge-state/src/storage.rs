//! Power-loss-safe metadata for `PineForge`'s external-flash storage region.

pub const STORAGE_BASE: u32 = 0x000B_4000;
pub const STORAGE_END: u32 = 0x003F_D000;
pub const STORAGE_SECTOR_SIZE: u32 = 4_096;
pub const STORAGE_SECTOR_COUNT: usize =
    ((STORAGE_END - STORAGE_BASE) / STORAGE_SECTOR_SIZE) as usize;
pub const STORAGE_DATA_SECTOR_COUNT: usize = STORAGE_SECTOR_COUNT - 1;

pub const STORAGE_FORMAT_VERSION: u16 = 1;
pub const STORAGE_HEADER_LEN: usize = 32;
pub const STORAGE_READY_HEADER_OFFSET: u32 = 32;
pub const STORAGE_PROGRESS_OFFSET: u32 = 64;

const MAGIC: [u8; 4] = *b"PFSG";
const FORMATTING_STATE: u8 = 0x7f;
const READY_STATE: u8 = 0x3f;
const CRC_OFFSET: usize = 28;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageHeader {
    Formatting,
    Ready,
}

#[must_use]
pub fn encode_storage_header(state: StorageHeader) -> [u8; STORAGE_HEADER_LEN] {
    let mut record = [0xff; STORAGE_HEADER_LEN];
    record[0..4].copy_from_slice(&MAGIC);
    record[4..6].copy_from_slice(&STORAGE_FORMAT_VERSION.to_le_bytes());
    record[6] = match state {
        StorageHeader::Formatting => FORMATTING_STATE,
        StorageHeader::Ready => READY_STATE,
    };
    record[8..12].copy_from_slice(&STORAGE_BASE.to_le_bytes());
    record[12..16].copy_from_slice(&STORAGE_END.to_le_bytes());
    let sector_count = u32::try_from(STORAGE_SECTOR_COUNT).unwrap_or(u32::MAX);
    record[16..20].copy_from_slice(&sector_count.to_le_bytes());
    let crc = crc32(&record[..CRC_OFFSET]);
    record[CRC_OFFSET..].copy_from_slice(&crc.to_le_bytes());
    record
}

#[must_use]
pub fn decode_storage_header(record: &[u8; STORAGE_HEADER_LEN]) -> Option<StorageHeader> {
    if record[0..4] != MAGIC
        || u16::from_le_bytes([record[4], record[5]]) != STORAGE_FORMAT_VERSION
        || u32::from_le_bytes(record[8..12].try_into().ok()?) != STORAGE_BASE
        || u32::from_le_bytes(record[12..16].try_into().ok()?) != STORAGE_END
        || u32::from_le_bytes(record[16..20].try_into().ok()?)
            != u32::try_from(STORAGE_SECTOR_COUNT).ok()?
        || u32::from_le_bytes(record[CRC_OFFSET..].try_into().ok()?) != crc32(&record[..CRC_OFFSET])
    {
        return None;
    }
    match record[6] {
        FORMATTING_STATE => Some(StorageHeader::Formatting),
        READY_STATE => Some(StorageHeader::Ready),
        _ => None,
    }
}

/// Returns the declared version when the record carries `PineForge`'s storage
/// magic, even if the rest of the record or its CRC is invalid.
#[must_use]
pub fn storage_header_version(record: &[u8; STORAGE_HEADER_LEN]) -> Option<u16> {
    (record[0..4] == MAGIC).then(|| u16::from_le_bytes([record[4], record[5]]))
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320_u32 & (0_u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_matches_documented_region() {
        assert_eq!(STORAGE_SECTOR_COUNT, 841);
        assert_eq!(STORAGE_END - STORAGE_BASE, 3_444_736);
        assert!(STORAGE_PROGRESS_OFFSET as usize + STORAGE_DATA_SECTOR_COUNT <= 4_096);
    }

    #[test]
    fn headers_round_trip_and_reject_corruption() {
        for state in [StorageHeader::Formatting, StorageHeader::Ready] {
            let mut encoded = encode_storage_header(state);
            assert_eq!(decode_storage_header(&encoded), Some(state));
            encoded[12] ^= 1;
            assert_eq!(decode_storage_header(&encoded), None);
        }
    }
}
