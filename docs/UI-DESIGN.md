# UI design

What the firmware may look like, and why the hardware allows exactly this. The
constraints come first because they are not negotiable; the choices that follow
are, and they are recorded so a screen built next year matches one built today.

## What the renderer permits

There is no framebuffer. 240 x 240 pixels at two bytes each are 115 KiB against
a 48 KiB static RAM budget, so the display is painted through one 5.6 KiB
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
- **Vertical movement is free, horizontal movement is not.** The controller
  holds 240x320 pixels and shows a 240-row window of them, and which rows are
  shown is a register. So a screen or a page can be slid on vertically for the
  price of one frame, while across the panel nothing can move at all and the
  best available is a strip-by-strip reveal. Anything that should look like it
  moves belongs on the vertical axis; see the launcher.

The backlight has three levels, and the watch dims to the lowest one before
sleeping. **A colour that disappears at level 1 is unusable**, which rules out
dark blues and deep saturated tones for anything that carries meaning.

## Palette

Defined in RGB565 because the panel stores 5-6-5 bits: green has finer steps
than red and blue, so a colour picked as web hex shifts visibly. The values
live in `crates/pineforge-ui/src/theme.rs` and nowhere else - a screen never
names a colour.

| Role | RGB565 | Approximate | Used for |
|---|---|---|---|
| `BACKGROUND` | `0x0000` | black | every screen's ground |
| `TEXT` | `0xFFFF` | white | labels, titles |
| `MUTED` | `0x7BCF` | 48% grey | a control that is there but cannot be used |
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
so much as left over - the four semantic colours below spend red, yellow, green
and azure, and the accent had been pushed into the last unused sector of the
wheel to keep it from reading as a warning.

That constraint has not gone away, and the neighbour that matters is `OK`: a
healthy battery is the one status colour on screen almost all the time, sitting
in the corner above a menu full of accent. The two are held apart by hue, 58
degrees of it, and both of them moved to get there - `OK` went from maximal
green to a leaf green shifted toward yellow, away from the accent rather than
merely darker. Brightness alone would not have done it, because both want to be
bright.

Being green-blue also means the accent survives dimming, which is the harder
test - green has six bits where red and blue have five, and pure blue is the
first colour to vanish, which is why `LINK` is azure rather than `0x001F`.

The four semantic colours exist because status has to be readable at a glance
rather than deciphered: they are for state, never for decoration.

**Anything that carries meaning clears 3:1 against the ground**, which is what
WCAG 1.4.11 asks of a non-text element. `FRAME` was 2.07:1 and failed it, and
the failure was visible rather than theoretical: a page rail drew one bright
mark and, to the eye, nothing at all beside it, so a screen with two pages
looked like a screen with one and a stray piece of decoration. It is a useful
line to hold on a panel that also dims to a third of its brightness before it
sleeps - the contrast measured here is the best case, not the typical one.

Components sit on `SURFACE` rather than on the background. Keeping the panel
mostly black is what it is good at, but a tile that is only an outline reads as
a frame around nothing; a face one step above the background reads as an object
that can be pressed.

`MUTED` is the disabled ink, and it was held back until something needed it: a
colour nothing draws in drifts out of step with the palette unnoticed. The
music transport is what earned it. Everything that screen does happens at the
other end of a Bluetooth link, so with no phone connected its three buttons are
the whole screen, and drawing them as though pressing them would do something
is the one thing on it that would be actively false. It sits between `SURFACE`
and `TEXT`: 3:1 against the ground so it survives backlight level 1, and about
3.5:1 against the face it sits on, which is what keeps a disabled control
legible as a control rather than as a smudge.

## Type

Every additional size costs internal flash and is drawn from the same shared
font data, so the scale is deliberately short:

| Size | Use |
|---|---|
| 10 x 22 monospace | rows, labels, titles, values - the UI face |
| 8 x 18 monospace | running prose - a notification body, and nothing else so far |
| 6 x 14 monospace | hints and secondary lines |
| 12 x 27 bold monospace | FORGE timer title and phase labels |
| segment numerals | a number that is the point of a screen - built from rectangles, not from an atlas |

