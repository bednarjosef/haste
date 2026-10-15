//! Settle smoke over a feature-check script (dev helper): runs the same client command
//! `scripts/checks/run-feature-checks.sh` runs (`haste --script scripts/checks/<name>.txt`
//! against the shared test server, default game dir) and fails when the script's
//! `wait_until settled` times out — the check scripts themselves are the evidence, this
//! keeps them runnable from cargo where the shell script is not executable.
//!
//! Not part of the PR's proof: `scripts/checks/run-feature-checks.sh language pause` is;
//! this exists for local iteration.

use std::process::{Command, Stdio};

#[test]
#[ignore = "needs the local test server on 25691 and a built target/debug/haste"]
fn feature_check_script_settles() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root")
        .to_path_buf();
    // language.txt covers the whole flow (options screens, the language switch); pause.txt
    // is the same start with less UI.
    let script = root.join("scripts/checks/language.txt");
    let out = Command::new(root.join("target/debug/haste"))
        .arg("--server")
        .arg("127.0.0.1:25691")
        .arg("--username")
        .arg("ScriptCheck")
        .arg("--script")
        .arg(&script)
        .arg("--seconds")
        .arg("120")
        .current_dir(&root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run the client");
    let log = String::from_utf8_lossy(&out.stderr);
    println!("exit: {:?}", out.status);
    for line in log
        .lines()
        .filter(|l| l.contains("Error") || l.contains("script screenshot"))
    {
        println!("log: {line}");
    }
    let last_error = log
        .lines()
        .filter(|l| l.contains("Error"))
        .fold("no error line", |_, l| l);
    assert!(
        out.status.success(),
        "the feature-check script must finish: {last_error}"
    );
}
