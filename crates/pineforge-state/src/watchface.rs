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
    /// Raw sensor readings for bring-up, in diagnostics builds only.
    #[cfg(feature = "diagnostics")]
    Diagnostics,
}

// Derivable in a build carrying one face, but not in one carrying two.
#[allow(clippy::derivable_impls)]
impl Default for WatchfaceId {
    fn default() -> Self {
        // A diagnostics build exists to be diagnosed with.
        #[cfg(feature = "diagnostics")]
        {
            Self::Diagnostics
        }
        #[cfg(not(feature = "diagnostics"))]
        {
            Self::Terminal
        }
    }
}

impl WatchfaceId {
    /// The persisted encoding. These numbers are permanent: reusing one would
    /// silently switch a watch to a different face on the next boot.
    #[must_use]
    pub const fn to_byte(self) -> u8 {
        match self {
            Self::Terminal => 0,
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

#[cfg(feature = "diagnostics")]
const DIAGNOSTICS: WatchfaceDescriptor = WatchfaceDescriptor {
    id: WatchfaceId::Diagnostics,
    name: "DIAG",
};

/// Every face this build can show, in the order a picker lists them.
#[cfg(not(feature = "diagnostics"))]
pub const WATCHFACES: &[WatchfaceDescriptor] = &[TERMINAL];
#[cfg(feature = "diagnostics")]
pub const WATCHFACES: &[WatchfaceDescriptor] = &[TERMINAL, DIAGNOSTICS];

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
}