The reading size exists because the other two are not one. A notification body
set in the hint face fitted 203 characters into a card that never carries more
than a hundred, and in the UI face it would not have fitted at all. A size earns
an atlas only when something is genuinely set in it; the three regular weights
plus one instrument-label face are the whole scale.

The complete UI family is Liberation Mono: Bold for the UI and instrument
labels, Regular for prose and hints. All carry **four bits of coverage per pixel**
rather than one. `embedded-graphics`' own `MonoFont` stores a single bit, so every glyph
edge is fully on or fully off and curves step; LVGL renders the reference
firmware for this watch at 4 bpp, and that difference is most of what separates
the two on screen. The atlases are generated by `tools/generate-font.py` and
committed, so a normal build needs neither Python nor the source face.

Large digits are not a font at all. They are seven rectangles and a table of
which are lit - see `crates/pineforge-ui/src/segment.rs` - because at the sizes
a number is meant to be read across a room an atlas is the expensive way to get
one: the FORGE clock's digits would cost around 23 KiB as a font. The size is a
parameter, so the same numerals set the clock at 70 x 80, a step count at
40 x 54 and a watchface footer at 16 x 22.

Unlit strokes are drawn too, in the surface colour. That is what makes a number
read as an instrument rather than as a square typeface, and it is why a screen
built from these does not mix them with type: a footer set in an antialiased
atlas under a clock built from rectangles is two rendering techniques stacked,
and no rearranging reconciles them. Symbols are 24 x 24 and one bit per pixel, tinted at draw time so one
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
drawn here once - a tile is an object, a row is a line in a list - did not
survive contact: an outline says "field", a filled and rounded shape says
"press me", and pressing is the only thing a menu row is for. So a row carries
the same face and the same curve a tile does, and darkens under a finger the
same way.

The rounding is five inset fills per corner rather than a rounded rectangle
primitive, because that primitive is a rasteriser the firmware needs nowhere
else and flash is the scarcer budget.

