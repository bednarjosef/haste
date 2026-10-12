//! Spike 2: objects. Vanilla types are generated Java proxies (same names/signatures) with no real
//! Minecraft jar on the classpath; state lives in Rust behind `haste$handle`s.
//! usage: objects <pure|mirror> <mod.jar> <sponge-mixin.jar> <bridge-dir> <proxies-dir> [asm jars...]
use std::io::Read;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use anyhow::{bail, Context, Result};
use jni::objects::{GlobalRef, JClass, JFieldID, JIntArray, JObject, JObjectArray, JStaticMethodID, JString, JValue};
use jni::signature::{Primitive, ReturnType};
use jni::sys::{jdouble, jint, jobject};
use jni::{InitArgsBuilder, JNIVersion, JNIEnv, JavaVM, NativeMethod};

// ---------- Rust-owned state ----------
const FIELDS: &[&str] = &["rightClickDelay", "missTime", "player"]; // Minecraft's shadowed fields
static INTS: [AtomicI32; 3] = [const { AtomicI32::new(0) }; 3];
static OBJECTS: Mutex<Vec<Option<GlobalRef>>> = Mutex::new(Vec::new()); // object-typed shadows by id
static ENTITY_X: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];   // entity handle -> x (f64 bits)
static STACK_COUNT: [AtomicI32; 4] = [const { AtomicI32::new(0) }; 4]; // stack handle -> count
static ENTITY_HANDLE: OnceLock<JFieldID> = OnceLock::new();
static STACK_HANDLE: OnceLock<JFieldID> = OnceLock::new();
static INVENTORY: OnceLock<GlobalRef> = OnceLock::new();
static STACKS: OnceLock<Vec<GlobalRef>> = OnceLock::new();

fn handle(env: &mut JNIEnv, this: &JObject, fid: &OnceLock<JFieldID>) -> usize {
    unsafe { env.get_field_unchecked(this, *fid.get().unwrap(), ReturnType::Primitive(Primitive::Long)) }.unwrap().j().unwrap() as usize
}

// RustState natives (shadow fields of Minecraft)
extern "system" fn rs_get_int(_e: JNIEnv, _c: JClass, id: jint) -> jint { INTS[id as usize].load(Relaxed) }
extern "system" fn rs_set_int(_e: JNIEnv, _c: JClass, id: jint, v: jint) { INTS[id as usize].store(v, Relaxed) }
extern "system" fn rs_get_object(_e: JNIEnv, _c: JClass, id: jint) -> jobject {
    OBJECTS.lock().unwrap().get(id as usize).and_then(|o| o.as_ref()).map(|g| g.as_raw()).unwrap_or(std::ptr::null_mut())
}
extern "system" fn rs_set_object(env: JNIEnv, _c: JClass, id: jint, v: JObject) {
    let g = env.new_global_ref(&v).ok();
    let mut t = OBJECTS.lock().unwrap();
    if t.len() <= id as usize { t.resize(id as usize + 1, None); }
    t[id as usize] = g;
}
// proxy natives
extern "system" fn entity_get_x(mut env: JNIEnv, this: JObject) -> jdouble {
    let h = handle(&mut env, &this, &ENTITY_HANDLE);
    f64::from_bits(ENTITY_X[h].load(Relaxed))
}
extern "system" fn player_get_inventory(_e: JNIEnv, _this: JObject) -> jobject { INVENTORY.get().unwrap().as_raw() }
extern "system" fn inventory_get_item(_e: JNIEnv, _this: JObject, slot: jint) -> jobject { STACKS.get().unwrap()[slot as usize].as_raw() }
extern "system" fn stack_get_count(mut env: JNIEnv, this: JObject) -> jint {
    let h = handle(&mut env, &this, &STACK_HANDLE);
    STACK_COUNT[h].load(Relaxed)
}
extern "system" fn stack_set_count(mut env: JNIEnv, this: JObject, v: jint) {
    let h = handle(&mut env, &this, &STACK_HANDLE);
    STACK_COUNT[h].store(v, Relaxed)
}

