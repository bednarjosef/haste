//! Landmark + state-bridge spike.
//!
//! 1. Parses the test mod jar: fabric.mod.json -> mixin config -> mixin class names (Rust, serde_json).
//! 2. Starts an embedded JVM (jni crate) with the mod jar, the real sponge-mixin jar (for CallbackInfo),
//!    ASM and our bridge classes on the classpath; registers RustState natives backed by Rust-owned ints.
//! 3. Asks the Java bridge to rewrite each @Inject handler into a static method whose @Shadow accesses
//!    call RustState natives, plus a boolean trampoline (CallbackInfo + cancel check) per handler.
//! 4. A simplified `Minecraft::tick` fires a HEAD landmark that dispatches handlers in Mixin order.
//! 5. Runs 100 ticks and asserts the numbers real Mixin produced on the real server; then benchmarks.
use std::io::Read;
use std::sync::atomic::{AtomicI32, Ordering::Relaxed};
use std::time::Instant;

use anyhow::{bail, Context, Result};
use jni::objects::{GlobalRef, JClass, JObject, JStaticMethodID, JString, JValue};
use jni::signature::{Primitive, ReturnType};
use jni::sys::{jint, jvalue};
use jni::{InitArgsBuilder, JNIVersion, JNIEnv, JavaVM, NativeMethod};

// ---------- Rust-owned game state ----------
/// Fields of the target class that live in Rust. Index == id baked into the rewritten bytecode.
const FIELDS: &[&str] = &["rightClickDelay", "missTime"];
static STATE: [AtomicI32; 2] = [const { AtomicI32::new(0) }; 2];

extern "system" fn rust_get_int(_env: JNIEnv, _cls: JClass, id: jint) -> jint {
    STATE[id as usize].load(Relaxed)
}
extern "system" fn rust_set_int(_env: JNIEnv, _cls: JClass, id: jint, v: jint) {
    STATE[id as usize].store(v, Relaxed)
}
fn field(name: &str) -> i32 {
    STATE[FIELDS.iter().position(|f| *f == name).unwrap()].load(Relaxed)
}

// ---------- landmarks ----------
struct Handler {
    name: String,
    priority: i32,
    class: GlobalRef,
    method: JStaticMethodID,
}

/// One hook site (e.g. HEAD of Minecraft.tick). Handlers are stored in execution order.
#[derive(Default)]
struct Landmark {
    handlers: Vec<Handler>,
}

impl Landmark {
    /// Returns true if a handler cancelled. Mixin semantics: every callback at the same injection node
    /// is inserted before that node in mixin application order (ascending priority), so a cancel from
    /// handler N skips handlers N+1.. and the method body.
    #[inline]
    fn fire(&self, env: &mut JNIEnv) -> bool {
        for h in &self.handlers {
            let cls = unsafe { JClass::from_raw(h.class.as_raw()) };
            let r = unsafe {
                env.call_static_method_unchecked(&cls, h.method, ReturnType::Primitive(Primitive::Boolean), &[])
            };
            match r {
                Ok(v) => {
                    if v.z().unwrap() {
                        return true;
                    }
                }
                Err(e) => {
                    let _ = env.exception_describe();
                    panic!("handler {} threw: {e}", h.name);
                }
            }
        }
        false
    }
}

/// The simplified client. Real Minecraft.tick does a lot; here the body just counts.
struct Minecraft {
    tick_head: Landmark,
    body_ticks: u32,
    cancelled_ticks: u32,
}

impl Minecraft {
    fn tick(&mut self, env: &mut JNIEnv) {
        if self.tick_head.fire(env) {
            self.cancelled_ticks += 1;
            return;
        }
        self.body_ticks += 1;
    }
}

// ---------- mod jar parsing (Rust side) ----------
#[derive(serde::Deserialize)]
struct FabricModJson {
    id: String,
    #[serde(default)]
    mixins: Vec<serde_json::Value>,
}
#[derive(serde::Deserialize)]
struct MixinConfig {
    package: String,
    #[serde(default)]
    mixins: Vec<String>,
    #[serde(default)]
    client: Vec<String>,
}

fn read_zip_entry(jar: &std::path::Path, name: &str) -> Result<String> {
    let f = std::fs::File::open(jar)?;
    let mut z = zip::ZipArchive::new(f)?;
    let mut e = z.by_name(name).with_context(|| format!("{name} in {}", jar.display()))?;
    let mut s = String::new();
    e.read_to_string(&mut s)?;
    Ok(s)
}

