use std::process::Command;

#[test]
fn test_cli_help() {
    // Test that the CLI can display help
    let output = Command::new("cargo")
        .args(["run", "-p", "oneamp-cli", "--", "--help"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success(), "Help command should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("OneAmp"), "Help should mention OneAmp");
    assert!(stdout.contains("Usage"), "Help should show usage");
}

#[test]
fn test_cli_missing_file() {
    // Test that the CLI handles missing files gracefully
    let output = Command::new("cargo")
        .args(["run", "-p", "oneamp-cli", "--", "nonexistent.mp3"])
        .output()
        .expect("Failed to execute command");

    // Should fail gracefully
    assert!(!output.status.success(), "Should fail for nonexistent file");

    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should contain some error message (either from our code or from file not found)
    assert!(
        !stderr.is_empty() || !output.stdout.is_empty(),
        "Should output error message"
    );
}
