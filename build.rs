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

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    fs::write(out.join("memory.x"), include_bytes!("memory.x"))
        .expect("write memory.x into OUT_DIR");
    decode_config(&out, "bma421");
    decode_config(&out, "bma425");
    publish_bond_schema();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=build.rs");
}
