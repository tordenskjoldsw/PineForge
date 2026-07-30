//! Which watchfaces exist and how the choice survives a reboot.
//!
//! The identity lives here rather than in the rendering code because it is
//! persisted: the byte written to flash is a compatibility surface, so it sits
//! next to the table that says which faces this build carries at all.

/// Identifies a watchface across reboots.
///
/// A build carries only the faces it was compiled with, so a record written by
/// another build may name one that is missing; [`Self::from_byte`] reports that
/// as absent rather than as corruption.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchfaceId {
    Terminal,
    /// `PineForge`'s own face: the time in numerals built from rectangles.
    Forge,
    /// Raw sensor readings for bring-up, in diagnostics builds only.
    #[cfg(feature = "diagnostics")]
    Diagnostics,
}

impl Default for WatchfaceId {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl WatchfaceId {
    /// The face a watch with no stored choice opens with.
    ///
    /// A `const` rather than only the [`Default`] impl because
    /// `DisplaySettings::DEFAULT` is itself a `const` and cannot call a trait
    /// method. It used to keep its own copy of this answer, and the two drifted
    /// the moment one of them changed - so `Default` defers to this and there is
    /// one answer again.
    ///
    /// Only a watch with no stored settings sees it: a record naming a face
    /// keeps that face, so an existing watch stays on whatever it was set to.
    pub const DEFAULT: Self = {
        // A diagnostics build exists to be diagnosed with.
        #[cfg(feature = "diagnostics")]
        {
            Self::Diagnostics
        }
        // Otherwise `PineForge`'s own face, not the one it borrowed.
        #[cfg(not(feature = "diagnostics"))]
        {
            Self::Forge
        }
    };

    /// Every face this build carries.
    ///
    /// Exists so that checks over the faces are driven by the list rather than
    /// by whichever one happens to be selected. A face is only as correct as the
    /// tests that see it, and the opacity contract - a face must paint every
    /// pixel, because the slide transition never clears behind one - is exactly
    /// the kind of thing a second face gets wrong silently.
    ///
    /// A variant added above and forgotten here is caught rather than trusted:
    /// [`Self::to_byte`] matches exhaustively, so a new face must be given a
    /// byte, and `every_face_is_listed_in_all` walks every byte that decodes and
    /// requires the face behind it to appear in this list.
    pub const ALL: &'static [Self] = &[
        Self::Terminal,
        Self::Forge,
        #[cfg(feature = "diagnostics")]
        Self::Diagnostics,
    ];

    /// The persisted encoding. These numbers are permanent: reusing one would
    /// silently switch a watch to a different face on the next boot.
    #[must_use]
    pub const fn to_byte(self) -> u8 {
        match self {
            Self::Terminal => 0,
            Self::Forge => 2,
            #[cfg(feature = "diagnostics")]
            Self::Diagnostics => 1,
        }
    }

    /// Returns the face for a persisted byte, or `None` when this build does
    /// not carry it.
    #[must_use]
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Terminal),
            2 => Some(Self::Forge),
            #[cfg(feature = "diagnostics")]
            1 => Some(Self::Diagnostics),
            _ => None,
        }
    }
}

/// A face offered to the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WatchfaceDescriptor {
    pub id: WatchfaceId,
    /// Shown in the picker.
    pub name: &'static str,
}

const TERMINAL: WatchfaceDescriptor = WatchfaceDescriptor {
    id: WatchfaceId::Terminal,
    name: "TERMINAL",
};

const FORGE: WatchfaceDescriptor = WatchfaceDescriptor {
    id: WatchfaceId::Forge,
    name: "FORGE",
};

#[cfg(feature = "diagnostics")]
const DIAGNOSTICS: WatchfaceDescriptor = WatchfaceDescriptor {
    id: WatchfaceId::Diagnostics,
    name: "DIAG",
};

/// Every face this build can show, in the order a picker lists them.
#[cfg(not(feature = "diagnostics"))]
pub const WATCHFACES: &[WatchfaceDescriptor] = &[TERMINAL, FORGE];
#[cfg(feature = "diagnostics")]
pub const WATCHFACES: &[WatchfaceDescriptor] = &[TERMINAL, FORGE, DIAGNOSTICS];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_face_is_one_this_build_carries() {
        let default = WatchfaceId::default();
        assert!(WATCHFACES.iter().any(|face| face.id == default));
    }

    #[test]
    fn every_face_round_trips_through_its_persisted_byte() {
        for face in WATCHFACES {
            assert_eq!(WatchfaceId::from_byte(face.id.to_byte()), Some(face.id));
        }
    }

    #[test]
    fn ids_are_distinct_so_no_face_is_mistaken_for_another() {
        for (index, face) in WATCHFACES.iter().enumerate() {
            for other in &WATCHFACES[index + 1..] {
                assert_ne!(face.id, other.id);
                assert_ne!(face.id.to_byte(), other.id.to_byte());
            }
        }
    }

    #[test]
    fn a_face_this_build_lacks_is_absent_rather_than_invalid() {
        assert_eq!(WatchfaceId::from_byte(0xFF), None);
        #[cfg(not(feature = "diagnostics"))]
        assert_eq!(WatchfaceId::from_byte(1), None);
    }

    /// Guards [`WatchfaceId::ALL`] against a face that was added and forgotten.
    ///
    /// `to_byte` matches exhaustively, so a new variant cannot compile without
    /// being given a byte. This then walks every byte that decodes and insists
    /// the face behind it is listed - which is what keeps the rendering tests,
    /// driven by that list, from quietly skipping a face nobody selected.
    #[test]
    fn every_face_is_listed_in_all() {
        for byte in u8::MIN..=u8::MAX {
            let Some(face) = WatchfaceId::from_byte(byte) else {
                continue;
            };
            assert!(
                WatchfaceId::ALL.contains(&face),
                "{face:?} decodes from byte {byte} but is missing from WatchfaceId::ALL"
            );
        }
    }

    /// And nothing is listed twice, which would run a check on one face while
    /// reporting the count of another.
    #[test]
    fn all_lists_each_face_once() {
        for (index, face) in WatchfaceId::ALL.iter().enumerate() {
            assert!(
                !WatchfaceId::ALL[..index].contains(face),
                "{face:?} appears twice in WatchfaceId::ALL"
            );
        }
    }
}
