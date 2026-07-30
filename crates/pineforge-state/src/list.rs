//! Paginated lists: which entries a page shows, and which entry a touch hit.
//!
//! A list is paginated rather than freely scrolled because the renderer has no
//! framebuffer: a page change is one full redraw per operating step, while
//! kinetic scrolling would be a full redraw per frame. So pagination is the
//! shape the hardware wants, and this module owns the arithmetic behind it.
//!
//! Nothing here knows what an entry is called, what it looks like, or what
//! happens when it is chosen. Entries are identified by index; the table that
//! gives them meaning belongs to the screen, the way `WATCHFACES` describes the
//! faces that `WatchfaceId` merely identifies.

use crate::{AppEvent, Button, ButtonBounds, ButtonOutcome, ButtonState, SwipeDirection};

/// The axis a list pages on.
///
/// This is not a free choice: a screen is left by the reverse of the gesture
/// that opened it, so the axis of the entry gesture belongs to navigation and
/// the list has to page on the other one. A launcher opened by swiping up pages
/// horizontally.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageAxis {
    Horizontal,
    Vertical,
}

impl PageAxis {
    /// Whether this gesture turns to the next page, the previous one, or belongs
    /// to somebody else.
    const fn paging(self, swipe: SwipeDirection) -> Option<bool> {
        match (self, swipe) {
            // The page follows the finger: dragging content left brings the next
            // page in from the right.
            (Self::Horizontal, SwipeDirection::Left) | (Self::Vertical, SwipeDirection::Up) => {
                Some(true)
            }
            (Self::Horizontal, SwipeDirection::Right) | (Self::Vertical, SwipeDirection::Down) => {
                Some(false)
            }
            _ => None,
        }
    }
}

/// How many entries a list holds, how many fit on a page, and which page shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PagedList {
    len: usize,
    per_page: usize,
    page: usize,
}

impl PagedList {
    /// A list of `len` entries showing `per_page` of them at a time.
    ///
    /// A page holds at least one entry; a zero-slot page would have no valid
    /// arithmetic at all.
    #[must_use]
    pub const fn new(len: usize, per_page: usize) -> Self {
        Self {
            len,
            per_page: if per_page == 0 { 1 } else { per_page },
            page: 0,
        }
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[must_use]
    pub const fn per_page(&self) -> usize {
        self.per_page
    }

    #[must_use]
    pub const fn page(&self) -> usize {
        self.page
    }

    /// Always at least one, so an empty list still has a page to draw.
    #[must_use]
    pub const fn page_count(&self) -> usize {
        if self.len == 0 {
            return 1;
        }
        self.len.div_ceil(self.per_page)
    }

    /// How many slots the current page actually fills. The last page is short
    /// whenever the entries do not divide evenly.
    #[must_use]
    pub const fn slots_on_page(&self) -> usize {
        let first = self.page * self.per_page;
        let remaining = self.len.saturating_sub(first);
        if remaining < self.per_page {
            remaining
        } else {
            self.per_page
        }
    }

    /// The entry shown in a slot of the current page.
    ///
    /// `None` for a slot the page does not fill, which is what keeps a touch on
    /// the empty half of a short last page from choosing an entry past the end.
    #[must_use]
    pub const fn entry_at(&self, slot: usize) -> Option<usize> {
        if slot >= self.per_page {
            return None;
        }
        let entry = self.page * self.per_page + slot;
        if entry < self.len { Some(entry) } else { None }
    }

    /// Turns to the next page, reporting whether there was one.
    pub const fn next_page(&mut self) -> bool {
        if self.page + 1 >= self.page_count() {
            return false;
        }
        self.page += 1;
        true
    }

    /// Turns to the previous page, reporting whether there was one.
    pub const fn previous_page(&mut self) -> bool {
        if self.page == 0 {
            return false;
        }
        self.page -= 1;
        true
    }

    /// Replaces the entry count, keeping the visible page in range.
    ///
    /// Returns whether the page moved, because entries can disappear while the
    /// user is looking at the last page.
    pub const fn set_len(&mut self, len: usize) -> bool {
        self.len = len;
        let last = self.page_count() - 1;
        if self.page > last {
            self.page = last;
            return true;
        }
        false
    }
}

/// What an event did to a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListOutcome {
    /// Nothing this list is responsible for.
    None,
    /// A slot changed how it draws.
    ///
    /// `Some(slot)` names the one that did, which is what a press and its
    /// release each are: one slot takes the pressed fill, the others are
    /// untouched. Repainting the rest costs a full-page SPI transfer for no
    /// visible change, so the slot is carried rather than left to be guessed.
    ///
    /// `None` when more than one may have moved and drawing has to assume the
    /// worst - a changed entry count shifts every slot's content at once.
    Redraw(Option<usize>),
    /// The visible page changed; redraw the page and its indicator.
    Paged,
    /// The user chose this entry.
    Activated(usize),
}

