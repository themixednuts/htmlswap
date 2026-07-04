use std::path::Path;
use std::process::Command;

#[test]
fn gpui_base_smoke_example_crate_checks() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("gpui_base_smoke")
        .join("Cargo.toml");
    let target_dir = std::env::temp_dir().join(format!(
        "htmlswap-gpui-base-smoke-target-{}",
        std::process::id()
    ));

    let output = Command::new(env!("CARGO"))
        .arg("check")
        .arg("--manifest-path")
        .arg(manifest)
        .arg("--quiet")
        .env("CARGO_TARGET_DIR", &target_dir)
        .output()
        .expect("cargo check should start");
    let _ = std::fs::remove_dir_all(&target_dir);

    assert!(
        output.status.success(),
        "gpui base smoke example failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
