# What this changes

<!-- What it does, and why. The diff already says how. -->

## Hardware testing

<!--
Delete whichever does not apply. "Not tested on hardware" is a perfectly
acceptable answer; an inaccurate claim is not.
-->

- [ ] Tested on hardware. Setup:
      <!-- watch, bootloader version, phone, Gadgetbridge version -->
- [ ] Not tested on hardware — host tests and CI only.

## Checks

- [ ] `cargo fmt --all -- --check`
- [ ] `./scripts/check-layers.sh`
- [ ] `cargo clippy --release -- -D warnings`, production and `diagnostics`
- [ ] Host tests pass for `pineforge-state` and `pineforge-ui`
- [ ] Behaviour that could be tested on the host is tested on the host

## Budgets

<!--
Delete if flash and static RAM are unaffected. If a budget in
.github/workflows/ci.yml changed, it must change in this pull request, with the
reason — quiet growth is the thing the budgets exist to catch.
-->

- [ ] Flash and static RAM are unaffected, or the budget change is justified below.

## Safety

<!-- Delete any that do not apply to this change. -->

- [ ] Does not alter the flash map, MCUBoot header, or DFU protocol.
- [ ] Does not change the recovery path — side-button reset, rollback of an
      unconfirmed image, or the bootloader recovery entry.
- [ ] Does not change a persisted record format. If it does, the format version
      was bumped and a migration test added.