/// A page of touchable slots over a [`PagedList`].
///
/// `N` is the number of slots on a page, and the layout is fixed: slot 0 is
/// always drawn in the same place, whatever entry it currently shows. Touch
/// handling is delegated to [`Button`], so a press that slides off its slot
/// cancels here exactly as it does anywhere else.
///
/// One press belongs to one slot. A [`Button`] on its own cannot know that a
/// press began somewhere else, so a finger dragged across a menu would arrive
/// pressed in every slot it crosses and choose whichever one it was released
/// over. Ownership is tracked here instead: the slot a press starts in keeps it
/// until the finger lifts, and dragging out of that slot cancels rather than
/// hands the press on.
pub struct ListSlots<const N: usize> {
    list: PagedList,
    slots: [Button; N],
    axis: PageAxis,
    pressed: Option<usize>,
}

impl<const N: usize> ListSlots<N> {
    #[must_use]
    pub fn new(bounds: [ButtonBounds; N], axis: PageAxis, len: usize) -> Self {
        Self {
            list: PagedList::new(len, N),
            slots: core::array::from_fn(|slot| Button::new(bounds[slot])),
            axis,
            pressed: None,
        }
    }

    #[must_use]
    pub const fn list(&self) -> &PagedList {
        &self.list
    }

    /// Replaces the entry count; see [`PagedList::set_len`].
    pub const fn set_len(&mut self, len: usize) -> ListOutcome {
        if self.list.set_len(len) {
            ListOutcome::Paged
        } else {
            // A different entry count can change what every slot shows, so this
            // one cannot name a single slot.
            ListOutcome::Redraw(None)
        }
    }

    /// The slots the current page fills, paired with the entry each one shows.
    ///
    /// This is what drawing iterates: slot for the position, entry for the
    /// content.
    pub fn visible(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        (0..N).filter_map(|slot| self.list.entry_at(slot).map(|entry| (slot, entry)))
    }

    /// The pressed state of a slot, for drawing it.
    #[must_use]
    pub fn slot_state(&self, slot: usize) -> Option<ButtonState> {
        self.slots.get(slot).map(Button::state)
    }

    pub fn handle_event(&mut self, event: AppEvent) -> ListOutcome {
        match event {
            AppEvent::Swipe(direction) => self.turn_page(direction),
            AppEvent::Touch { pressed, .. } => self.touch(event, pressed),
            AppEvent::TouchCancelled => {
                // Read before releasing: the slot losing the press is the one
                // that has to be repainted, and `release` forgets which it was.
                let Some(owner) = self.pressed else {
                    return ListOutcome::None;
                };
                self.release();
                ListOutcome::Redraw(Some(owner))
            }
            _ => ListOutcome::None,
        }
    }

