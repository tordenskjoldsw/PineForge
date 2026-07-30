# Security Policy

## Scope and status

PineForge is experimental firmware and has not had a security review. It is not
suitable for any use where a compromise would matter. In particular:

- **Firmware images are not signed or verified by PineForge.** The MCUBoot
  header written by `imgtool` carries no signature in this configuration, and
  the stock PineTime bootloader does not verify one. Anything able to complete a
  DFU transfer to the watch can replace the firmware.
- **The DFU characteristics require no encryption or bonding.** Matching
  InfiniTime, they are plain writable characteristics. The only gate on starting
  a transfer is that the running image must already be confirmed — a safety
  interlock protecting the rollback image, not an access control.
- **BLE pairing uses passkey entry**, which protects against passive
  eavesdropping and casual impostors, not against a determined local attacker.
  Nothing else on the watch requires a bond either.
- Persisted settings and bonds are stored in external flash **without
  encryption**. Anyone with physical access to the flash can read them.

These are properties of the design as it stands, documented so that nobody
assumes otherwise. They are not treated as vulnerabilities.

## Supported versions

Only the latest release receives fixes. Older tags are kept as historical
baselines and are not updated.

| Version | Supported |
|---|---|
| Latest release | Yes, best effort |
| Earlier releases | No |

## Reporting a vulnerability

Report anything that breaks the model above — rather than merely describing it —
privately, through GitHub's **Report a vulnerability** button on the Security tab
of this repository. Please do not open a public issue first.

Useful things to include: the affected version and commit, what an attacker must
already have (proximity, a bond, physical access), what they gain, and how to
reproduce it.

Expect a best-effort reply. This is a one-person hobby project; there is no
bounty and no guaranteed response time. Findings that require an attacker to
already be able to flash the watch are usually not separately exploitable,
because that capability is already total.
