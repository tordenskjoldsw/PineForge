//! The prompts that own the whole panel.
//!
//! A modal is not a screen: it has no place in the navigation stack and keeps
//! no state, because everything it shows is in the [`Modal`] value that raised
//! it. What it shares with a screen is the contract that actually matters
//! here: it must paint every pixel it covers. Nothing clears behind a modal,
//! so a gap does not come out black, it comes out as whatever the screen
//! underneath had there.
//!
//! Which is why this dispatch lives here rather than in the display task, where
//! it sat next to the SPI bus. The screens have been checked against that
//! contract on the host since they moved into this crate; the modals drawn over
//! them were the one thing still painting to a panel nobody could inspect.

use pineforge_state::Modal;

use crate::{
    canvas::{Canvas, CanvasError},
    dfu::{draw_dfu_failed, draw_dfu_progress, draw_storage_progress, refresh_progress},
    pairing::draw_pairing,
    timer::draw_expired_modal,
};

/// Draws the modal that owns the screen.
pub fn draw(
    canvas: &mut Canvas<'_>,
    modal: Modal,
    keep_alive: &mut dyn FnMut(),
) -> Result<(), CanvasError> {
    match modal {
        Modal::StorageFormat(percent) => draw_storage_progress(canvas, percent, keep_alive),
        Modal::Pairing(passkey) => draw_pairing(canvas, passkey, keep_alive),
        Modal::DfuProgress(percent) => draw_dfu_progress(canvas, percent, keep_alive),
        Modal::DfuFailed(reason) => draw_dfu_failed(canvas, reason, keep_alive),
        Modal::TimerExpired => draw_expired_modal(canvas, keep_alive),
    }
}

/// Repaints the part of a showing modal that its new value moved.
///
/// A transfer reports about a hundred percentage steps, and a full repaint per
/// step blanks the panel for the length of an SPI frame each time - which is
/// the flicker. The two progress prompts have a partial path; the others have
/// nothing that moves without the whole prompt changing, so they fall back.
pub fn refresh(
    canvas: &mut Canvas<'_>,
    modal: Modal,
    keep_alive: &mut dyn FnMut(),
) -> Result<(), CanvasError> {
    match modal {
        Modal::StorageFormat(percent) | Modal::DfuProgress(percent) => {
            refresh_progress(canvas, percent, keep_alive)
        }
        Modal::Pairing(_) | Modal::DfuFailed(_) | Modal::TimerExpired => {
            draw(canvas, modal, keep_alive)
        }
    }
}
