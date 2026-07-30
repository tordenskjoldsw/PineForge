# Contributing to PineForge

Thanks for looking. PineForge is experimental firmware for a device that can be
difficult to recover, so this document is mostly about what that costs.

## Support expectation

Best effort, by one person, on an experimental project. Issues and pull requests
are read and appreciated. There is no response-time commitment, and no support
for running PineForge as the only firmware on a watch you depend on.

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

## What CI will check

Run these before opening a pull request; they are the same gates CI applies.

```bash
cargo fmt --all -- --check
./scripts/check-layers.sh
cargo clippy --release -- -D warnings
cargo clippy --release --features diagnostics -- -D warnings
cargo test --manifest-path crates/pineforge-state/Cargo.toml --target x86_64-unknown-linux-gnu
cargo test --manifest-path crates/pineforge-ui/Cargo.toml --target x86_64-unknown-linux-gnu
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
  which setup — see
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
