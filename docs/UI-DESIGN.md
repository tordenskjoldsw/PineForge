# UI design

What the firmware may look like, and why the hardware allows exactly this. The
constraints come first because they are not negotiable; the choices that follow
are, and they are recorded so a screen built next year matches one built today.

## What the renderer permits

There is no framebuffer. 240 x 240 pixels at two bytes each are 115 KiB against
a 44 KiB static RAM budget, so the display is painted through one 5.6 KiB
scratch buffer holding a stripe of 12 pixels. Three rules follow, and every
design decision below respects them:

- **Everything drawn is opaque.** No background survives underneath, so each
  element paints its own. Transparency, shadows and gradients exist only if a
  single stripe can compute them from geometry alone.
- **A screen is drawn many times per transition**, once per stripe, and must
  draw identically each time without changing state. That is why `draw_full`
  takes `&self`.
- **A page change is one full redraw**, which is why lists paginate instead of
  scrolling. Kinetic scrolling would be a redraw per frame, and the SPI bus is
  too slow for it.

The backlight has three levels, and the watch dims to the lowest one before
sleeping. **A colour that disappears at level 1 is unusable**, which rules out
dark blues and deep saturated tones for anything that carries meaning.

## Palette

Defined in RGB565 because the panel stores 5-6-5 bits: green has finer steps
than red and blue, so a colour picked as web hex shifts visibly. The values
live in `src/ui/theme.rs` and nowhere else — a screen never names a colour.

| Role | RGB565 | Approximate | Used for |
|---|---|---|---|
| `BACKGROUND` | `0x0000` | black | every screen's ground |
| `TEXT` | `0xFFFF` | white | labels, titles |
| `TEXT_MUTED` | `0x8410` | 50% grey | disabled entries, secondary lines |
| `FRAME` | `0x4208` | 25% grey | borders of rows and tiles |
| `ACCENT` | `0xFD20` | orange | values, selection fill |
| `LINK` | `0x451F` | azure | Bluetooth connected |
| `OK` | `0x07E0` | green | battery healthy |
| `WARN` | `0xFFE0` | yellow | battery low |
| `DANGER` | `0xF800` | red | battery critical, disconnected, rollback, DFU failure |

Orange stays the accent because the terminal look is the decided default and
that is its colour. The four semantic colours exist because status has to be
readable at a glance rather than deciphered: they are for state, never for
decoration. Azure rather than pure blue (`0x001F`) because pure blue is the
first colour to vanish when the backlight dims.

## Type

Every additional size costs internal flash and is drawn from the same shared
font data, so the scale is deliberately short:

| Size | Use |
|---|---|
| 10 x 20 monospace | everything: rows, labels, titles, values |
| large digits | the clock on a watchface, per face, digits `0-9:` only |

Large digits are per-watchface rather than system-wide: a full proportional
face at 40 px does not fit the flash budget, while digits alone cost about
2.5 KiB monochrome and about 10 KiB with 4-bit antialiasing. Body text stays
small. Symbols are 24 x 24 and monochrome, tinted by the palette above.

## Component states

Every touchable component has four, and they are drawn the same way everywhere:

| State | Drawing |
|---|---|
| normal | `FRAME` border, `TEXT` label, `ACCENT` value |
| pressed | filled `ACCENT`, label and value in `BACKGROUND` |
| disabled | `FRAME` border, `TEXT_MUTED` label and value |
| selected | filled `ACCENT` like pressed, but held until the choice changes |

Pressed inverts rather than merely recolouring because inversion is the only
treatment that survives the lowest backlight level, and because a filled area
is pure geometry — a stripe can compute it without knowing anything outside
itself.

## Status corner

The watchface is the main display and owns its whole surface; each face decides
what it shows, and several faces will differ. Every other screen carries two
symbols in the top-right corner instead, in a 16-pixel strip:

```text
+------------------------------ 240 --+
|                          [BT] [BAT] |  16 px status strip
|                                     |
|            screen content           |
```

- **Bluetooth**: `LINK` when connected, `DANGER` when not.
- **Battery**: `OK` above 50%, `WARN` from 20 to 50%, `DANGER` below 20%.

The thresholds are product policy and live in the state crate, host-tested; the
strip only maps a level to a colour. Both symbols redraw when their value
changes, not on a timer, so a menu costs no periodic drawing.

## Launcher

Four tiles per page, two by two, below the status strip:

```text
+------------------------------ 240 --+
|                          [BT] [BAT] |  16
|  +-------------+  +-------------+   |
|  |             |  |             |   |
|  |  SETTINGS   |  |    TIMER    |   |  112
|  |    [icon]   |  |    [icon]   |   |
|  +-------------+  +-------------+   |
|  +-------------+  +-------------+   |
|  |             |  |             |   |
|  |   HEART     |  |  FIRMWARE   |   |  112
|  |    [icon]   |  |    [icon]   |   |
|  +-------------+  +-------------+   |
|              o  .                   |  page indicator
+-------------------------------------+
```

Tiles are 120 x 112 with a two-pixel gap, which leaves a comfortable target and
room for a 24 x 24 icon above a label. The launcher is opened by swiping up, so
navigation owns the vertical axis and **pages turn horizontally** — the page
indicator sits at the bottom, one dot per page, the current one filled.

## Menus

Rows are 200 x 34 at x = 20, the geometry already in use. Under a status strip
a page holds six of them; the paginated list model decides which entries a page
shows and which entry a touch hit, and pages turn vertically because a settings
screen is reached by tapping rather than by a gesture.

## What this does not decide

- **Animation timing.** Transitions currently run as fast as SPI allows, with
  no easing, because that would need a timer inside the transition. A design
  must not depend on motion curves before that exists.
- **Full-screen artwork.** It would live in external flash, which shares the
  SPI bus with the display, so it is read once per screen change and never per
  redraw. Fonts and symbols stay in internal flash for exactly that reason.
- **Watchface looks.** Faces are free to differ completely; this document
  governs the system around them.
