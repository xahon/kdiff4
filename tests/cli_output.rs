use std::process::Command;


#[test]
fn version_reaches_piped_stdout() {
    let output = Command::new(env!("CARGO_BIN_EXE_kdiff4")).arg("--version").output().expect("Process: cannot run kdiff4");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), format!("kdiff4 {}", env!("CARGO_PKG_VERSION")));
}


#[test]
fn usage_error_reaches_piped_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_kdiff4")).args(["a", "b", "c", "d"]).output().expect("Process: cannot run kdiff4");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("too many input files"));
}
