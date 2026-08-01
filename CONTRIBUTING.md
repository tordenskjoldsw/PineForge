# Contributing to PineForge

Thanks for looking. PineForge runs a PineTime as an everyday watch, on hardware
that is awkward to recover when something goes wrong - so this document is
mostly about what that second part costs.

## Support expectation

Best effort, by one person, in the evenings around a day job. Issues and pull
requests are read and answered as fast as I can manage, which is usually quick.
That is an intention rather than a commitment: some weeks there is no time at
all.

PineForge is past being a proof of concept. It has been the firmware on my own
PineTime since late July 2026, and the everyday paths hold up - the time, the
watchface, settings that survive a reboot, notifications, pairing, OTA.

What it is not is broadly proven. Every claim above rests on one watch, one
bootloader version, one phone. Nobody has run it for a month, measured its
battery life, or tried it on a second unit. And two features are further along
in the menu than in the evidence: **step counting and heart-rate measurement
have never been compared against a reference**, so read both as indications
rather than as measurements. See
[`docs/TESTED-CONFIGURATIONS.md`](docs/TESTED-CONFIGURATIONS.md) for what the
evidence actually covers.

So: worth using, worth reporting bugs against, and still not something to put on
the only watch you own without keeping a way back.

## Before you flash anything

Read [`GETTING-STARTED.md`](GETTING-STARTED.md) and
[`docs/SEALED-PINETIME-TESTING.md`](docs/SEALED-PINETIME-TESTING.md) first. A
sealed PineTime has no exposed SWD pads: if an image neither boots nor reaches
the recovery path, the remaining option is to let the battery run flat. Keep a
known-good official InfiniTime DFU package on your phone before you start, and
leave a new image unconfirmed until you have used it.

## Setting up

```bash
./scripts/setup-build-tools.sh
```

This installs the pinned toolchain, `flip-link`, `cargo-binutils`, a pinned
MCUBoot checkout for `imgtool`, and a `.venv` with pinned Python tooling. It is
safe to re-run.

## Debugging, and why so much is host-tested

A sealed PineTime exposes no SWD pads. Until now the only way to get code onto
mine has been an OTA update, and the only way to learn what it did has been to
watch the screen - no breakpoints, no logs, one attempt per DFU transfer.

That constraint is why so much of this codebase is arranged the way it is.
Product policy lives in `pineforge-state` and screens render into any surface,
so both can be tested on a laptop in milliseconds. It was not primarily an
aesthetic choice; it was the only fast feedback available.

The firmware is already wired for a probe: `.cargo/config.toml` sets a
`probe-rs run --chip nRF52832_xxAA` runner, and `defmt` logs over RTT with
`DEFMT_LOG=info`. None of it is reachable on a sealed watch.

A second PineTime has been ordered so the first can be opened, giving SWD and
therefore RTT. No result from that second unit or from an SWD capture is recorded
yet. Some questions that have been left deliberately open are waiting on exactly
that - the input task logs every touch report at `info!`, and what the CST816S
puts in its coordinate registers on the report that ends a touch has never been
measured. Expect the host-side testing to stay even once a probe is available:
it is quicker than a hardware capture.

## What CI will check

Run these before opening a pull request; they are the same gates CI applies.

```bash
cargo fmt --all -- --check
./scripts/check-layers.sh
cargo check --release
cargo check --release --features diagnostics
cargo clippy --release -- -D warnings
cargo clippy --release --features diagnostics -- -D warnings
cargo test --locked --manifest-path crates/pineforge-state/Cargo.toml --target x86_64-unknown-linux-gnu
cargo test --locked --manifest-path crates/pineforge-state/Cargo.toml --target x86_64-unknown-linux-gnu --features diagnostics
cargo test --locked --manifest-path crates/pineforge-ui/Cargo.toml --target x86_64-unknown-linux-gnu
cargo check --locked --manifest-path crates/pineforge-ui/Cargo.toml --release --features ui-animations,diagnostics
```

CI additionally enforces flash and RAM budgets. These are design targets rather
than the hardware ceiling, and they are meant to be argued with: if a change
genuinely needs the space, say so in the pull request and change the budget in
the same commit, with the reasoning. What is not acceptable is quiet growth.

## Architecture

[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) is worth reading before a first
change. The layering it describes is enforced by `scripts/check-layers.sh`, not
by convention:

- `board` owns pins and buses;
- `drivers` talk to one chip each and know nothing about the product;
- `services` turn readings into meaning;
- `tasks` own a peripheral apiece and communicate only through `ipc`;
- `crates/pineforge-state` holds product policy and has no hardware dependency;
- `crates/pineforge-ui` renders state onto any surface.

Two consequences worth stating plainly, because they are the mistakes that
happen most often:

- **Product decisions belong in `pineforge-state`**, where they can be tested on
  the host. If a behaviour can only be checked by wearing the watch, it is in
  the wrong layer.
- **Input reaches only the active screen, but readings reach the shared state
  first.** A screen that ingests a reading only when it happens to be visible
  loses it.

## Commits and pull requests

- One logical change per commit; a commit that both moves code and changes what
  it does is hard to review and harder to bisect.
- Conventional-commit subjects (`feat(ui): …`, `fix(dfu): …`) matching the
  existing history.
- Explain *why* in the body. The code says what.
- Say in the pull request whether the change was tested on hardware, and on
  which setup - see
  [`docs/TESTED-CONFIGURATIONS.md`](docs/TESTED-CONFIGURATIONS.md). "Not tested
  on hardware" is a perfectly good answer; a wrong claim is not.

## Reporting a problem

Include the PineForge version and commit shown on the About screen, the watch
and bootloader versions, and the phone and Gadgetbridge versions. A report
naming its configuration is worth several that do not.

## Licence

Contributions are accepted under the same dual MIT / Apache-2.0 licence as the
project. By opening a pull request you agree your work may be distributed under
both.
