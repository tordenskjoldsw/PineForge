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
//!
//! Sectors are erased lazily, just ahead of the write offset, rather than
//! erasing the whole slot up front: on this BLE stack a long unbroken run of
//! flash operations inside the GATT event handler stalls the connection
//! badly enough that the transfer never completes, so the erase cost stays
//! spread across the data phase instead.
//!
//! A session that is abandoned must never outlive itself. The host may retry
//! on the same connection, and there are three ways back to idle for it to
//! land on: a fresh `StartDFU`, the `Reset System` opcode the host sends when
//! it aborts, and [`DfuEngine::abort`] for the BLE task's stall timeout. An
//! engine that answered none of them could only be cleared by reconnecting.

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
const RESET_SYSTEM: u8 = 0x06;
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
    /// A protocol acknowledgement the host acts on (start/init/receive/validate
    /// success or failure). The BLE task makes queued flash durable before
    /// sending it, so the host never advances past an ack the flash is behind.
    Notify(Vec<u8, 5>),
    /// A packet-receipt notification: pure flow control that releases the next
    /// batch. The BLE task sends it without draining in-flight flash, so the
    /// radio keeps receiving while the pipeline catches up.
    Receipt(Vec<u8, 5>),
    Erase(u32),
    Program {
        offset: u32,
        data: Vec<u8, PAGE_SIZE>,
    },
    Reset,
    /// An update was refused because the running image is not confirmed.
    ///
    /// The host is told over the wire by the `Notify` beside this, but a
    /// refusal nobody can see is indistinguishable from a broken link - so the
    /// watch says so on its own screen too. This is the step that asks it to.
    Refused,
}

// One packet can at most cross one page boundary. The largest sequence is a
// single erase-ahead, the completed page, a final partial page, and its
// completion notification. The one exception with the same bound is the very
// first page: it primes the frontier with two erases (its own sector and the
// one ahead) plus the page and a possible receipt. Keeping this exact bound
// matters because every inline `DfuStep` can carry a 256-byte page inside the
// BLE task's static future.
type Steps = Vec<DfuStep, 4>;

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
///
/// Deliberately holds no opinion about whether updates are allowed. That
/// depends on whether the running image has been confirmed, which can change
/// while a phone is connected - and an engine that latched it at construction
/// went on refusing for the life of the connection after the user had already
/// confirmed. Asking the caller at each control write makes the stale answer
/// impossible to hold rather than merely wrong.
pub struct DfuEngine {
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

impl Default for DfuEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl DfuEngine {
    #[must_use]
    pub const fn new() -> Self {
        Self {
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
        *self = Self::new();
    }

    /// Abandons a transfer in progress and returns the engine to idle.
    ///
    /// Deliberately does not reboot: the host sends `Reset System` on *error*
    /// as well as on a user abort, so rebooting here would turn every failed
    /// upload into a restart. Nothing was activated, so dropping the partial
    /// image in the secondary slot is the whole of the cleanup.
    pub fn abort(&mut self) {
        self.reset();
    }

    /// Whether a transfer is in flight, so the caller can hold it to a
    /// deadline. Idle sessions must not be timed out - the connection is
    /// allowed to sit there for hours without an update.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        !matches!(self.state, State::Idle)
    }

    /// Transfer progress as a percentage while the image is being received,
    /// for the on-watch update screen. `None` outside the transfer.
    #[must_use]
    pub fn progress_percent(&self) -> Option<u8> {
        if self.application_size == 0 {
            return None;
        }
        match self.state {
            State::Data | State::Validate | State::Validated => {
                let done = u64::from(self.bytes_received) * 100 / u64::from(self.application_size);
                Some(u8::try_from(done).unwrap_or(100))
            }
            _ => None,
        }
    }