// ---------- landmarks (as in spike 1) ----------
struct Handler { name: String, priority: i32, class: GlobalRef, method: JStaticMethodID }
#[derive(Default)]
struct Landmark { handlers: Vec<Handler> }
impl Landmark {
    fn fire(&self, env: &mut JNIEnv) -> bool {
        for h in &self.handlers {
            let cls = unsafe { JClass::from_raw(h.class.as_raw()) };
            let r = unsafe { env.call_static_method_unchecked(&cls, h.method, ReturnType::Primitive(Primitive::Boolean), &[]) };
            match r {
                Ok(v) => if v.z().unwrap() { return true; },
                Err(e) => { let _ = env.exception_describe(); panic!("handler {} threw: {e}", h.name); }
            }
        }
        false
    }
}

struct Mirror { player: GlobalRef, x_fid: JFieldID, stack0: GlobalRef, count_fid: JFieldID }
struct Minecraft { tick_head: Landmark, body_ticks: u32, mirror: Option<Mirror> }
impl Minecraft {
    fn tick(&mut self, env: &mut JNIEnv) {
        if let Some(m) = &self.mirror {
            // mirror mode: push the cheap primitives into Java fields once per landmark
            env.set_field_unchecked(&m.player, m.x_fid, JValue::Double(f64::from_bits(ENTITY_X[0].load(Relaxed)))).unwrap();
            env.set_field_unchecked(&m.stack0, m.count_fid, JValue::Int(STACK_COUNT[0].load(Relaxed))).unwrap();
        }
        if self.tick_head.fire(env) { return; }
        self.body_ticks += 1;
    }
}

