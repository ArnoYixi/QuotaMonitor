use sha2::{Digest, Sha256};
use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=QUOTA_BUNDLE_RUNTIME");
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let target = env::var("TARGET").unwrap();
    let (platform, digest) = if target.contains("windows") {
        (
            "x86_64-pc-windows-msvc",
            "402a36449947ffbdd137b8a6b372df5b3f220ab0fb5c52b1f032f801cd7caed2",
        )
    } else {
        (
            "x86_64-unknown-linux-musl",
            "41153de39d4f4ea8f747cea6fbd36dc3aefb3b3a54b0dfffac9ac73c08237e5d",
        )
    };
    let archive = root.join(format!(
        "vendor/codex-app-server-package-{platform}.tar.zst"
    ));
    println!("cargo:rerun-if-changed={}", archive.display());
    let bundle = env::var("QUOTA_BUNDLE_RUNTIME").as_deref() == Ok("1");
    let bytes = if bundle {
        assert!(
            target.starts_with("x86_64-"),
            "The bundled runtime is x86_64 only"
        );
        assert!(
            archive.is_file(),
            "Run python scripts/fetch_runtime.py first"
        );
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(fs::read(&archive).expect("Read runtime package"))
            ),
            digest,
            "Runtime SHA-256 mismatch; run scripts/fetch_runtime.py to repair"
        );
        format!("include_bytes!({:?})", archive)
    } else {
        "&[]".into()
    };
    let source = format!(
        "pub const RUNTIME_ARCHIVE: &[u8] = {bytes};\npub const RUNTIME_SHA256: &str = {digest:?};\npub const RUNTIME_VERSION: &str = \"0.162.1\";\n"
    );
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("bundled.rs"),
        source,
    )
    .unwrap();
}