    /// Handles a write to the control-point characteristic.
    ///
    /// `confirmed` is read fresh at every call rather than remembered, and
    /// gates `StartDFU`: an unconfirmed image must refuse updates so it never
    /// erases the secondary slot that still holds the rollback image. Confirming
    /// therefore takes effect on the next attempt, on the connection already
    /// open, instead of after a reconnect nobody knew was needed.
    pub fn control_write(&mut self, data: &[u8], confirmed: bool) -> Steps {
        let mut steps = Steps::new();
        let Some(&opcode) = data.first() else {
            return steps;
        };
        match opcode {
            START_DFU => {
                if !confirmed {
                    let _ = steps.push(response(START_DFU, STATUS_NOT_SUPPORTED));
                    let _ = steps.push(DfuStep::Refused);
                } else if data.get(1) == Some(&IMAGE_APPLICATION) {
                    // A new transfer supersedes whatever is left of the last
                    // one. Ignoring this while a stale session is open leaves
                    // the host waiting for a response that never comes, and
                    // only a reconnect - which rebuilds the engine - clears it;
                    // that is exactly the "upload fails until I disconnect"
                    // symptom. Nothing of the old session is worth keeping:
                    // the sizes and CRC that follow describe the new image.
                    self.reset();
                    self.state = State::Start;
                } else {
                    // Softdevice and bootloader images have nowhere to go on
                    // this target. Saying so is what keeps the host from
                    // waiting out its own timeout.
                    let _ = steps.push(response(START_DFU, STATUS_NOT_SUPPORTED));
                }
            }
            // Sent by the host on an aborted or failed transfer. The protocol
            // expects no reply, only that the target forgets the session, so
            // the next attempt starts clean on the same connection.
            RESET_SYSTEM => self.reset(),
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
            let _ = steps.push(DfuStep::Receipt(notification));
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

    /// Keeps the erase frontier one sector *ahead* of the write offset, capped
    /// at the slot end. The sector being programmed is therefore already blank,
    /// and the sector after it is erased in the background while the current one
    /// fills — so the ~tens-of-milliseconds sector erase overlaps reception
    /// instead of stalling in the FIFO in front of the program that needs it.
    fn ensure_erased(&mut self, offset: u32, steps: &mut Steps) {
        let current = (offset / SECTOR_SIZE) * SECTOR_SIZE;
        // Cover `offset`'s own sector and the one after it, capped at the slot
        // end so no erase base ever lands at or beyond it. Starting at
        // `max(frontier, current)` skips any gap left by a far jump — the
        // trailer magic leaps to the slot end, and the sectors it skips are
        // never programmed, so they must not be erased. That bounds this to at
        // most two erases per call (only the first page and a far jump reach
        // two; every steady-state crossing emits one).
        let target_end = (current + 2 * SECTOR_SIZE).min(DFU_SLOT_SIZE);
        let mut base = self.last_erased_end.max(current);
        while base < target_end {
            let _ = steps.push(DfuStep::Erase(base));
            base += SECTOR_SIZE;
        }
        self.last_erased_end = self.last_erased_end.max(base);
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
    fn an_unconfirmed_image_refuses_to_start() {
        let mut engine = DfuEngine::new();
        let steps = engine.control_write(&[START_DFU, IMAGE_APPLICATION], false);
        assert_eq!(
            steps[0],
            notify(&[RESPONSE, START_DFU, STATUS_NOT_SUPPORTED])
        );
        // The phone is told over the wire; the watch is told separately, or the
        // refusal is invisible to the person holding both.
        assert!(steps.contains(&DfuStep::Refused));
        // No flash touched, and a following packet is ignored.
        assert!(engine.packet_write(&[0; 12]).is_empty());
    }

    #[test]
    fn rejects_an_image_larger_than_the_slot() {
        let mut engine = DfuEngine::new();
        assert!(
            engine
                .control_write(&[START_DFU, IMAGE_APPLICATION], true)
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

        let mut engine = DfuEngine::new();
        assert!(
            engine
                .control_write(&[START_DFU, IMAGE_APPLICATION], true)
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
            engine.control_write(&[INIT_PARAMETERS, 1], true)[0],
            notify(&[RESPONSE, INIT_PARAMETERS, STATUS_SUCCESS])
        );
        engine.control_write(&[PACKET_RECEIPT_REQUEST, 4], true);
        assert!(engine.control_write(&[RECEIVE_IMAGE], true).is_empty());

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
                    DfuStep::Notify(_)
                    | DfuStep::Receipt(_)
                    | DfuStep::Reset
                    | DfuStep::Refused => {}
                }
            }
        }

        assert!(completed);
        assert_eq!(&programmed[..], &image[..]);
        // 300 bytes fit in the first sector; the erase frontier still runs one
        // sector ahead, so sector 1 is pre-erased even though it is unused.
        assert_eq!(&erased[..], &[0, SECTOR_SIZE]);

        assert_eq!(
            engine.control_write(&[VALIDATE], true)[0],
            notify(&[RESPONSE, VALIDATE, STATUS_SUCCESS])
        );

        let activate = engine.control_write(&[ACTIVATE_RESET], true);
        assert!(matches!(activate.last(), Some(DfuStep::Reset)));
        // The magic sector is erased and programmed before reset.
        assert!(activate.iter().any(
            |step| matches!(step, DfuStep::Program { offset, .. } if *offset == MAGIC_OFFSET)
        ));
    }

    #[test]
    fn activation_erases_only_the_trailer_sector_not_the_gap() {
        // A small image leaves the frontier far below the trailer. Staging the
        // magic must erase just the trailer's own sector, never the unwritten
        // sectors in between, and never a base at or beyond the slot end.
        let image = [7_u8, 7, 7, 7];
        let mut engine = DfuEngine::new();
        engine.control_write(&[START_DFU, IMAGE_APPLICATION], true);
        let mut sizes = [0_u8; 12];
        sizes[8..12].copy_from_slice(&(image.len() as u32).to_le_bytes());
        engine.packet_write(&sizes);
        engine.packet_write(&init_packet(crc16(&image)));
        engine.control_write(&[INIT_PARAMETERS, 1], true);
        engine.control_write(&[RECEIVE_IMAGE], true);
        engine.packet_write(&image);
        engine.control_write(&[VALIDATE], true);

        let activate = engine.control_write(&[ACTIVATE_RESET], true);
        let erases: Vec<u32, 4> = activate
            .iter()
            .filter_map(|step| match step {
                DfuStep::Erase(base) => Some(*base),
                _ => None,
            })
            .collect();
        let last_sector = (MAGIC_OFFSET / SECTOR_SIZE) * SECTOR_SIZE;
        assert_eq!(&erases[..], &[last_sector]);
        assert!(erases.iter().all(|&base| base < DFU_SLOT_SIZE));
        assert!(matches!(activate.last(), Some(DfuStep::Reset)));
    }

    #[test]
    fn a_bad_crc_reports_an_error_and_resets() {
        let image = [1_u8, 2, 3, 4];
        let mut engine = DfuEngine::new();
        engine.control_write(&[START_DFU, IMAGE_APPLICATION], true);
        let mut sizes = [0_u8; 12];
        sizes[8..12].copy_from_slice(&(image.len() as u32).to_le_bytes());
        engine.packet_write(&sizes);
        engine.packet_write(&init_packet(0x0000)); // wrong CRC
        engine.control_write(&[RECEIVE_IMAGE], true);
        engine.packet_write(&image);

        let steps = engine.control_write(&[VALIDATE], true);
        assert_eq!(steps[0], notify(&[RESPONSE, VALIDATE, STATUS_CRC_ERROR]));
        // A fresh transfer can start again after the reset.
        assert!(
            engine
                .control_write(&[START_DFU, IMAGE_APPLICATION], true)
                .is_empty()
        );
    }

    /// Drives a transfer up to the middle of the data phase and leaves it
    /// there, the way an upload that is cancelled or that loses its host does.
    fn engine_stalled_mid_transfer() -> DfuEngine {
        let mut engine = DfuEngine::new();
        engine.control_write(&[START_DFU, IMAGE_APPLICATION], true);
        let mut sizes = [0_u8; 12];
        sizes[8..12].copy_from_slice(&1_024_u32.to_le_bytes());
        engine.packet_write(&sizes);
        engine.packet_write(&init_packet(0x1234));
        engine.control_write(&[INIT_PARAMETERS, 1], true);
        engine.control_write(&[RECEIVE_IMAGE], true);
        engine.packet_write(&[0xAB; 300]);
        assert!(engine.is_active());
        engine
    }

    /// Asserts the engine will carry a fresh transfer all the way to a reset.
    fn assert_accepts_a_new_transfer(engine: &mut DfuEngine) {
        let image = [3_u8, 1, 4, 1, 5];
        assert!(
            engine
                .control_write(&[START_DFU, IMAGE_APPLICATION], true)
                .is_empty()
        );
        let mut sizes = [0_u8; 12];
        sizes[8..12].copy_from_slice(&(image.len() as u32).to_le_bytes());
        assert_eq!(
            engine.packet_write(&sizes)[0],
            notify(&[RESPONSE, START_DFU, STATUS_SUCCESS])
        );
        engine.packet_write(&init_packet(crc16(&image)));
        engine.control_write(&[INIT_PARAMETERS, 1], true);
        engine.control_write(&[RECEIVE_IMAGE], true);
        assert!(engine.packet_write(&image).iter().any(
            |step| matches!(step, DfuStep::Notify(bytes) if bytes.as_slice() == [RESPONSE, RECEIVE_IMAGE, STATUS_SUCCESS])
        ));
        assert_eq!(
            engine.control_write(&[VALIDATE], true)[0],
            notify(&[RESPONSE, VALIDATE, STATUS_SUCCESS])
        );
        assert!(matches!(
            engine.control_write(&[ACTIVATE_RESET], true).last(),
            Some(DfuStep::Reset)
        ));
    }

    #[test]
    fn a_new_start_supersedes_a_stalled_transfer() {
        // The host retrying on the same connection is the case that used to
        // hang: StartDFU was ignored outside idle, so no response ever came
        // and only a reconnect - which builds a new engine - recovered.
        let mut engine = engine_stalled_mid_transfer();
        assert_accepts_a_new_transfer(&mut engine);
    }

    #[test]
    fn reset_system_returns_a_stalled_transfer_to_idle() {
        let mut engine = engine_stalled_mid_transfer();
        // The protocol expects no reply, only that the session is forgotten.
        assert!(engine.control_write(&[RESET_SYSTEM], true).is_empty());
        assert!(!engine.is_active());
        assert_accepts_a_new_transfer(&mut engine);
    }

    #[test]
    fn abort_returns_a_stalled_transfer_to_idle() {
        let mut engine = engine_stalled_mid_transfer();
        engine.abort();
        assert!(!engine.is_active());
        assert_eq!(engine.progress_percent(), None);
        assert_accepts_a_new_transfer(&mut engine);
    }

    #[test]
    fn a_superseded_transfer_leaves_no_state_behind() {
        // Bytes, CRC and write offset from the abandoned attempt must not
        // bleed into the next one: a stale CRC would fail validation, and a
        // stale offset would write the new image into the middle of the slot.
        let mut engine = engine_stalled_mid_transfer();
        let image = [9_u8; 64];
        engine.control_write(&[START_DFU, IMAGE_APPLICATION], true);
        let mut sizes = [0_u8; 12];
        sizes[8..12].copy_from_slice(&(image.len() as u32).to_le_bytes());
        engine.packet_write(&sizes);
        engine.packet_write(&init_packet(crc16(&image)));
        engine.control_write(&[INIT_PARAMETERS, 1], true);
        engine.control_write(&[RECEIVE_IMAGE], true);

        let mut programmed: Vec<u8, 64> = Vec::new();
        for step in engine.packet_write(&image) {
            if let DfuStep::Program { offset, data } = step {
                assert_eq!(offset, 0);
                programmed.extend_from_slice(&data).unwrap();
            }
        }
        assert_eq!(&programmed[..], &image[..]);
        assert_eq!(
            engine.control_write(&[VALIDATE], true)[0],
            notify(&[RESPONSE, VALIDATE, STATUS_SUCCESS])
        );
    }

    #[test]
    fn an_unsupported_image_type_is_refused_rather_than_ignored() {
        // Softdevice and bootloader images have nowhere to go here. Silence
        // would leave the host waiting out its own timeout.
        let mut engine = DfuEngine::new();
        const IMAGE_SOFTDEVICE: u8 = 0x01;
        assert_eq!(
            engine.control_write(&[START_DFU, IMAGE_SOFTDEVICE], true)[0],
            notify(&[RESPONSE, START_DFU, STATUS_NOT_SUPPORTED])
        );
        assert!(!engine.is_active());
    }

    #[test]
    fn an_unconfirmed_image_still_refuses_a_repeated_start() {
        // The rollback image in the secondary slot outranks a retry.
        let mut engine = DfuEngine::new();
        for _ in 0..2 {
            assert_eq!(
                engine.control_write(&[START_DFU, IMAGE_APPLICATION], false)[0],
                notify(&[RESPONSE, START_DFU, STATUS_NOT_SUPPORTED])
            );
            assert!(!engine.is_active());
        }
    }

    /// The regression for "I confirmed it and it still will not flash".
    ///
    /// Confirmation happens on the watch while the phone sits connected, so the
    /// answer has to be re-read rather than remembered. An engine that latched
    /// it at construction went on refusing for the life of that connection -
    /// which is why disconnecting and reconnecting appeared to be the fix, and
    /// why nothing about the watch explained it.
    #[test]
    fn confirming_takes_effect_without_a_reconnect() {
        let mut engine = DfuEngine::new();
        assert_eq!(
            engine.control_write(&[START_DFU, IMAGE_APPLICATION], false)[0],
            notify(&[RESPONSE, START_DFU, STATUS_NOT_SUPPORTED])
        );

        // Same engine, same connection: the user has just confirmed.
        assert!(
            engine
                .control_write(&[START_DFU, IMAGE_APPLICATION], true)
                .is_empty()
        );
        assert!(engine.is_active());
        assert_accepts_a_new_transfer(&mut engine);
    }

    #[test]
    fn packet_receipt_notifications_report_progress() {
        let image: Vec<u8, 120> = (0..120).map(|i| i as u8).collect();
        let mut engine = DfuEngine::new();
        engine.control_write(&[START_DFU, IMAGE_APPLICATION], true);
        let mut sizes = [0_u8; 12];
        sizes[8..12].copy_from_slice(&120_u32.to_le_bytes());
        engine.packet_write(&sizes);
        engine.packet_write(&init_packet(crc16(&image)));
        engine.control_write(&[INIT_PARAMETERS, 1], true);
        engine.control_write(&[PACKET_RECEIPT_REQUEST, 2], true);
        engine.control_write(&[RECEIVE_IMAGE], true);

        let mut receipts = 0;
        for chunk in image.chunks(20) {
            for step in engine.packet_write(chunk) {
                if let DfuStep::Receipt(bytes) = step {
                    assert_eq!(bytes[0], PACKET_RECEIPT_NOTIFICATION);
                    receipts += 1;
                }
            }
        }
        // 6 packets, notify every 2, last packet sends completion instead.
        assert_eq!(receipts, 2);
    }

    #[test]
    fn negotiated_mtu_sized_packets_complete_the_transfer() {
        const ATT_WRITE_VALUE_MAX: usize = 248;
        let image: Vec<u8, 5000> = (0..5000).map(|i| (i * 13) as u8).collect();
        let mut engine = DfuEngine::new();
        engine.control_write(&[START_DFU, IMAGE_APPLICATION], true);
        let mut sizes = [0_u8; 12];
        sizes[8..12].copy_from_slice(&(image.len() as u32).to_le_bytes());
        engine.packet_write(&sizes);
        engine.packet_write(&init_packet(crc16(&image)));
        engine.control_write(&[INIT_PARAMETERS, 1], true);
        engine.control_write(&[RECEIVE_IMAGE], true);

        let mut completed = false;
        for chunk in image.chunks(ATT_WRITE_VALUE_MAX) {
            completed |= engine.packet_write(chunk).iter().any(
                |step| matches!(step, DfuStep::Notify(bytes) if bytes.as_slice() == [RESPONSE, RECEIVE_IMAGE, STATUS_SUCCESS]),
            );
        }

        assert!(completed);
        assert_eq!(
            engine.control_write(&[VALIDATE], true)[0],
            notify(&[RESPONSE, VALIDATE, STATUS_SUCCESS])
        );
    }

    #[test]
    fn final_packet_fits_the_exact_maximum_step_bound() {
        // Reach a fresh sector with 16 bytes already buffered. The final
        // 248-byte packet first completes and flushes that page (emitting the
        // erase-ahead for the *next* sector), then flushes its remaining eight
        // bytes and emits the completion notification: erase + program +
        // program + notify — the exact maximum step bound.
        const PREFIX_LEN: usize = SECTOR_SIZE as usize + 16;
        const FINAL_LEN: usize = 248;
        const IMAGE_LEN: usize = PREFIX_LEN + FINAL_LEN;
        let image: Vec<u8, IMAGE_LEN> = (0..IMAGE_LEN).map(|i| (i * 17) as u8).collect();

        let mut engine = DfuEngine::new();
        engine.control_write(&[START_DFU, IMAGE_APPLICATION], true);
        let mut sizes = [0_u8; 12];
        sizes[8..12].copy_from_slice(&(IMAGE_LEN as u32).to_le_bytes());
        engine.packet_write(&sizes);
        engine.packet_write(&init_packet(crc16(&image)));
        engine.control_write(&[INIT_PARAMETERS, 1], true);
        engine.control_write(&[RECEIVE_IMAGE], true);

        for chunk in image[..PREFIX_LEN].chunks(248) {
            let _ = engine.packet_write(chunk);
        }
        let steps = engine.packet_write(&image[PREFIX_LEN..]);

        assert_eq!(steps.len(), 4);
        // The current sector was already erased ahead of time; this flush
        // erases the sector beyond it.
        assert!(matches!(steps[0], DfuStep::Erase(sector) if sector == 2 * SECTOR_SIZE));
        assert!(
            matches!(&steps[1], DfuStep::Program { offset, data } if *offset == SECTOR_SIZE && data.len() == PAGE_SIZE)
        );
        assert!(
            matches!(&steps[2], DfuStep::Program { offset, data } if *offset == SECTOR_SIZE + PAGE_SIZE as u32 && data.len() == 8)
        );
        assert_eq!(steps[3], notify(&[RESPONSE, RECEIVE_IMAGE, STATUS_SUCCESS]));
    }
}