// ---------- mod jar parsing ----------
#[derive(serde::Deserialize)]
struct FabricModJson { #[serde(default)] mixins: Vec<serde_json::Value> }
#[derive(serde::Deserialize)]
struct MixinConfig { package: String, #[serde(default)] mixins: Vec<String>, #[serde(default)] client: Vec<String> }
fn read_zip_entry(jar: &std::path::Path, name: &str) -> Result<String> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(jar)?)?;
    let mut e = z.by_name(name).with_context(|| name.to_string())?;
    let mut s = String::new(); e.read_to_string(&mut s)?; Ok(s)
}
fn client_mixins(jar: &std::path::Path) -> Result<Vec<String>> {
    let fmj: FabricModJson = serde_json::from_str(&read_zip_entry(jar, "fabric.mod.json")?)?;
    let mut out = Vec::new();
    for m in &fmj.mixins {
        let cfg_name = match m { serde_json::Value::String(s) => s.clone(), serde_json::Value::Object(o) => o["config"].as_str().unwrap().to_string(), _ => bail!("bad mixins entry") };
        let cfg: MixinConfig = serde_json::from_str(&read_zip_entry(jar, &cfg_name)?)?;
        for c in cfg.mixins.iter().chain(cfg.client.iter()) { out.push(format!("{}.{}", cfg.package, c)); }
    }
    Ok(out)
}
fn jstring_array<'a>(env: &mut JNIEnv<'a>, items: &[String]) -> Result<JObject<'a>> {
    let cls = env.find_class("java/lang/String")?;
    let arr = env.new_object_array(items.len() as i32, cls, JObject::null())?;
    for (i, s) in items.iter().enumerate() { let js = env.new_string(s)?; env.set_object_array_element(&arr, i as i32, js)?; }
    Ok(arr.into())
}
fn nm(name: &str, sig: &str, f: *mut std::ffi::c_void) -> NativeMethod { NativeMethod { name: name.into(), sig: sig.into(), fn_ptr: f } }
fn reg(env: &mut JNIEnv, class: &str, methods: &[NativeMethod]) -> Result<()> {
    let c = env.find_class(class).with_context(|| format!("find {class}"))?;
    env.register_native_methods(&c, methods).with_context(|| format!("register natives on {class}"))?;
    Ok(())
}
fn probe_int(env: &mut JNIEnv, f: &str) -> Result<i32> { Ok(env.get_static_field("haste/testmod/Probe", f, "I")?.i()?) }
fn probe_double(env: &mut JNIEnv, f: &str) -> Result<f64> { Ok(env.get_static_field("haste/testmod/Probe", f, "D")?.d()?) }
fn probe_string(env: &mut JNIEnv, f: &str) -> Result<String> {
    let o = env.get_static_field("haste/testmod/Probe", f, "Ljava/lang/String;")?.l()?;
    Ok(env.get_string(&JString::from(o))?.into())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 6 { bail!("usage: objects <pure|mirror> <mod.jar> <sponge-mixin.jar> <bridge-dir> <proxies-dir> [asm jars...]"); }
    let mirror_mode = args[1] == "mirror";
    let mod_jar = std::path::Path::new(&args[2]);
    let classpath = args[2..].join(":");
    println!("== mode: {} (classpath has NO Minecraft jar; vanilla types come from {})", args[1], args[5]);

    let mut mixins = client_mixins(mod_jar)?;
    mixins.retain(|m| !m.ends_with("MinecraftMixinA") && !m.ends_with("MinecraftMixinB")); // spike-1 mixins
    println!("client mixins: {mixins:?}");

    let jvm = JavaVM::new(InitArgsBuilder::new().version(JNIVersion::V8).option(format!("-Djava.class.path={classpath}")).build()?)?;
    let mut env = jvm.attach_current_thread_permanently()?;

    // ---- natives ----
    reg(&mut env, "haste/bridge/RustState", &[
        nm("getInt", "(I)I", rs_get_int as *mut _), nm("setInt", "(II)V", rs_set_int as *mut _),
        nm("getObject", "(I)Ljava/lang/Object;", rs_get_object as *mut _), nm("setObject", "(ILjava/lang/Object;)V", rs_set_object as *mut _)])?;
    reg(&mut env, "net/minecraft/world/entity/player/Player", &[nm("getInventory", "()Lnet/minecraft/world/entity/player/Inventory;", player_get_inventory as *mut _)])?;
    reg(&mut env, "net/minecraft/world/entity/player/Inventory", &[nm("getItem", "(I)Lnet/minecraft/world/item/ItemStack;", inventory_get_item as *mut _)])?;
    reg(&mut env, "net/minecraft/world/item/ItemStack", &[nm("setCount", "(I)V", stack_set_count as *mut _)])?;
    if !mirror_mode {
        reg(&mut env, "net/minecraft/world/entity/Entity", &[nm("getX", "()D", entity_get_x as *mut _)])?;
        reg(&mut env, "net/minecraft/world/item/ItemStack", &[nm("getCount", "()I", stack_get_count as *mut _)])?;
    }
    ENTITY_HANDLE.set(env.get_field_id("net/minecraft/world/entity/Entity", "haste$handle", "J")?).ok();
    STACK_HANDLE.set(env.get_field_id("net/minecraft/world/item/ItemStack", "haste$handle", "J")?).ok();

    // ---- proxy objects: one canonical Java object per Rust entity, held by Rust ----
    let t = Instant::now();
    let player = env.new_object("net/minecraft/client/player/LocalPlayer", "()V", &[])?;
    env.set_field(&player, "haste$handle", "J", JValue::Long(0))?;
    let player = env.new_global_ref(player)?;
    let inv = env.new_object("net/minecraft/world/entity/player/Inventory", "()V", &[])?;
    INVENTORY.set(env.new_global_ref(inv)?).ok();
    let mut stacks = Vec::new();
    for h in 0..2 { let s = env.new_object("net/minecraft/world/item/ItemStack", "()V", &[])?; env.set_field(&s, "haste$handle", "J", JValue::Long(h))?; stacks.push(env.new_global_ref(s)?); }
    STACKS.set(stacks).ok();
    OBJECTS.lock().unwrap().resize(3, None);
    OBJECTS.lock().unwrap()[2] = Some(player.clone());
    println!("proxy hierarchy loaded + 4 proxy objects created in {:?}", t.elapsed());

    // ---- bridge: rewrite handlers as in spike 1 (object shadows -> RustState.getObject) ----
    let bridge = env.find_class("haste/bridge/Bridge")?;
    let ma = jstring_array(&mut env, &mixins)?;
    let fa = jstring_array(&mut env, &FIELDS.iter().map(|s| s.to_string()).collect::<Vec<_>>())?;
    let desc = env.call_static_method(&bridge, "scan", "([Ljava/lang/String;[Ljava/lang/String;)Ljava/lang/String;", &[JValue::Object(&ma), JValue::Object(&fa)]);
    let desc = match desc { Ok(v) => v.l()?, Err(e) => { env.exception_describe()?; bail!("Bridge.scan failed: {e}") } };
    let desc: String = env.get_string(&JString::from(desc))?.into();
    let mirror = if mirror_mode {
        Some(Mirror {
            player: player.clone(), x_fid: env.get_field_id("net/minecraft/world/entity/Entity", "haste$x", "D")?,
            stack0: STACKS.get().unwrap()[0].clone(), count_fid: env.get_field_id("net/minecraft/world/item/ItemStack", "count", "I")?,
        })
    } else { None };
    let mut mc = Minecraft { tick_head: Landmark::default(), body_ticks: 0, mirror };
    for line in desc.lines().filter(|l| !l.is_empty()) {
        let p: Vec<&str> = line.split('|').collect();
        let jname = env.new_string(p[5])?;
        let cls = env.call_static_method(&bridge, "classFor", "(Ljava/lang/String;)Ljava/lang/Class;", &[JValue::Object(&jname)])?.l()?;
        let class = env.new_global_ref(&cls)?;
        let method = env.get_static_method_id(&JClass::from(cls), p[6], "()Z")?;
        println!("  handler {} -> {}.{}  priority={}", p[7], p[5], p[6], p[3]);
        mc.tick_head.handlers.push(Handler { name: p[7].to_string(), priority: p[3].parse()?, class, method });
    }
    mc.tick_head.handlers.sort_by_key(|h| h.priority);

    // ---- step 3 + 4: object shadow, vanilla calls, setter into Rust, helper, duck ----
    println!("\n== step 3/4: object shadow + setter + helper + @Unique duck");
    ENTITY_X[0].store(123.5f64.to_bits(), Relaxed);
    STACK_COUNT[0].store(7, Relaxed);
    mc.tick(&mut env);
    let (x, c, helper, score) = (probe_double(&mut env, "lastX")?, probe_int(&mut env, "lastCount")?, probe_string(&mut env, "helper")?, probe_int(&mut env, "score")?);
    let duck_score = env.call_method(&player, "haste$getScore", "()I", &[])?.i()?; // Rust reads the @Unique field via the duck interface method
    println!("handler saw x={x} count={c}; Rust count now {}; helper=\"{helper}\"; duck score (via interface) = {duck_score}", STACK_COUNT[0].load(Relaxed));
    assert_eq!(x, 123.5); assert_eq!(c, 7); assert_eq!(STACK_COUNT[0].load(Relaxed), 8);
    assert_eq!(helper, "x=123.5 count=7"); assert_eq!(score, 10); assert_eq!(duck_score, 10);
    for _ in 0..99 { mc.tick(&mut env); }
    let duck_score = env.call_method(&player, "haste$getScore", "()I", &[])?.i()?;
    println!("after 100 ticks: Rust count={} duck score={duck_score} lastCount={}", STACK_COUNT[0].load(Relaxed), probe_int(&mut env, "lastCount")?);
    assert_eq!(STACK_COUNT[0].load(Relaxed), 107); assert_eq!(duck_score, 1000); assert_eq!(probe_int(&mut env, "lastCount")?, 106);
    println!("STEP 3/4 PASSED");

    // ---- step 5: foreign object (MyScreen extends Screen) with batched GuiGraphicsExtractor ----
    println!("\n== step 5: MyScreen extends Screen, batched draw calls");
    let screen = env.new_object("haste/testmod/MyScreen", "(Lnet/minecraft/network/chat/Component;)V", &[JValue::Object(&JObject::null())])?;
    env.set_field(&screen, "width", "I", JValue::Int(320))?;
    env.set_field(&screen, "height", "I", JValue::Int(240))?;
    env.call_method(&screen, "init", "()V", &[])?;
    let gfx = env.new_object("net/minecraft/client/gui/GuiGraphicsExtractor", "()V", &[])?;
    let key = env.new_object("net/minecraft/client/input/KeyEvent", "()V", &[])?;
    let render = |env: &mut JNIEnv| -> Result<(Vec<i32>, Vec<String>)> {
        env.call_method(&screen, "extractRenderState", "(Lnet/minecraft/client/gui/GuiGraphicsExtractor;IIF)V", &[JValue::Object(&gfx), JValue::Int(0), JValue::Int(0), JValue::Float(0.0)])?;
        let arr: JIntArray = env.call_static_method("haste/rt/GuiBatch", "drain", "()[I", &[])?.l()?.into();
        let mut cmds = vec![0i32; env.get_array_length(&arr)? as usize];
        env.get_int_array_region(&arr, 0, &mut cmds)?;
        let sarr: JObjectArray = env.call_static_method("haste/rt/GuiBatch", "drainStrings", "()[Ljava/lang/String;", &[])?.l()?.into();
        let mut strs = Vec::new();
        for i in 0..env.get_array_length(&sarr)? { let o = env.get_object_array_element(&sarr, i)?; strs.push(env.get_string(&JString::from(o))?.into()); }
        Ok((cmds, strs))
    };
    let (cmds, strs) = render(&mut env)?;
    println!("frame 1 commands: {cmds:?} strings: {strs:?}");
    assert_eq!(cmds, vec![1, 0, 0, 320, 240, 0xFF000000u32 as i32, 2, 0, 10, 10, 0xFFFFFF]);
    assert_eq!(strs, vec!["presses=0"]);
    env.set_field(&key, "key", "I", JValue::Int(32))?;
    let handled = env.call_method(&screen, "keyPressed", "(Lnet/minecraft/client/input/KeyEvent;)Z", &[JValue::Object(&key)])?.z()?;
    env.set_field(&key, "key", "I", JValue::Int(65))?;
    let handled2 = env.call_method(&screen, "keyPressed", "(Lnet/minecraft/client/input/KeyEvent;)Z", &[JValue::Object(&key)])?.z()?;
    let (_, strs) = render(&mut env)?;
    println!("space handled={handled}, 'A' handled={handled2}, frame 2 strings: {strs:?}");
    assert!(handled && !handled2); assert_eq!(strs, vec!["presses=1"]);
    let n = 20_000; let t = Instant::now();
    for _ in 0..n { render(&mut env)?; }
    println!("render + drain of a 2-command frame: {:.0} ns/frame", t.elapsed().as_nanos() as f64 / n as f64);
    println!("STEP 5 PASSED");

    // ---- step 6: chatty mod ----
    println!("\n== step 6: chatty handler, 10,000 x (player.getX() + stack.getCount()) per tick");
    env.set_static_field("haste/testmod/Probe", ("haste/testmod/Probe", "benchEnabled", "Z"), JValue::Bool(1))?;
    for _ in 0..30 { mc.tick(&mut env); } // JIT warm-up
    let ticks = 20; let t = Instant::now();
    for _ in 0..ticks { mc.tick(&mut env); }
    let per_tick = t.elapsed().as_nanos() as f64 / ticks as f64;
    let label = if mirror_mode { "(c) proxies, primitives mirrored into Java fields, synced per landmark" } else { "(a) pure JNI proxies (every getter is a Java->Rust native)" };
    println!("{label}:\n    {:.1} ns/call   {:.3} ms/tick", per_tick / 20_000.0, per_tick / 1e6);
    let base = env.call_static_method("haste/rt/Baseline", "run", "(II)J", &[JValue::Int(10_000), JValue::Int(50)])?.j()? as f64;
    println!("(b) pure-Java baseline, plain objects (vanilla Fabric approximation):\n    {:.1} ns/call   {:.3} ms/tick", base / 20_000.0, base / 1e6);
    let sink = probe_double(&mut env, "sink")?;
    println!("(sink={sink}, expected {})", 10_000.0 * (123.5 + STACK_COUNT[0].load(Relaxed) as f64));

    let seen = env.call_static_method("haste/rt/Unimplemented", "seen", "()Ljava/lang/String;", &[])?.l()?;
    let seen: String = env.get_string(&JString::from(seen))?.into();
    println!("\nunimplemented proxy methods hit during the run: {}", if seen.is_empty() { "none".to_string() } else { format!("\n{seen}") });
    Ok(())
}
