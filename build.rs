use std::{env, fs, path::PathBuf};

fn decode_hex(input: &str) -> Vec<u8> {
    let input = input.trim();
    assert!(input.len().is_multiple_of(2), "hex stream has odd length");
    input
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("hex stream is UTF-8");
            u8::from_str_radix(text, 16).expect("hex stream contains only hexadecimal bytes")
        })
        .collect()
}

fn decode_config(out: &std::path::Path, variant: &str) {
    let source = format!("third_party/bosch-bma42x/{variant}.hex");
    let bytes = decode_hex(&fs::read_to_string(&source).expect("read BMA42x config stream"));
    assert_eq!(bytes.len(), 6_144, "unexpected BMA42x config size");
    fs::write(out.join(format!("{variant}.bin")), bytes).expect("write BMA42x config stream");
    println!("cargo:rerun-if-changed={source}");
}

/// Publishes where the primary image slot ends, taken from `memory.x` rather
/// than written out a second time.
///
/// `src/boot/confirm.rs` writes one word of internal flash - the only such
/// write in the firmware - and it locates that word from the end of the slot.
/// The address had been a literal in that file and a row in
/// `docs/FLASH-MAP.md`, agreeing with `memory.x` only because nobody had moved
/// the layout yet. On a sealed watch the failure that would follow is not a
/// wrong pixel: it is NVMC clearing bits somewhere else in internal flash.
///
/// So the slot end comes from the linker script, and `confirm.rs` asserts the
/// address it derives is still the documented one. Moving `memory.x` now fails
/// the build instead of the watch.
fn publish_flash_map(out: &std::path::Path) {
    let script = fs::read_to_string("memory.x").expect("read memory.x");
    let flash = script
        .lines()
        .find_map(|line| line.trim().strip_prefix("FLASH"))
        .map(|rest| rest.trim().trim_start_matches(':').trim())
        .expect("memory.x declares a FLASH region");

    let field = |name: &str| -> u32 {
        let value = flash
            .split(',')
            .find_map(|part| part.trim().strip_prefix(name))
            .unwrap_or_else(|| panic!("FLASH region declares {name}"))
            .trim()
            .trim_start_matches('=')
            .trim();
        let digits = value
            .strip_prefix("0x")
            .unwrap_or_else(|| panic!("FLASH {name} {value:?} is hexadecimal"));
        u32::from_str_radix(digits, 16)
            .unwrap_or_else(|_| panic!("FLASH {name} {value:?} is a number"))
    };

    // The image region starts after imgtool's 32-byte header and runs to the
    // end of the slot, so its far end is the slot's.
    let slot_end = field("ORIGIN") + field("LENGTH");
    // Grouped the way the firmware writes its addresses, because clippy's
    // pedantic set reads generated code too.
    let digits = format!("{slot_end:08X}");
    let (high, low) = digits.split_at(4);
    fs::write(
        out.join("flash_map.rs"),
        format!("const PRIMARY_SLOT_END: usize = 0x{high}_{low};\n"),
    )
    .expect("write flash_map.rs into OUT_DIR");
}

/// Publishes the locked version of the crate that owns the serialized BLE bond
/// layout, so the persisted record can name the layout it was written with.
///
/// This is read from the lockfile rather than written by hand on purpose: the
/// tag has to change when the dependency changes, and a constant somebody must
/// remember to bump is exactly the constant that does not get bumped. A payload
/// read back under a different tag falls back to re-pairing instead of
/// installing keys decoded from a layout that no longer applies.
fn publish_bond_schema() {
    const CRATE: &str = "trouble-host";

    println!("cargo:rerun-if-changed=Cargo.lock");
    let lock = fs::read_to_string("Cargo.lock").expect("read Cargo.lock");
    let version = lock
        .split("[[package]]")
        .find_map(|package| {
            let mut name = None;
            let mut version = None;
            for line in package.lines() {
                if let Some(value) = line.strip_prefix("name = ") {
                    name = Some(value.trim_matches('"'));
                } else if let Some(value) = line.strip_prefix("version = ") {
                    version = Some(value.trim_matches('"'));
                }
            }
            (name == Some(CRATE)).then_some(version).flatten()
        })
        .unwrap_or_else(|| panic!("{CRATE} is missing from Cargo.lock"));

    // The tag is fixed-width in flash; a version string that cannot fit would
    // silently lose its tail and stop distinguishing layouts.
    assert!(
        !version.is_empty() && version.len() <= 16,
        "{CRATE} version {version:?} does not fit the bond schema tag"
    );
    println!("cargo:rustc-env=PINEFORGE_BOND_SCHEMA={version}");
}

