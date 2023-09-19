use std::process::Command;

#[test]
fn build_no_std_macros() {
    // Ensure that the macro crate and the macro-expanded code can work in a `no_std` environment.
    let status = Command::new("cargo")
        .args(["build", "-p", "test-no-std-macros"])
        .status()
        .unwrap();
    assert!(status.success());
}
