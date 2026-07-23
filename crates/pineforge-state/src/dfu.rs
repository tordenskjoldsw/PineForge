//! Nordic legacy DFU protocol engine, driven by Gadgetbridge.
//!
//! This is pure protocol logic: it consumes control-point and packet writes
//! and emits flash operations and control-point notifications. The BLE task
//! executes the steps and the storage service performs the flash I/O, adding
//! the secondary-slot base to the image-relative offsets used here.
//!
//! The firmware CRC is accumulated over the received bytes as they arrive
//! rather than read back from flash; the flash driver verifies each program,
//! so a matching CRC means the correct image was both received and stored.

use heapless::Vec;

/// `MCUBoot` secondary-slot size; also the `imgtool` slot size in
/// `build-dfu.sh`.
pub const DFU_SLOT_SIZE: u32 = 475_136;

const PAGE_SIZE: usize = 256;
const SECTOR_SIZE: u32 = 4_096;

/// `MCUBoot` image-trailer magic, written once the image is staged.
const MAGIC: [u32; 4] = [0xf395_c277, 0x7fef_d260, 0x0f50_5235, 0x8079_b62c];
const MAGIC_LEN: u32 = 16;
const MAGIC_OFFSET: u32 = DFU_SLOT_SIZE - MAGIC_LEN;

// Control-point opcodes.
const START_DFU: u8 = 0x01;
const INIT_PARAMETERS: u8 = 0x02;
const RECEIVE_IMAGE: u8 = 0x03;
const VALIDATE: u8 = 0x04;
const ACTIVATE_RESET: u8 = 0x05;
const PACKET_RECEIPT_REQUEST: u8 = 0x08;
const RESPONSE: u8 = 0x10;
const PACKET_RECEIPT_NOTIFICATION: u8 = 0x11;

// Image types and status codes.
const IMAGE_APPLICATION: u8 = 0x04;
const STATUS_SUCCESS: u8 = 0x01;
const STATUS_NOT_SUPPORTED: u8 = 0x03;
const STATUS_SIZE_EXCEEDED: u8 = 0x04;
const STATUS_CRC_ERROR: u8 = 0x05;

/// One action for the BLE task to execute in order. Offsets are relative to
/// the secondary slot; no `Program` ever crosses a 256-byte page boundary.
// `Program` carries an inline page because the target has no heap to box it.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DfuStep {
    Notify(Vec<u8, 5>),
    Erase(u32),
    Program {
        offset: u32,
        data: Vec<u8, PAGE_SIZE>,
    },
    Reset,
}

type Steps = Vec<DfuStep, 6>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Idle,
    Start,
    Init,
    Data,
    Validate,
    Validated,
}

/// Nordic legacy DFU state machine.
pub struct DfuEngine {
    validated: bool,
    state: State,
    application_size: u32,
    expected_crc: u16,
    crc: u16,
    bytes_received: u32,
    write_offset: u32,
    last_erased_end: u32,
    page: Vec<u8, PAGE_SIZE>,
    packets_to_notify: u8,
    packets_received: u32,
}

impl DfuEngine {
    /// `validated` gates `StartDFU`: an unconfirmed image must refuse updates
    /// so it never erases the secondary slot that still holds the rollback
    /// image.
    #[must_use]
    pub const fn new(validated: bool) -> Self {
        Self {
            validated,
            state: State::Idle,
            application_size: 0,
            expected_crc: 0,
            crc: 0xFFFF,
            bytes_received: 0,
            write_offset: 0,
            last_erased_end: 0,
            page: Vec::new(),
            packets_to_notify: 0,
            packets_received: 0,
        }
    }

    fn reset(&mut self) {
        let validated = self.validated;
        *self = Self::new(validated);
    }