/// Publishes what this build is, so the watch can say so on its own screen.
///
/// None of it is derivable from the manifest. The package version there is
/// `0.2.1` for every build ever cut from it - the number that actually
/// identifies one, `0.2.1+20`, is an argument to `build-dfu.sh` and had never
/// reached the firmware at all. And a commit is not in the manifest by nature.
///
/// Both arrive through the environment, which `build-dfu.sh` sets, so a
/// packaged image is exact by construction. A plain `cargo build` has neither,
/// and falls back to asking git directly and then to saying it does not know -
/// which is the honest answer and better than a confident wrong one.
fn publish_build_identity() {
    for key in ["PINEFORGE_VERSION", "PINEFORGE_COMMIT", "PINEFORGE_DATE"] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    // Covers the ordinary case of committing and rebuilding without the script.
    println!("cargo:rerun-if-changed=.git/HEAD");

    let version = env::var("PINEFORGE_VERSION")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| env::var("CARGO_PKG_VERSION").ok())
        .unwrap_or_else(|| "unknown".into());
    // The firmware screen sets this in the 10-pixel face after a nine-character
    // prefix, and the device information service stores it in sixteen bytes.
    assert!(
        version.len() <= 12,
        "version {version:?} is too long to show on the watch"
    );
    println!("cargo:rustc-env=PINEFORGE_VERSION={version}");
    publish_release_triple(&version);

    let commit = env::var("PINEFORGE_COMMIT")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let head = git(&["rev-parse", "--short=7", "HEAD"])?;
            // A commit id that names a commit the binary was not built from is
            // worse than no id, so an unclean tree says so. Only as good as the
            // rerun triggers above, mind: an edit made after a build does not
            // bring the build script back, so a development binary can carry a
            // clean id it no longer deserves. `build-dfu.sh` sets this in the
            // environment instead, which does force a rebuild - so the answer
            // is exact for anything actually packaged.
            let dirty = git(&["status", "--porcelain"]).is_some();
            Some(if dirty { format!("{head}-dirty") } else { head })
        })
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=PINEFORGE_COMMIT={commit}");

    // The commit's date rather than the moment of the build: it answers the
    // same question - how old is what I am running - and gives the same answer
    // every time this commit is built, which a wall clock would not.
    let date = env::var("PINEFORGE_DATE")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| git(&["log", "-1", "--format=%cs"]))
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=PINEFORGE_DATE={date}");
}

/// Publishes the release as exactly `major.minor.patch`, for the one consumer
/// that cannot read anything else.
///
/// The Device Information Service reports a firmware revision, and a companion
/// parses it. Gadgetbridge's does it by splitting on `.`, and if it gets three
/// parts it calls `Integer.parseInt` on each - with no try around it. So the
/// `0.2.1+7` form that distinguishes two packages of one release becomes
/// `["0", "2", "1+7"]`, the third parse throws, and the version event that
/// registers the watch in its database is never handled. What that looks like
/// from the outside is a device whose `deviceId` comes back null, and every
/// activity sample failing to save.
///
/// So the build metadata stays out of this one string. It is still what the
/// About screen shows and what names the package, because there it identifies
/// the build; here it only has to parse.
fn publish_release_triple(version: &str) {
    let release = version.split('+').next().unwrap_or_default();
    let mut parts: Vec<&str> = release.split('.').collect();
    // A release given as `0.6` is still a release; the missing component is
    // zero, which is what a parser expecting three of them needs to see.
    while parts.len() < 3 {
        parts.push("0");
    }
    let triple = parts[..3].join(".");
    assert!(
        triple
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())),
        "firmware revision {triple:?} is not major.minor.patch; a companion will fail to parse it"
    );
    guard_companion_feature_gates(&triple);
    println!("cargo:rustc-env=PINEFORGE_RELEASE={triple}");
}

/// Refuses a release number that would switch on companion behaviour this
/// firmware cannot answer.
///
/// The version reported over BLE is not only a label. Gadgetbridge reads it as
/// a capability statement and changes what it sends:
///
/// - below `0.15` a notification arrives as the body alone, with the sender
///   discarded before it leaves the phone
/// - from `0.15` it arrives as `title NUL body`, which is the shape this
///   firmware's parser has always split on, and the reason the version is where
///   it is
/// - from `1.8` weather packets begin, CBOR-framed up to `1.13` and a simple
///   binary form from `1.14`
///
/// `PineForge` implements neither weather form. Crossing `1.8` would therefore
/// have a phone sending packets to a characteristic that does not exist, which
/// is the kind of thing that is discovered on a wrist rather than in CI. So it
/// is discovered here instead, and the way past it is to implement weather and
/// then relax this - deliberately, not by releasing 1.8 one day.
fn guard_companion_feature_gates(triple: &str) {
    let mut parts = triple
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0));
    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);

    assert!(
        major > 0 || minor >= 15,
        "release {triple} is below 0.15, where a companion still sends notifications \
         without their sender - see guard_companion_feature_gates"
    );
    assert!(
        (major, minor) < (1, 8),
        "release {triple} is at or past 1.8, where Gadgetbridge starts sending weather \
         this firmware does not implement - see guard_companion_feature_gates"
    );
}

/// Runs git and returns its trimmed output, or `None` if it did not work.
///
/// Every failure is the same failure here - no git, no repository, a source
/// tarball - and all of them mean the same thing: this build cannot name its
/// commit.
fn git(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    fs::write(out.join("memory.x"), include_bytes!("memory.x"))
        .expect("write memory.x into OUT_DIR");
    decode_config(&out, "bma421");
    decode_config(&out, "bma425");
    publish_flash_map(&out);
    publish_bond_schema();
    publish_build_identity();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=build.rs");
}
