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
live in `crates/pineforge-ui/src/theme.rs` and nowhere else — a screen never
names a colour.

| Role | RGB565 | Approximate | Used for |
|---|---|---|---|
| `BACKGROUND` | `0x0000` | black | every screen's ground |
| `TEXT` | `0xFFFF` | white | labels, titles |
| `TEXT_MUTED` | `0x8410` | 50% grey | disabled entries, secondary lines |
| `FRAME` | `0x4208` | 25% grey | borders of rows |
| `SURFACE` | `0x2125` | near-black grey | the face of a tile or a row |
| `ACCENT` | `0x1E33` | verdigris | values, icons, selection fill |
| `LINK` | `0x451F` | azure | Bluetooth connected |
| `OK` | `0x07E0` | green | battery healthy |
| `WARN` | `0xFFE0` | yellow | battery low |
| `DANGER` | `0xF800` | red | battery critical, disconnected, rollback, DFU failure |

Verdigris is the accent because the product is called PineForge: it is what
forged copper ages into and what a pine needle is in shade, so the one colour
the firmware is recognised by says its name. The earlier indigo was not chosen
so much as left over — the four semantic colours below spend red, yellow, green
and azure, and the accent had been pushed into the last unused sector of the
wheel to keep it from reading as a warning.

That constraint has not gone away; verdigris answers it differently. It clears
`OK` by saturation rather than by hue: pure green carries no blue at all, this
carries a great deal, so the two read as different colours rather than as two
greens. Being green-blue also means it survives dimming, which is the harder
test — green has six bits where red and blue have five, and pure blue is the
first colour to vanish, which is why `LINK` is azure rather than `0x001F`.

The four semantic colours exist because status has to be readable at a glance
rather than deciphered: they are for state, never for decoration.

Components sit on `SURFACE` rather than on the background. Keeping the panel
mostly black is what it is good at, but a tile that is only an outline reads as
a frame around nothing; a face one step above the background reads as an object
that can be pressed.

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
| normal | `SURFACE` face, `TEXT` label, `ACCENT` value or icon |
| pressed | filled `ACCENT`, label and icon in `BACKGROUND` |
| disabled | `SURFACE` face, `TEXT_MUTED` label and value |
| selected | filled `ACCENT` like pressed, but held until the choice changes |

Tiles have rounded corners, rows do not — a tile is an object, a row is a line
in a list. The rounding is five inset fills per corner rather than a rounded
rectangle primitive, because that primitive is a rasteriser the firmware needs
nowhere else and flash is the scarcer budget.

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

Tiles are 115 x 106 with a two-pixel gap and a four-pixel margin, leaving room
under the second row for the page indicator. The launcher is opened by swiping
up, so navigation owns the vertical axis and **pages turn horizontally** — the
indicator sits at the bottom, one mark per page, the current one in `ACCENT` and
the rest in `FRAME`. It is absent while there is only one page, because there is
nothing to indicate.

The marks are four-pixel squares rather than dots: a circle is a rasteriser the
firmware otherwise never needs, and at that size the shapes are
indistinguishable.

A tile carries a 24 x 24 icon above a centred label. Icons are one-bit bitmaps,
four bytes per row, tinted at draw time so one bitmap serves both the normal and
the pressed colour. Drawing them as geometry would cost more than storing them:
a gear built from arcs needs a rasteriser, while its bitmap costs 96 bytes.

## Menus

Rows are 200 x 34 at x = 20, the geometry already in use. Under a status strip
a page holds six of them; the paginated list model decides which entries a page
shows and which entry a touch hit, and pages turn vertically because a settings
screen is reached by tapping rather than by a gesture.

## Notifications

One message per page, not a list of subject lines. The panel fits about six
lines of body text, so a list would show almost nothing of any message and still
cost a second screen to read one.

Which gestures the screen can use follows from how it is opened. Pulling down
from the watchface brings it up, so up is spent leaving again, and what is left
is down and the horizontal pair. Down browses to the next message and wraps at
the oldest — the finger that opened the screen keeps going through it. Right
dismisses.

**Browsing and dismissing sit on different axes deliberately.** Both are single
swipes and one of them destroys something, so a gesture that lands short or
crooked must not be able to delete the message being read.

No watchface carries a notification count. What is pending is read where it is
read, and a tally on a face would only be a second place to keep it right.

## What this does not decide

- **Animation timing.** Transitions currently run as fast as SPI allows, with
  no easing, because that would need a timer inside the transition. A design
  must not depend on motion curves before that exists.
- **Full-screen artwork.** It would live in external flash, which shares the
  SPI bus with the display, so it is read once per screen change and never per
  redraw. Fonts and symbols stay in internal flash for exactly that reason.
- **Watchface looks.** Faces are free to differ completely; this document
  governs the system around them.