    /// Handles a write to the control-point characteristic.
    pub fn control_write(&mut self, data: &[u8]) -> Steps {
        let mut steps = Steps::new();
        let Some(&opcode) = data.first() else {
            return steps;
        };
        match opcode {
            START_DFU => {
                if !self.validated {
                    let _ = steps.push(response(START_DFU, STATUS_NOT_SUPPORTED));
                } else if self.state == State::Idle && data.get(1) == Some(&IMAGE_APPLICATION) {
                    self.state = State::Start;
                }
            }
            INIT_PARAMETERS => {
                let complete = data.get(1).copied().unwrap_or(0) != 0;
                if self.state == State::Init && complete {
                    let _ = steps.push(response(INIT_PARAMETERS, STATUS_SUCCESS));
                }
            }
            PACKET_RECEIPT_REQUEST => {
                self.packets_to_notify = data.get(1).copied().unwrap_or(0);
            }
            RECEIVE_IMAGE => {
                if self.state == State::Init {
                    self.state = State::Data;
                }
            }
            VALIDATE => {
                if self.state == State::Validate {
                    if self.crc == self.expected_crc {
                        self.state = State::Validated;
                        let _ = steps.push(response(VALIDATE, STATUS_SUCCESS));
                    } else {
                        let _ = steps.push(response(VALIDATE, STATUS_CRC_ERROR));
                        self.reset();
                    }
                }
            }
            ACTIVATE_RESET if self.state == State::Validated => {
                self.stage_magic(&mut steps);
                let _ = steps.push(DfuStep::Reset);
            }
            _ => {}
        }
        steps
    }

    /// Handles a write to the packet characteristic.
    pub fn packet_write(&mut self, data: &[u8]) -> Steps {
        let mut steps = Steps::new();
        match self.state {
            State::Start => self.receive_sizes(data, &mut steps),
            State::Init => self.receive_init_packet(data),
            State::Data => self.receive_firmware(data, &mut steps),
            _ => {}
        }
        steps
    }

    fn receive_sizes(&mut self, data: &[u8], steps: &mut Steps) {
        if data.len() < 12 {
            return;
        }
        self.application_size = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        if self.application_size == 0 || self.application_size > MAGIC_OFFSET {
            let _ = steps.push(response(START_DFU, STATUS_SIZE_EXCEEDED));
            self.reset();
            return;
        }
        self.state = State::Init;
        let _ = steps.push(response(START_DFU, STATUS_SUCCESS));
    }

    fn receive_init_packet(&mut self, data: &[u8]) {
        // Layout: device type, revision, app version, softdevice array, then
        // the firmware CRC16 as the final two bytes.
        let Some(sd_len) = data.get(8..10) else {
            return;
        };
        let sd_len = u16::from_le_bytes([sd_len[0], sd_len[1]]) as usize;
        let crc_offset = 10 + sd_len * 2;
        if let Some(bytes) = data.get(crc_offset..crc_offset + 2) {
            self.expected_crc = u16::from_le_bytes([bytes[0], bytes[1]]);
        }
    }

    fn receive_firmware(&mut self, data: &[u8], steps: &mut Steps) {
        self.packets_received += 1;
        for &byte in data {
            if self.bytes_received >= self.application_size {
                break;
            }
            self.crc = crc16_update(self.crc, byte);
            let _ = self.page.push(byte);
            self.bytes_received += 1;
            if self.page.len() == PAGE_SIZE {
                self.flush_page(steps);
            }
        }

        if self.bytes_received == self.application_size {
            if !self.page.is_empty() {
                self.flush_page(steps);
            }
            let _ = steps.push(response(RECEIVE_IMAGE, STATUS_SUCCESS));
            self.state = State::Validate;
        } else if self.packets_to_notify != 0
            && self
                .packets_received
                .is_multiple_of(u32::from(self.packets_to_notify))
        {
            let mut notification = Vec::new();
            let _ = notification.push(PACKET_RECEIPT_NOTIFICATION);
            let _ = notification.extend_from_slice(&self.bytes_received.to_le_bytes());
            let _ = steps.push(DfuStep::Notify(notification));
        }
    }

    /// Emits an erase (once per sector) and a program for the buffered page.
    fn flush_page(&mut self, steps: &mut Steps) {
        self.ensure_erased(self.write_offset, steps);
        let mut data = Vec::new();
        let _ = data.extend_from_slice(&self.page);
        // The page length never exceeds PAGE_SIZE, so the cast cannot truncate.
        let len = u32::try_from(self.page.len()).unwrap_or(0);
        let _ = steps.push(DfuStep::Program {
            offset: self.write_offset,
            data,
        });
        self.write_offset += len;
        self.page.clear();
    }