/// Returns the fully-qualified mixin class names that apply on the client side.
fn client_mixins(jar: &std::path::Path) -> Result<(String, Vec<String>)> {
    let fmj: FabricModJson = serde_json::from_str(&read_zip_entry(jar, "fabric.mod.json")?)?;
    let mut out = Vec::new();
    for m in &fmj.mixins {
        let cfg_name = match m {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Object(o) => o["config"].as_str().unwrap().to_string(),
            _ => bail!("bad mixins entry"),
        };
        let cfg: MixinConfig = serde_json::from_str(&read_zip_entry(jar, &cfg_name)?)?;
        for c in cfg.mixins.iter().chain(cfg.client.iter()) {
            out.push(format!("{}.{}", cfg.package, c));
        }
    }
    Ok((fmj.id, out))
}

fn jstring_array<'a>(env: &mut JNIEnv<'a>, items: &[String]) -> Result<JObject<'a>> {
    let cls = env.find_class("java/lang/String")?;
    let arr = env.new_object_array(items.len() as i32, cls, JObject::null())?;
    for (i, s) in items.iter().enumerate() {
        let js = env.new_string(s)?;
        env.set_object_array_element(&arr, i as i32, js)?;
    }
    Ok(arr.into())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        bail!("usage: haste-landmark <mod.jar> <sponge-mixin.jar> <bridge-classes-dir> [asm jars...]");
    }
    let mod_jar = std::path::Path::new(&args[1]);
    let classpath = args[1..].join(":");

    let (mod_id, mut mixins) = client_mixins(mod_jar)?;
    // spike 2 added object-shadow mixins to the same mod; this binary only bridges ints
    mixins.retain(|m| m.ends_with("MinecraftMixinA") || m.ends_with("MinecraftMixinB"));
    println!("mod '{mod_id}': client mixins = {mixins:?}");

    // ---- JVM ----
    let jvm_args = InitArgsBuilder::new()
        .version(JNIVersion::V8)
        .option(format!("-Djava.class.path={classpath}"))
        .option("-Xshare:auto")
        .build()?;
    let t0 = Instant::now();
    let jvm = JavaVM::new(jvm_args)?;
    let mut env = jvm.attach_current_thread_permanently()?;
    println!("JVM started in {:?}", t0.elapsed());

    // ---- register natives: @Shadow accesses land here ----
    let state_cls = env.find_class("haste/bridge/RustState")?;
    env.register_native_methods(
        &state_cls,
        &[
            NativeMethod { name: "getInt".into(), sig: "(I)I".into(), fn_ptr: rust_get_int as *mut _ },
            NativeMethod { name: "setInt".into(), sig: "(II)V".into(), fn_ptr: rust_set_int as *mut _ },
        ],
    )?;

    // ---- bridge: scan annotations, rewrite handlers, build trampolines ----
    let bridge = env.find_class("haste/bridge/Bridge")?;
    let mixin_arr = jstring_array(&mut env, &mixins)?;
    let field_arr = jstring_array(&mut env, &FIELDS.iter().map(|s| s.to_string()).collect::<Vec<_>>())?;
    let t1 = Instant::now();
    let desc = env.call_static_method(
        &bridge,
        "scan",
        "([Ljava/lang/String;[Ljava/lang/String;)Ljava/lang/String;",
        &[JValue::Object(&mixin_arr), JValue::Object(&field_arr)],
    );
    let desc = match desc {
        Ok(v) => v.l()?,
        Err(e) => {
            env.exception_describe()?;
            bail!("Bridge.scan failed: {e}");
        }
    };
    let desc: String = env.get_string(&JString::from(desc))?.into();
    println!("bridge scan+rewrite took {:?}", t1.elapsed());

    // ---- attach handlers to landmarks ----
    let mut mc = Minecraft { tick_head: Landmark::default(), body_ticks: 0, cancelled_ticks: 0 };
    for line in desc.lines().filter(|l| !l.is_empty()) {
        let p: Vec<&str> = line.split('|').collect();
        let (target, method, at, priority, cancellable, gen_class, tramp, name) =
            (p[0], p[1], p[2], p[3].parse::<i32>()?, p[4], p[5], p[6], p[7]);
        println!("  handler {name}: {target}.{method} @{at} priority={priority} cancellable={cancellable} -> {gen_class}.{tramp}()");
        let jname = env.new_string(gen_class)?;
        let cls = env
            .call_static_method(&bridge, "classFor", "(Ljava/lang/String;)Ljava/lang/Class;", &[JValue::Object(&jname)])?
            .l()?;
        let class = env.new_global_ref(&cls)?;
        let method_id = env.get_static_method_id(&JClass::from(cls), tramp, "()Z")?;
        let handler = Handler { name: name.to_string(), priority, class, method: method_id };
        match (target, method, at) {
            ("net.minecraft.client.Minecraft", "tick", "HEAD") => mc.tick_head.handlers.push(handler),
            _ => bail!("no landmark for {target}.{method}@{at}"),
        }
    }
    // Mixin application order == ascending priority; callbacks at one node execute in that order.
    mc.tick_head.handlers.sort_by_key(|h| h.priority);
    println!("Minecraft.tick HEAD dispatch order: {:?}",
        mc.tick_head.handlers.iter().map(|h| format!("{}(p{})", h.name, h.priority)).collect::<Vec<_>>());

    // ---- the test: 100 ticks ----
    for _ in 0..100 {
        mc.tick(&mut env);
    }
    let counter = field("rightClickDelay");
    let ticks_b = field("missTime");
    println!("after 100 ticks: rightClickDelay={counter} missTime={ticks_b} cancelled={} body_ran={}",
        mc.cancelled_ticks, mc.body_ticks);
    // Ground truth from real Mixin on the real 26.3 Fabric server (see REPORT.md): lower priority first,
    // so A's increment happens even on ticks B cancels. 100 ticks -> counter 100, 20 cancels, body 80.
    assert_eq!(counter, 100, "A's @Shadow counter");
    assert_eq!(ticks_b, 100, "B's @Shadow tick counter");
    assert_eq!(mc.cancelled_ticks, 20, "cancellations");
    assert_eq!(mc.body_ticks, 80, "body executions");
    println!("TEST PASSED");

    // ---- benchmarks ----
    bench(&mut env, &bridge, &mut mc)?;
    Ok(())
}

