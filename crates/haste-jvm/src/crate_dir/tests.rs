use std::fs;
use std::path::{Path, PathBuf};

use super::jvm_crate_dir;

/// A fresh fake checkout under the system temp dir: `<tmp>/<name>` with the given crate files.
fn fake_root(name: &str, with_conf: bool) -> PathBuf {
    let root = std::env::temp_dir().join(format!("haste-crate-dir-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let crate_dir = root.join("crates").join("haste-jvm");
    fs::create_dir_all(&crate_dir).unwrap();
    if with_conf {
        fs::write(crate_dir.join("proxies.conf"), "").unwrap();
    }
    root
}

#[test]
fn walks_up_from_the_executable_to_its_checkout() {
    let root = fake_root("walk-up", true);
    let exe = root.join("target").join("release").join("haste");
    let compiled = Path::new("/nonexistent/compiled/haste-jvm");
    let found = jvm_crate_dir(None, Some(&exe), compiled).unwrap();
    assert_eq!(found, root.join("crates").join("haste-jvm"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn haste_root_wins_when_set() {
    let exe_root = fake_root("root-exe", true);
    let env_root = fake_root("root-env", true);
    let exe = exe_root.join("target").join("release").join("haste");
    let compiled = Path::new("/nonexistent/compiled/haste-jvm");
    let found = jvm_crate_dir(Some(env_root.as_os_str()), Some(&exe), compiled).unwrap();
    assert_eq!(found, env_root.join("crates").join("haste-jvm"));
    let _ = fs::remove_dir_all(&exe_root);
    let _ = fs::remove_dir_all(&env_root);
}

#[test]
fn haste_root_without_proxies_conf_falls_through() {
    let env_root = fake_root("root-missing", false);
    let exe_root = fake_root("fall-exe", true);
    let exe = exe_root.join("target").join("release").join("haste");
    let compiled = Path::new("/nonexistent/compiled/haste-jvm");
    let found = jvm_crate_dir(Some(env_root.as_os_str()), Some(&exe), compiled).unwrap();
    assert_eq!(found, exe_root.join("crates").join("haste-jvm"));
    let _ = fs::remove_dir_all(&env_root);
    let _ = fs::remove_dir_all(&exe_root);
}

#[test]
fn layout_without_proxies_conf_falls_through_to_compiled_dir() {
    let root = fake_root("no-conf", false);
    let exe = root.join("target").join("release").join("haste");
    let compiled = Path::new("/nonexistent/compiled/haste-jvm");
    let err = jvm_crate_dir(None, Some(&exe), compiled).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("proxies.conf"), "{message}");
    assert!(
        message.contains("/nonexistent/compiled/haste-jvm/proxies.conf"),
        "{message}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn compiled_dir_is_the_last_resort() {
    let compiled = fake_root("compiled", true).join("crates").join("haste-jvm");
    let exe = Path::new("/nonexistent/target/release/haste");
    let found = jvm_crate_dir(None, Some(exe), &compiled).unwrap();
    assert_eq!(found, compiled);
    let _ = fs::remove_dir_all(compiled.parent().unwrap().parent().unwrap());
}