    fn turn_page(&mut self, direction: SwipeDirection) -> ListOutcome {
        let Some(forward) = self.axis.paging(direction) else {
            // The other axis is how this screen was entered, so the gesture
            // belongs to navigation and must not also turn a page.
            return ListOutcome::None;
        };
        let turned = if forward {
            self.list.next_page()
        } else {
            self.list.previous_page()
        };
        if turned {
            // The press that was in flight belongs to the page that just left.
            self.release();
            ListOutcome::Paged
        } else {
            ListOutcome::None
        }
    }

    fn touch(&mut self, event: AppEvent, pressed: bool) -> ListOutcome {
        if let Some(owner) = self.pressed {
            // Only the owning slot sees the rest of the press, so a finger
            // dragged onwards cannot arrive pressed in the next slot.
            let entry = self.list.entry_at(owner);
            let outcome = self.slots[owner].handle_event(event);
            if !pressed || self.slots[owner].state() != ButtonState::Pressed {
                self.pressed = None;
            }
            return match (outcome, entry) {
                (ButtonOutcome::None, _) => ListOutcome::None,
                (ButtonOutcome::Activated, Some(entry)) => ListOutcome::Activated(entry),
                // The entry went away mid-press, so there is nothing to choose.
                (ButtonOutcome::Activated | ButtonOutcome::Redraw, _) => {
                    ListOutcome::Redraw(Some(owner))
                }
            };
        }

        // A release nobody claimed is a leftover of a cancelled press.
        if !pressed {
            return ListOutcome::None;
        }
        for slot in 0..N {
            // A slot the page does not fill is not offered the event at all, so
            // it can neither activate nor draw itself as pressed.
            if self.list.entry_at(slot).is_none() {
                continue;
            }
            let _ = self.slots[slot].handle_event(event);
            if self.slots[slot].state() == ButtonState::Pressed {
                self.pressed = Some(slot);
                return ListOutcome::Redraw(Some(slot));
            }
        }
        ListOutcome::None
    }

