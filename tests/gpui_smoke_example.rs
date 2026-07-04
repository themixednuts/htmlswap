use std::path::Path;
use std::process::Command;

#[test]
fn gpui_smoke_example_crate_checks() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("gpui_smoke")
        .join("Cargo.toml");

    let output = Command::new(env!("CARGO"))
        .arg("check")
        .arg("--manifest-path")
        .arg(manifest)
        .arg("--quiet")
        .output()
        .expect("cargo check should start");

    assert!(
        output.status.success(),
        "gpui smoke example failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
