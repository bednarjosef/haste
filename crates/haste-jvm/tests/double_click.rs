//! Double-click rule self-test (ignored: needs cache/).
use std::path::PathBuf;

use haste_jvm::JvmConfig;

fn cache() -> PathBuf {
    let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cache");
    if local.join("versions").is_dir() {
        local
    } else {
        haste_core::paths::cache_dir()
    }
}

#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn double_click_rule() {
    let h = haste_jvm::JvmHost::start(JvmConfig::from_cache(&cache()).expect("cache layout"))
        .expect("JVM start");
    let ok: bool = h
        .with_env(|env| {
            let cls = h.load_class(env, "haste.gui.Bridge")?;
            let v = env
                .call_static_method(
                    &cls,
                    jni::jni_str!("doubleClickRuleSelfTest"),
                    jni::jni_sig!("()Z"),
                    &[],
                )?
                .z()?;
            Ok(v)
        })
        .unwrap();
    assert!(ok, "double-click rule self-test failed");
}