Pressed inverts rather than merely recolouring because inversion is the only
treatment that survives the lowest backlight level, and because a filled area
is pure geometry - a stripe can compute it without knowing anything outside
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
|   +-----------+   +-----------+     |
|   |  [icon]   |   |  [icon]   |     |
|   | SETTINGS  |   | FIRMWARE  |     |  88
|   +-----------+   +-----------+     |
|   +-----------+   +-----------+     |
|   |  [icon]   |   |  [icon]   |     |
|   |   ABOUT   |   |   LIGHT   |     |  88
|   +-----------+   +-----------+     |
|              o  .                   |  page rail
+-------------------------------------+
```

Thirteen production tiles today across four pages - settings, firmware, about,
light, pulse, steps, music, stopwatch, timer, time, date, Bluetooth, and
battery. Diagnostics adds touch as a fourteenth tile. The grid always draws its
full four slots: an empty slot still paints, because the surface has to be
opaque or the previous page shows through.

The stopwatch deliberately borrows the FORGE instrument language rather than
looking like another menu. Five rectangle-built digits show `MM:SS.T`; the
running digits use the accent colour, while paused and ready values use normal
text. RESET and the changing START/PAUSE/RESUME control remain ordinary touch
buttons so their interaction state is unambiguous. Elapsed time is derived from
monotonic uptime, not counted repaint by repaint, so display sleep and leaving
the app do not lose time.

Steps and pulse share the same instrument hierarchy: a turquoise bold title
over rectangle-built numerals, followed by smaller Regular guidance. Pulse also
sets its changing READY/MEASURING/BPM/error state in the bold instrument face;
the line below it remains an instruction rather than a competing title. All
four instrument screens use one centred-label renderer.

The timer uses the same digits at `MM:SS`, but keeps adjustment and execution
as two explicit control rows. The minute controls are enabled only while ready;
the lower row changes from RESET/START to CANCEL/PAUSE and RESET/RESUME as the
countdown moves. Its title, phase and alarm title use a dedicated 12 x 27
native bold uppercase cut: larger and heavier than menu text without a synthetic
outline, and only 26 glyphs in flash. A running value is `ACCENT`, a paused or ready value is
`TEXT`, and zero is `DANGER`. Expiry owns a full-screen red `00:00` modal above
the navigation stack, because a vibration without an explanation is not an
alarm.

TIME and DATE are separate manual editors but deliberately share one control
grammar. The selected field is turquoise in the large rectangle-built value;
the remaining fields stay white. The upper buttons decrement and increment the
selected field, NEXT advances through hour/minute or year/month/day, and APPLY
updates the system clock. Invalid dates are impossible: changing year or month
clamps the day to the destination month's valid range.

Their partial repaint follows the timer rather than clearing the value row: an
ordinary increment redraws only each numeral cell whose digit changed and the
button that was touched. NEXT additionally repaints only the old and new field
groups whose colour changes, the field label, and the two controls whose labels
change. The title, separators, APPLY and untouched digits remain on the panel.

The Bluetooth application is one decision rather than a settings list: a large
constructed rune, the actual OFF/ADVERTISING/PAIRING/CONNECTED/UPDATING state,
and one full-width ENABLE or DISABLE button. The desired state changes as soon
as the button is released, while the system corner keeps its rune until the BLE
task publishes actual OFF. OFF clears that corner cell and draws no replacement
rune. During DFU the button is disabled, because disconnecting the transport
that is installing the running image is not a valid setting change.

Tiles are 105 x 88 with a ten-pixel gap and a ten-pixel margin, centred in what
the status strip leaves, with the page rail standing upright to their right. The
grid deliberately does not reach the edges of the panel; it used to, at
115 x 106, and being as large as the layout allowed is what made the launcher
feel heavy.

**Pages turn vertically, the same way the launcher was opened.** Up carries on
into the next page and down comes back, and from the first page down is the
watchface again. That is the one place a screen pages along the axis it was
entered on, and it is allowed only because the screen declares each gesture in
advance: it claims the swipe while it has a page that way and hands it back at
the ends, where it becomes the way out. The reason it is worth the exception is
below, under transitions - the panel can only slide its picture vertically, so
this is the axis on which a page turn can be a movement rather than a wipe.

Four to a page rather than six. `InfiniTime` shows six because it has around ten
applications; borrowing its proportions is worth more here than borrowing its
grid.

## The page rail

Every paginated screen reports its position the same way, through one shared
component.

**The rail runs along the axis the screen pages on.** That is the whole content
of the message: a column of marks beside a list that pages up and down says
there is more that way, while the same marks in a row underneath would point
across a movement that never goes across. Every rail in the firmware stands
upright beside its content, because everything pages vertically.

**The showing page is a bar, the rest are squares** - 24 pixels against 6,
6 across, 6 apart. Two treatments rather than one, because the second is
colour, and colour has to be seen accurately at backlight level 1 to carry
anything; the long mark still reads as position when the palette barely does.
The showing mark is `ACCENT` and the rest are `FRAME`.

Squares rather than dots: a circle is a rasteriser the firmware otherwise never
needs, and at four pixels the shapes are indistinguishable.

Exactly one mark is long whichever page shows, so the run keeps its length and
its box. That is what lets the component clear one fixed rectangle before
drawing - without it, a page turn would move the bar within the run and a
partial redraw would leave the tail of its old position standing.

Nothing is drawn for a single page. There is no position to report when there is
only one place to be, and a lone mark would read as a control.

This shape stops working somewhere around ten pages, where the marks are too
many to count and the rail runs out of edge to sit on. No screen is near that,
but it is the boundary at which this component should become something else - a
proportional thumb - rather than be made smaller.

## Progress

One bar, shared by the firmware update and the first-boot format. It is a
filled **track** in `SURFACE` with the completed part in `ACCENT` over it, and
both ends carry the same corner curve the rows do - 200 x 16 at x = 20.

A track rather than an outline around nothing. An empty box says where a bar
would be; a track that is visible the whole way across says how far there is
left to go, which is the only question anybody watching a progress screen has.

The percentage above it is set in the UI face and the accent, and the title
above that in the same face in `TEXT`. There is no explanatory line under the
bar: "keep the watch nearby" told nobody anything they could act on. The format
screen keeps its one line, because "safe to restart" answers the question a
first boot sitting on a progress bar actually raises.

A tile carries a 24 x 24 icon above a centred label. Icons are one-bit bitmaps,
four bytes per row, tinted at draw time so one bitmap serves both the normal and
the pressed colour. Drawing them as geometry would cost more than storing them:
a gear built from arcs needs a rasteriser, while its bitmap costs 96 bytes.

## Music

The one screen that has to set arbitrary text and a value at the same time, so
it is where the rule between the two got written down: **type names things,
segments carry numbers.** The elapsed time is built from the watchface's
numerals, the title and artist are the only text, and there is no second value
set in a face beside them - which is the mixture the FORGE face exists to avoid.

Its progress bar is the shared one and is deliberately not divided into cells.
The steps gauge has five because each is two thousand steps and a glance reads
thousands; a track has no such unit, and four minutes cut into fifths would be
four marks that mean nothing.

The transport is three controls on the row rhythm, in the same column a menu row
and a notification card occupy, with glyphs stepped out of rectangles the way
the FORGE face steps its charging bolt. Volume has no on-screen control: it sits
on the up and down swipes, where `InfiniTime` puts it and where this screen has
gestures to spare, because a launcher tile opened it and right is what leaves.

## Menus

Rows are 200 x 42 at x = 20, set on a 48-pixel rhythm. Under a status strip a
page holds **four** of them; a page that also carries a title holds three. The
paginated list model decides which entries a page shows and which entry a touch
hit, and pages turn vertically because a settings screen is reached by tapping
rather than by a gesture.

A menu is a description rather than drawing code: the rows, the rhythm, the
title and the hint are a `'static` value, and one interpreter draws all of
them. A row states its geometry once, so it cannot disagree with its own hit
box - which is the failure the three-places-per-row arrangement kept producing.

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
the oldest - the finger that opened the screen keeps going through it. Right
dismisses.

