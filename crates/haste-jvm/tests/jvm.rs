//! JVM tests. They need the cache (JDK, client jar, libraries, ASM) and are ignored by default:
//! `cargo test -p haste-jvm -- --ignored --test-threads=1`.
use std::path::PathBuf;
use std::time::Instant;

use haste_jvm::{ClassSource, JvmConfig, JvmHost};
use jni::objects::{JObject, JString, JValue};
use jni::{jni_sig, jni_str};

fn cache() -> PathBuf {
    let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cache");
    if local.join("versions").is_dir() { local } else { haste_core::paths::cache_dir() }
}

fn host() -> JvmHost {
    let t = Instant::now();
    let h = JvmHost::start(JvmConfig::from_cache(&cache()).expect("cache layout")).expect("JVM start");
    eprintln!("host ready in {:?} ({:?}); {}", t.elapsed(), h.timings(), h.proxy_summary());
    h
}

#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM)"]
fn tier3_value_types_from_the_jar() {
    let h = host();
    let t = Instant::now();
    let (id, pos, hi) = h
        .with_env(|env| {
            let idc = h.load_class(env, "net.minecraft.resources.Identifier")?;
            let s = JString::from_str(env, "minecraft:stone")?;
            let id = env
                .call_static_method(&idc, jni_str!("parse"), jni_sig!("(Ljava/lang/String;)Lnet/minecraft/resources/Identifier;"), &[JValue::Object(&s)])?
                .l()?;
            let id = to_string(env, &id)?;
            let first = t.elapsed();

            let bp = h.load_class(env, "net.minecraft.core.BlockPos")?;
            let p = env.new_object(&bp, jni_sig!("(III)V"), &[JValue::Int(1), JValue::Int(2), JValue::Int(3)])?;
            let q = env
                .call_method(&p, jni_str!("offset"), jni_sig!("(III)Lnet/minecraft/core/BlockPos;"), &[JValue::Int(4), JValue::Int(5), JValue::Int(6)])?
                .l()?;
            let x = env.call_method(&q, jni_str!("getX"), jni_sig!("()I"), &[])?.i()?;
            let y = env.call_method(&q, jni_str!("getY"), jni_sig!("()I"), &[])?.i()?;
            let z = env.call_method(&q, jni_str!("getZ"), jni_sig!("()I"), &[])?.i()?;

            let comp = h.load_class(env, "net.minecraft.network.chat.Component")?;
            let s = JString::from_str(env, "hi")?;
            let c = env
                .call_static_method(&comp, jni_str!("literal"), jni_sig!("(Ljava/lang/String;)Lnet/minecraft/network/chat/MutableComponent;"), &[JValue::Object(&s)])?
                .l()?;
            let got = env.call_method(&c, jni_str!("getString"), jni_sig!("()Ljava/lang/String;"), &[])?.l()?;
            let got = to_string(env, &got)?;
            eprintln!("first call (Identifier.parse incl. class loading): {first:?}; all three: {:?}", t.elapsed());
            Ok((id, (x, y, z), got))
        })
        .expect("calls");
    assert_eq!(id, "minecraft:stone");
    assert_eq!(pos, (5, 7, 9));
    assert_eq!(hi, "hi");

    let log = h.class_log().unwrap();
    assert_eq!(log.classes.get("net.minecraft.resources.Identifier"), Some(&ClassSource::Jar));
    assert_eq!(log.classes.get("net.minecraft.core.BlockPos"), Some(&ClassSource::Jar));
    // tier-1 types never come from the jar
    let refused = h.with_env(|env| Ok(h.load_class(env, "net.minecraft.world.level.Level").is_err())).unwrap();
    assert!(refused, "Level must be refused");
    let log = h.class_log().unwrap();
    assert!(matches!(log.classes.get("net.minecraft.world.level.Level"), Some(ClassSource::Refused(_))));
    eprintln!("class log: {} jar, {} proxy, {} lib, {} refused",
        log.with(|s| *s == ClassSource::Jar).len(), log.with(|s| *s == ClassSource::Proxy).len(),
        log.with(|s| *s == ClassSource::Library).len(), log.refused().len());
    for (n, why) in log.refused() {
        eprintln!("  refused {n}: {why}");
    }
}

fn to_string(env: &mut jni::Env, o: &JObject) -> jni::errors::Result<String> {
    let s = env.call_method(o, jni_str!("toString"), jni_sig!("()Ljava/lang/String;"), &[])?.l()?;
    // Safety: toString returns a String.
    let s = unsafe { JString::from_raw(env, s.into_raw()) };
    s.try_to_string(env)
}