    /// Drops a press in flight, leaving no slot drawn as pressed.
    fn release(&mut self) {
        if let Some(owner) = self.pressed.take() {
            let _ = self.slots[owner].handle_event(AppEvent::Touch {
                x: i32::MIN,
                y: i32::MIN,
                pressed: true,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLOT_HEIGHT: i32 = 40;

    /// Six stacked rows, the shape a settings page has.
    fn rows() -> [ButtonBounds; 6] {
        core::array::from_fn(|slot| {
            ButtonBounds::new(0, slot as i32 * SLOT_HEIGHT, 240, SLOT_HEIGHT)
        })
    }

    fn tap(slot: usize) -> [AppEvent; 2] {
        let y = slot as i32 * SLOT_HEIGHT + 2;
        [
            AppEvent::Touch {
                x: 10,
                y,
                pressed: true,
            },
            AppEvent::Touch {
                x: 10,
                y,
                pressed: false,
            },
        ]
    }

    fn activate<const N: usize>(slots: &mut ListSlots<N>, slot: usize) -> ListOutcome {
        let [press, release] = tap(slot);
        let _ = slots.handle_event(press);
        slots.handle_event(release)
    }

    #[test]
    fn pages_are_counted_from_the_entries_that_exist() {
        assert_eq!(PagedList::new(0, 6).page_count(), 1);
        assert_eq!(PagedList::new(1, 6).page_count(), 1);
        assert_eq!(PagedList::new(6, 6).page_count(), 1);
        assert_eq!(PagedList::new(7, 6).page_count(), 2);
        assert_eq!(PagedList::new(12, 6).page_count(), 2);
    }

    #[test]
    fn a_short_last_page_leaves_its_remaining_slots_empty() {
        let mut list = PagedList::new(7, 6);
        assert_eq!(list.slots_on_page(), 6);

        assert!(list.next_page());
        assert_eq!(list.slots_on_page(), 1);
        assert_eq!(list.entry_at(0), Some(6));
        // The entries these slots would show do not exist. Reporting them would
        // be the bug this type is here to prevent.
        for slot in 1..6 {
            assert_eq!(list.entry_at(slot), None);
        }
        assert_eq!(list.entry_at(6), None);
    }

    #[test]
    fn an_empty_list_still_has_a_page_to_draw() {
        let list = PagedList::new(0, 6);

        assert_eq!(list.page_count(), 1);
        assert_eq!(list.slots_on_page(), 0);
        assert_eq!(list.entry_at(0), None);
        assert!(list.is_empty());
    }

    #[test]
    fn paging_stops_at_both_ends() {
        let mut list = PagedList::new(7, 6);

        assert!(!list.previous_page());
        assert_eq!(list.page(), 0);
        assert!(list.next_page());
        assert!(!list.next_page());
        assert_eq!(list.page(), 1);
        assert!(list.previous_page());
        assert_eq!(list.page(), 0);
    }

    #[test]
    fn entries_that_disappear_pull_the_page_back_into_range() {
        let mut list = PagedList::new(13, 6);
        assert!(list.next_page());
        assert!(list.next_page());
        assert_eq!(list.page(), 2);

        // Two entries are gone and the page the user is on no longer exists.
        assert!(list.set_len(7));
        assert_eq!(list.page(), 1);
        assert_eq!(list.entry_at(0), Some(6));

        // A count that leaves the page valid does not move it.
        assert!(!list.set_len(12));
        assert_eq!(list.page(), 1);
    }

    #[test]
    fn a_zero_slot_page_is_refused_rather_than_dividing_by_zero() {
        let list = PagedList::new(3, 0);

        assert_eq!(list.per_page(), 1);
        assert_eq!(list.page_count(), 3);
    }

    #[test]
    fn a_slot_activates_the_entry_it_currently_shows() {
        let mut slots = ListSlots::new(rows(), PageAxis::Vertical, 7);

        assert_eq!(activate(&mut slots, 2), ListOutcome::Activated(2));
        // The same slot means a different entry one page on.
        assert_eq!(
            slots.handle_event(AppEvent::Swipe(SwipeDirection::Up)),
            ListOutcome::Paged
        );
        assert_eq!(activate(&mut slots, 0), ListOutcome::Activated(6));
    }

    #[test]
    fn an_empty_slot_on_a_short_page_activates_nothing() {
        let mut slots = ListSlots::new(rows(), PageAxis::Vertical, 7);
        let _ = slots.handle_event(AppEvent::Swipe(SwipeDirection::Up));

        // Slot 3 of the last page is drawn empty; pressing it must not choose
        // entry 9, and must not even look pressed.
        assert_eq!(activate(&mut slots, 3), ListOutcome::None);
        assert_eq!(slots.slot_state(3), Some(ButtonState::Idle));
    }

    #[test]
    fn a_swipe_along_the_navigation_axis_is_left_alone() {
        let mut slots = ListSlots::new(rows(), PageAxis::Vertical, 12);

        // This screen was entered horizontally, so a horizontal swipe is the way
        // out of it and must not be spent turning a page.
        for direction in [SwipeDirection::Left, SwipeDirection::Right] {
            assert_eq!(
                slots.handle_event(AppEvent::Swipe(direction)),
                ListOutcome::None
            );
            assert_eq!(slots.list().page(), 0);
        }

        let mut tiles = ListSlots::new(rows(), PageAxis::Horizontal, 12);
        for direction in [SwipeDirection::Up, SwipeDirection::Down] {
            assert_eq!(
                tiles.handle_event(AppEvent::Swipe(direction)),
                ListOutcome::None
            );
            assert_eq!(tiles.list().page(), 0);
        }
    }

    #[test]
    fn the_page_follows_the_finger() {
        let mut tiles = ListSlots::new(rows(), PageAxis::Horizontal, 12);

        assert_eq!(
            tiles.handle_event(AppEvent::Swipe(SwipeDirection::Left)),
            ListOutcome::Paged
        );
        assert_eq!(tiles.list().page(), 1);
        assert_eq!(
            tiles.handle_event(AppEvent::Swipe(SwipeDirection::Right)),
            ListOutcome::Paged
        );
        assert_eq!(tiles.list().page(), 0);
        // A swipe with no page behind it is not this list's business either.
        assert_eq!(
            tiles.handle_event(AppEvent::Swipe(SwipeDirection::Right)),
            ListOutcome::None
        );
    }

    #[test]
    fn visible_slots_pair_a_position_with_its_entry() {
        let mut slots = ListSlots::new(rows(), PageAxis::Vertical, 7);

        let first: heapless::Vec<_, 6> = slots.visible().collect();
        assert_eq!(
            first.as_slice(),
            [(0, 0), (1, 1), (2, 2), (3, 3), (4, 4), (5, 5)]
        );

        let _ = slots.handle_event(AppEvent::Swipe(SwipeDirection::Up));
        let last: heapless::Vec<_, 6> = slots.visible().collect();
        assert_eq!(last.as_slice(), [(0, 6)]);
    }

    #[test]
    fn a_press_cannot_slide_from_one_entry_to_another() {
        let mut slots = ListSlots::new(rows(), PageAxis::Vertical, 6);

        assert_eq!(
            slots.handle_event(AppEvent::Touch {
                x: 10,
                y: 2,
                pressed: true,
            }),
            // Naming the slot is what lets drawing repaint one row instead of
            // the page.
            ListOutcome::Redraw(Some(0))
        );
        assert_eq!(slots.slot_state(0), Some(ButtonState::Pressed));

        // Dragging into the next slot cancels the first and leaves the second
        // alone: the press did not start there.
        let _ = slots.handle_event(AppEvent::Touch {
            x: 10,
            y: SLOT_HEIGHT + 2,
            pressed: true,
        });
        assert_eq!(slots.slot_state(0), Some(ButtonState::Idle));
        assert_eq!(slots.slot_state(1), Some(ButtonState::Idle));

        // Releasing there must choose nothing at all - neither the entry the
        // press began on nor the one it ended over.
        assert_eq!(
            slots.handle_event(AppEvent::Touch {
                x: 10,
                y: SLOT_HEIGHT + 2,
                pressed: false,
            }),
            ListOutcome::None
        );
    }

    #[test]
    fn a_gesture_takes_its_touch_with_it() {
        let mut slots = ListSlots::new(rows(), PageAxis::Vertical, 6);
        let [press, release] = tap(1);
        let _ = slots.handle_event(press);

        // The swipe was recognised while the finger was still inside slot 1, and
        // the minimum swipe distance fits inside a row - so without the cancel
        // the release below would choose entry 1.
        assert_eq!(
            slots.handle_event(AppEvent::TouchCancelled),
            // The slot losing the press is the one that has to be repainted.
            ListOutcome::Redraw(Some(1))
        );
        assert_eq!(slots.slot_state(1), Some(ButtonState::Idle));
        assert_eq!(slots.handle_event(release), ListOutcome::None);
        assert_eq!(
            slots.handle_event(AppEvent::TouchCancelled),
            ListOutcome::None
        );
    }

    #[test]
    fn turning_the_page_drops_a_press_in_flight() {
        let mut slots = ListSlots::new(rows(), PageAxis::Vertical, 12);
        let _ = slots.handle_event(AppEvent::Touch {
            x: 10,
            y: 2,
            pressed: true,
        });

        let _ = slots.handle_event(AppEvent::Swipe(SwipeDirection::Up));
        // Slot 0 shows a different entry now, so the press must not survive to
        // activate it, and must not stay drawn as pressed either.
        assert_eq!(slots.slot_state(0), Some(ButtonState::Idle));
        assert_eq!(
            slots.handle_event(AppEvent::Touch {
                x: 10,
                y: 2,
                pressed: false,
            }),
            ListOutcome::None
        );
    }
}