fn bench(env: &mut JNIEnv, bridge: &JClass, mc: &mut Minecraft) -> Result<()> {
    println!("\n== benchmarks ==");
    // 1. empty landmark: no handlers attached
    let empty = Landmark::default();
    let n = 200_000_000u64;
    let t = Instant::now();
    let mut cancels = 0u64;
    for _ in 0..n {
        if std::hint::black_box(&empty).fire(env) {
            cancels += 1;
        }
    }
    let per = t.elapsed().as_nanos() as f64 / n as f64;
    println!("empty landmark (0 handlers):        {per:.2} ns/call  ({n} calls, cancels={cancels})");

    // 2. raw JNI call floor: static void noop()
    let noop = env.get_static_method_id(bridge, "noop", "()V")?;
    let bridge_global = env.new_global_ref(bridge)?;
    let call_noop = |env: &mut JNIEnv| unsafe {
        env.call_static_method_unchecked(
            &JClass::from_raw(bridge_global.as_raw()),
            noop,
            ReturnType::Primitive(Primitive::Void),
            &[] as &[jvalue],
        )
        .unwrap();
    };
    for _ in 0..200_000 {
        call_noop(env);
    } // warm up
    let n = 5_000_000u64;
    let t = Instant::now();
    for _ in 0..n {
        call_noop(env);
    }
    println!("Rust -> Java empty static (JNI):    {:.1} ns/call", t.elapsed().as_nanos() as f64 / n as f64);

    // 3. one real handler via its trampoline: new CallbackInfo + handler (getInt/setInt natives) + isCancelled
    let one = Landmark { handlers: vec![Handler {
        name: mc.tick_head.handlers[0].name.clone(),
        priority: 0,
        class: mc.tick_head.handlers[0].class.clone(),
        method: mc.tick_head.handlers[0].method,
    }] };
    for _ in 0..200_000 {
        one.fire(env);
    } // warm up (JIT)
    let n = 5_000_000u64;
    let t = Instant::now();
    for _ in 0..n {
        one.fire(env);
    }
    println!("landmark with 1 handler ({}):\n                                    {:.1} ns/call  (CallbackInfo alloc + 1 getInt + 1 setInt native + isCancelled)",
        one.handlers[0].name, t.elapsed().as_nanos() as f64 / n as f64);

    // 4. the full tick landmark (2 handlers, one cancels every 5th)
    let n = 2_000_000u64;
    let t = Instant::now();
    for _ in 0..n {
        mc.tick(env);
    }
    println!("Minecraft.tick with 2 handlers:     {:.1} ns/tick", t.elapsed().as_nanos() as f64 / n as f64);
    Ok(())
}
