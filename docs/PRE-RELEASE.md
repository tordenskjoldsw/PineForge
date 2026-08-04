# Pre-Release Checklist

What CI cannot see, and therefore what a person has to do before an image is
given to anyone.

This exists because three regressions reached hardware in one release while
every automated gate stayed green: twice a static-RAM change put the stack under
what pairing needs, and once a test harness raced only on a machine that
scheduled its threads differently. None of them were subtle in hindsight. All of
them were invisible to `cargo`.

Every line below is here because something went wrong that it would have caught.
Nothing is here for completeness.

## Before flashing

- [ ] `cargo fmt --all -- --check`
- [ ] `./scripts/check-layers.sh`
- [ ] `cargo clippy --release -- -D warnings`, and again with `--features diagnostics`
- [ ] Every host suite, both feature sets: `pineforge-state`, `pineforge-ui`, `pineforge-services`
- [ ] `./scripts/check-size.sh` for both budgets
- [ ] The tree is clean, so the image is not stamped `-dirty`

CI runs all of this. Running it locally first is not the point of this file; the
point is that passing it proves less than it looks like.

## On the watch

### Pairing, from nothing

- [ ] Remove the watch in the **phone's Bluetooth settings**, not only in the companion app
- [ ] Flash the image
- [ ] Pair, and confirm the passkey is shown and accepted

This is the line that matters most. A reconnection is not a pairing: the bond
lives in external flash and survives a DFU, so a phone that merely reconnects
never runs the key exchange - which is both the deepest stack path the firmware
has and the one two regressions broke. It is also what forces the phone to
re-read the attribute table, so a newly added GATT service is invisible without
it.

If static RAM moved at all since the last release, read the stack mark as well:
flash the diagnostics build instead, pair the same way, then open the **TOUCH**
tile and note `STACK used / total`. The reserve in `.github/workflows/ci.yml` is
derived from that number.

### The services a companion uses

- [ ] A notification arrives, **with the sender's name on it**
- [ ] The time syncs from the phone
- [ ] Music shows the current track and the transport controls work
- [ ] Take a heart-rate reading in the pulse application and watch it reach the companion
- [ ] Walk far enough for the step count to move **twice** - a companion discards the first sample of each day

The step and heart-rate lines are new and both were broken in ways that looked
like the other end's fault. Neither is proven by the watch's own screen showing a
number.

### The screens themselves

- [ ] Open the pulse application and leave it a minute: the number updates without the screen flashing
- [ ] Open the stopwatch and leave it running a minute
- [ ] Page through the launcher in both directions and return to the watchface
- [ ] Let the watch sleep and wake it, and confirm the first gesture after waking is not swallowed

A screen that repaints in full where it should repaint in part costs battery and
looks wrong, and nothing automated notices - the host tests check that a repaint
is *complete*, not that it is *small*.

### Recovery, if anything was touched near the boot path

- [ ] The image is unconfirmed on first boot, and a side-button reset returns to the previous firmware
- [ ] Confirming the image on the FIRMWARE screen clears the status corner's mark

## After

- [ ] Add the verified paths to `docs/TESTED-CONFIGURATIONS.md`, and be as specific about what was *not* tried
- [ ] Write the release notes, and name the commit the image was built from

The evidence file is what decides what this project may claim. Filling it in
afterwards, honestly, is the only thing that keeps the claims and the reality
from drifting apart.
