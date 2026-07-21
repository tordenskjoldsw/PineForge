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

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    fs::write(out.join("memory.x"), include_bytes!("memory.x"))
        .expect("write memory.x into OUT_DIR");
    decode_config(&out, "bma421");
    decode_config(&out, "bma425");
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=build.rs");
}
