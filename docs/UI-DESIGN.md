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
| `FRAME` | `0x5ACB` | 35% grey | page marks not on the current page, rules |
| `SURFACE` | `0x2125` | near-black grey | the face of a tile or a row |
| `ACCENT` | `0x1E33` | verdigris | values, icons, selection fill |
| `LINK` | `0x451F` | azure | Bluetooth connected |
| `OK` | `0x5646` | leaf green | battery healthy |
| `WARN` | `0xFFE0` | yellow | battery low |
| `DANGER` | `0xF800` | red | battery critical, disconnected, rollback, DFU failure |

Verdigris is the accent because the product is called PineForge: it is what
forged copper ages into and what a pine needle is in shade, so the one colour
the firmware is recognised by says its name. The earlier indigo was not chosen
so much as left over — the four semantic colours below spend red, yellow, green
and azure, and the accent had been pushed into the last unused sector of the
wheel to keep it from reading as a warning.

That constraint has not gone away, and the neighbour that matters is `OK`: a
healthy battery is the one status colour on screen almost all the time, sitting
in the corner above a menu full of accent. The two are held apart by hue, 58
degrees of it, and both of them moved to get there — `OK` went from maximal
green to a leaf green shifted toward yellow, away from the accent rather than
merely darker. Brightness alone would not have done it, because both want to be
bright.

Being green-blue also means the accent survives dimming, which is the harder
test — green has six bits where red and blue have five, and pure blue is the
first colour to vanish, which is why `LINK` is azure rather than `0x001F`.

The four semantic colours exist because status has to be readable at a glance
rather than deciphered: they are for state, never for decoration.

**Anything that carries meaning clears 3:1 against the ground**, which is what
WCAG 1.4.11 asks of a non-text element. `FRAME` was 2.07:1 and failed it, and
the failure was visible rather than theoretical: a page rail drew one bright
mark and, to the eye, nothing at all beside it, so a screen with two pages
looked like a screen with one and a stray piece of decoration. It is a useful
line to hold on a panel that also dims to a third of its brightness before it
sleeps — the contrast measured here is the best case, not the typical one.

Components sit on `SURFACE` rather than on the background. Keeping the panel
mostly black is what it is good at, but a tile that is only an outline reads as
a frame around nothing; a face one step above the background reads as an object
that can be pressed.

There is no muted ink. A disabled state was specified here before anything
needed one, and a colour nothing draws in is a colour that drifts out of step
with the palette unnoticed. It comes back with the first component that has
something to disable.

## Type

Every additional size costs internal flash and is drawn from the same shared
font data, so the scale is deliberately short:

| Size | Use |
|---|---|
| 10 x 22 monospace | rows, labels, titles, values — the UI face |
| 6 x 14 monospace | hints and secondary lines |
| large digits | the clock on a watchface, per face, digits `0-9:` only — not built |

Both are JetBrains Mono, carrying **four bits of coverage per pixel** rather
than one. `embedded-graphics`' own `MonoFont` stores a single bit, so every
glyph edge is fully on or fully off and curves step; LVGL renders the reference
firmware for this watch at 4 bpp, and that difference is most of what separates
the two on screen. The atlases are generated by `tools/generate-font.py` and
committed, so a normal build needs neither Python nor the source face.

Large digits are per-watchface rather than system-wide: a full proportional
face at 40 px does not fit the flash budget, while digits alone cost about
2.5 KiB monochrome and about 10 KiB with 4-bit antialiasing. Body text stays
small. Symbols are 24 x 24 and one bit per pixel, tinted at draw time so one
bitmap serves a component in every colour it takes.

## Component states

Every touchable component has three, and they are drawn the same way
everywhere:

| State | Drawing |
|---|---|
| normal | `SURFACE` face, `TEXT` label, `ACCENT` value or icon |
| pressed | filled `ACCENT`, label and icon in `BACKGROUND` |
| selected | filled `ACCENT` like pressed, but held until the choice changes |

Rows are rounded like tiles, not squared off against them. The distinction
drawn here once — a tile is an object, a row is a line in a list — did not
survive contact: an outline says "field", a filled and rounded shape says
"press me", and pressing is the only thing a menu row is for. So a row carries
the same face and the same curve a tile does, and darkens under a finger the
same way.

The rounding is five inset fills per corner rather than a rounded rectangle
primitive, because that primitive is a rasteriser the firmware needs nowhere
else and flash is the scarcer budget.

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
|  |    [icon]   |  |    [icon]   |   |
|  |  SETTINGS   |  |  FIRMWARE   |   |  106
|  |             |  |             |   |
|  +-------------+  +-------------+   |
|  +-------------+  +-------------+   |
|  |             |  |             |   |
|  |             |  |             |   |  106
|  |             |  |             |   |
|  +-------------+  +-------------+   |
|              o  .                   |  page indicator
+-------------------------------------+
```

Two tiles today, a third under `diagnostics`. The grid is drawn at its full
four slots because that is what the geometry reserves; the apps to fill it are
the roadmap's business, not this document's.

Tiles are 115 x 106 with a two-pixel gap and a four-pixel margin, leaving room
under the second row for the page rail. The launcher is opened by swiping up, so
navigation owns the vertical axis and **pages turn horizontally**.

## The page rail

Every paginated screen reports its position the same way, through one shared
component.

**The rail runs along the axis the screen pages on.** That is the whole content
of the message: a column of marks beside a list that pages up and down says
there is more that way, while the same marks in a row underneath would point
across a movement that never goes across. So the launcher's rail lies flat
under the tiles and a menu's stands upright beside the rows.

**The showing page is a bar, the rest are squares** — 24 pixels against 6,
6 across, 6 apart. Two treatments rather than one, because the second is
colour, and colour has to be seen accurately at backlight level 1 to carry
anything; the long mark still reads as position when the palette barely does.
The showing mark is `ACCENT` and the rest are `FRAME`.

Squares rather than dots: a circle is a rasteriser the firmware otherwise never
needs, and at four pixels the shapes are indistinguishable.

Exactly one mark is long whichever page shows, so the run keeps its length and
its box. That is what lets the component clear one fixed rectangle before
drawing — without it, a page turn would move the bar within the run and a
partial redraw would leave the tail of its old position standing.

Nothing is drawn for a single page. There is no position to report when there is
only one place to be, and a lone mark would read as a control.

This shape stops working somewhere around ten pages, where the marks are too
many to count and the rail runs out of edge to sit on. No screen is near that,
but it is the boundary at which this component should become something else — a
proportional thumb — rather than be made smaller.

A tile carries a 24 x 24 icon above a centred label. Icons are one-bit bitmaps,
four bytes per row, tinted at draw time so one bitmap serves both the normal and
the pressed colour. Drawing them as geometry would cost more than storing them:
a gear built from arcs needs a rasteriser, while its bitmap costs 96 bytes.

## Menus

Rows are 200 x 42 at x = 20, set on a 48-pixel rhythm. Under a status strip a
page holds **four** of them; a page that also carries a title holds three. The
paginated list model decides which entries a page shows and which entry a touch
hit, and pages turn vertically because a settings screen is reached by tapping
rather than by a gesture.

A menu is a description rather than drawing code: the rows, the rhythm, the
title and the hint are a `'static` value, and one interpreter draws all of
them. A row states its geometry once, so it cannot disagree with its own hit
box — which is the failure the three-places-per-row arrangement kept producing.

The page rail sits in the gutter to the right of the rows, centred on the panel
rather than on them, so it stays put between screens that carry different
numbers of rows.

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