**Browsing and dismissing sit on different axes deliberately.** Both are single
swipes and one of them destroys something, so a gesture that lands short or
crooked must not be able to delete the message being read.

No watchface carries a notification count. What is pending is read where it is
read, and a tally on a face would only be a second place to keep it right.

## Transitions

A screen arrives the way the gesture that called for it travelled, and what
that costs depends entirely on the axis.

**Vertically it is a real slide.** Each step composes one 12-row band of the
incoming screen into the rows the panel is not showing, then moves the window
by exactly that band, so the band just written is the strip the move uncovers.
The screen being left is carried off without a single one of its pixels being
sent again, and the whole movement costs one frame - the same 240x240 an
ordinary redraw costs. Every vertical navigation uses it, and so does every
page turn on a vertically paged screen.

The price is that the window does not return to where it started: after a slide
screen row 0 is some other memory row, for good. The translation lives in the
display task's panel wrapper and nothing above it knows, which is the only
reason this is affordable at all.

**Horizontally nothing can move**, because the register only scrolls one way.
The incoming screen is revealed strip by strip from the edge it enters through,
which reads as arriving from the right side without anything sliding.

## What this does not decide

- **Animation timing.** Transitions run as fast as SPI allows, with no easing,
  because that would need a timer inside the transition. A design must not
  depend on motion curves before that exists.
- **Full-screen artwork.** It would live in external flash, which shares the
  SPI bus with the display, so it is read once per screen change and never per
  redraw. Fonts and symbols stay in internal flash for exactly that reason.
- **Watchface looks.** Faces are free to differ completely; this document
  governs the system around them.