    /// Stages the `MCUBoot` magic in the slot's trailer, erasing its sector
    /// first only if the image did not already reach it.
    fn stage_magic(&mut self, steps: &mut Steps) {
        self.ensure_erased(MAGIC_OFFSET, steps);
        let mut data = Vec::new();
        for word in MAGIC {
            let _ = data.extend_from_slice(&word.to_le_bytes());
        }
        let _ = steps.push(DfuStep::Program {
            offset: MAGIC_OFFSET,
            data,
        });
    }

    fn ensure_erased(&mut self, offset: u32, steps: &mut Steps) {
        if offset < self.last_erased_end {
            return;
        }
        let sector = (offset / SECTOR_SIZE) * SECTOR_SIZE;
        let _ = steps.push(DfuStep::Erase(sector));
        self.last_erased_end = sector + SECTOR_SIZE;
    }
}

fn response(request: u8, status: u8) -> DfuStep {
    let mut bytes = Vec::new();
    let _ = bytes.push(RESPONSE);
    let _ = bytes.push(request);
    let _ = bytes.push(status);
    DfuStep::Notify(bytes)
}

/// nRF SDK `crc16_compute`, seeded with `0xFFFF` (CRC-16/CCITT-FALSE).
#[must_use]
pub const fn crc16_update(crc: u16, byte: u8) -> u16 {
    let mut crc = crc.rotate_left(8);
    crc ^= byte as u16;
    crc ^= (crc & 0x00FF) >> 4;
    crc ^= (crc << 8) << 4;
    crc ^= ((crc & 0x00FF) << 4) << 1;
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crc16(data: &[u8]) -> u16 {
        data.iter().fold(0xFFFF, |crc, &b| crc16_update(crc, b))
    }

    fn notify(bytes: &[u8]) -> DfuStep {
        DfuStep::Notify(Vec::from_slice(bytes).unwrap())
    }

    #[test]
    fn crc16_matches_the_ccitt_false_vector() {
        assert_eq!(crc16(b"123456789"), 0x29B1);
    }

    #[test]
    fn unvalidated_engine_refuses_to_start() {
        let mut engine = DfuEngine::new(false);
        let steps = engine.control_write(&[START_DFU, IMAGE_APPLICATION]);
        assert_eq!(
            steps[0],
            notify(&[RESPONSE, START_DFU, STATUS_NOT_SUPPORTED])
        );
        // No flash touched, and a following packet is ignored.
        assert!(engine.packet_write(&[0; 12]).is_empty());
    }

    #[test]
    fn rejects_an_image_larger_than_the_slot() {
        let mut engine = DfuEngine::new(true);
        assert!(
            engine
                .control_write(&[START_DFU, IMAGE_APPLICATION])
                .is_empty()
        );
        let mut sizes = [0_u8; 12];
        sizes[8..12].copy_from_slice(&(DFU_SLOT_SIZE).to_le_bytes());
        let steps = engine.packet_write(&sizes);
        assert_eq!(
            steps[0],
            notify(&[RESPONSE, START_DFU, STATUS_SIZE_EXCEEDED])
        );
    }

    fn init_packet(crc: u16) -> Vec<u8, 32> {
        // Zero softdevices, so the CRC follows the 10-byte header directly.
        let mut packet = Vec::new();
        let _ = packet.extend_from_slice(&[0; 10]);
        let _ = packet.extend_from_slice(&crc.to_le_bytes());
        packet
    }

    #[test]
    fn full_transfer_validates_and_activates() {
        // 300 bytes crosses one page boundary and leaves a partial page.
        let image: Vec<u8, 300> = (0..300).map(|i| (i * 7) as u8).collect();
        let expected = crc16(&image);

        let mut engine = DfuEngine::new(true);
        assert!(
            engine
                .control_write(&[START_DFU, IMAGE_APPLICATION])
                .is_empty()
        );

        let mut sizes = [0_u8; 12];
        sizes[8..12].copy_from_slice(&(image.len() as u32).to_le_bytes());
        assert_eq!(
            engine.packet_write(&sizes)[0],
            notify(&[RESPONSE, START_DFU, STATUS_SUCCESS])
        );

        engine.packet_write(&init_packet(expected));
        assert_eq!(
            engine.control_write(&[INIT_PARAMETERS, 1])[0],
            notify(&[RESPONSE, INIT_PARAMETERS, STATUS_SUCCESS])
        );
        engine.control_write(&[PACKET_RECEIPT_REQUEST, 4]);
        assert!(engine.control_write(&[RECEIVE_IMAGE]).is_empty());

        let mut programmed: Vec<u8, 512> = Vec::new();
        let mut erased: Vec<u32, 4> = Vec::new();
        let mut completed = false;
        for chunk in image.chunks(20) {
            for step in engine.packet_write(chunk) {
                match step {
                    DfuStep::Erase(sector) => erased.push(sector).unwrap(),
                    DfuStep::Program { offset, data } => {
                        assert_eq!(offset as usize, programmed.len());
                        programmed.extend_from_slice(&data).unwrap();
                    }
                    DfuStep::Notify(bytes) if bytes[0] == RESPONSE => {
                        assert_eq!(&bytes[..], &[RESPONSE, RECEIVE_IMAGE, STATUS_SUCCESS]);
                        completed = true;
                    }
                    DfuStep::Notify(_) | DfuStep::Reset => {}
                }
            }
        }

        assert!(completed);
        assert_eq!(&programmed[..], &image[..]);
        assert_eq!(&erased[..], &[0]); // 300 bytes fit in the first sector

        assert_eq!(
            engine.control_write(&[VALIDATE])[0],
            notify(&[RESPONSE, VALIDATE, STATUS_SUCCESS])
        );

        let activate = engine.control_write(&[ACTIVATE_RESET]);
        assert!(matches!(activate.last(), Some(DfuStep::Reset)));
        // The magic sector is erased and programmed before reset.
        assert!(activate.iter().any(
            |step| matches!(step, DfuStep::Program { offset, .. } if *offset == MAGIC_OFFSET)
        ));
    }

    #[test]
    fn a_bad_crc_reports_an_error_and_resets() {
        let image = [1_u8, 2, 3, 4];
        let mut engine = DfuEngine::new(true);
        engine.control_write(&[START_DFU, IMAGE_APPLICATION]);
        let mut sizes = [0_u8; 12];
        sizes[8..12].copy_from_slice(&(image.len() as u32).to_le_bytes());
        engine.packet_write(&sizes);
        engine.packet_write(&init_packet(0x0000)); // wrong CRC
        engine.control_write(&[RECEIVE_IMAGE]);
        engine.packet_write(&image);

        let steps = engine.control_write(&[VALIDATE]);
        assert_eq!(steps[0], notify(&[RESPONSE, VALIDATE, STATUS_CRC_ERROR]));
        // A fresh transfer can start again after the reset.
        assert!(
            engine
                .control_write(&[START_DFU, IMAGE_APPLICATION])
                .is_empty()
        );
    }

    #[test]
    fn packet_receipt_notifications_report_progress() {
        let image: Vec<u8, 120> = (0..120).map(|i| i as u8).collect();
        let mut engine = DfuEngine::new(true);
        engine.control_write(&[START_DFU, IMAGE_APPLICATION]);
        let mut sizes = [0_u8; 12];
        sizes[8..12].copy_from_slice(&120_u32.to_le_bytes());
        engine.packet_write(&sizes);
        engine.packet_write(&init_packet(crc16(&image)));
        engine.control_write(&[INIT_PARAMETERS, 1]);
        engine.control_write(&[PACKET_RECEIPT_REQUEST, 2]);
        engine.control_write(&[RECEIVE_IMAGE]);

        let mut receipts = 0;
        for chunk in image.chunks(20) {
            for step in engine.packet_write(chunk) {
                if let DfuStep::Notify(bytes) = step {
                    if bytes[0] == PACKET_RECEIPT_NOTIFICATION {
                        receipts += 1;
                    }
                }
            }
        }
        // 6 packets, notify every 2, last packet sends completion instead.
        assert_eq!(receipts, 2);
    }
}
