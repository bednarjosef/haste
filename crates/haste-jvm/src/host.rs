//! The JVM host: starts the one JVM of the process and boots Haste's Java side.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jni::objects::{JClass, JObject, JString, JValue};
use jni::refs::Global;
use jni::{jni_sig, jni_str, Env, InitArgsBuilder, JNIVersion, JavaVM};
use parking_lot::Mutex;

use crate::error::{JvmError, Result};
use crate::gui::GuiPipeline;
use crate::loader::{self, ClassLog};

/// Where everything the JVM host needs lives. Build one with [`JvmConfig::from_cache`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JvmConfig {
    /// JDK 25 home (`cache/jdk`); `lib/server/libjvm.so` and `bin/javac` are used.
    pub java_home: PathBuf,
    /// The player's 26.3 client jar. Read one class at a time by `HasteClassLoader`, never on the class path.
    pub client_jar: PathBuf,
    /// Vanilla's third-party libraries (fastutil, guava, gson, joml, DFU, brigadier, ...).
    pub libraries: Vec<PathBuf>,
    /// ASM jars for the proxy generator (boot class path only).
    pub asm_jars: Vec<PathBuf>,
    /// Directory with this crate's Java sources (`java/`) and `build-java.sh`.
    pub java_src: PathBuf,
    /// Output of `build-java.sh` (`boot/`, `game/`); gitignored.
    pub build_dir: PathBuf,
    /// The proxy config (`proxies.conf`).
    pub proxies_conf: PathBuf,
    /// Where generated proxies are written at startup (`cache/proxies/26.3`).
    pub proxy_dir: PathBuf,
    /// Extra JVM options (e.g. `-Xmx512m`).
    pub jvm_options: Vec<String>,
    /// Print refused classes to stderr as they happen.
    pub verbose: bool,
}

impl JvmConfig {
    /// The standard layout under a Haste cache directory (`haste_core::paths::cache_dir()` or the
    /// repo's `cache/` symlink). Libraries come from `libraries/` (WS7's download) or, until that
    /// exists, from the spike server's `server-spike/libraries`.
    pub fn from_cache(cache: &Path) -> Result<JvmConfig> {
        let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let java_home = cache.join("jdk");
        let client_jar = cache
            .join("versions")
            .join(haste_core::paths::MC_VERSION)
            .join("client.jar");
        let lib_dir = [
            cache.join("libraries"),
            cache.join("server-spike").join("libraries"),
        ]
        .into_iter()
        .find(|d| d.is_dir())
        .ok_or(JvmError::Missing {
            what: "vanilla libraries directory",
            path: cache.join("libraries"),
        })?;
        let asm_dir = cache.join("mojang-spike");
        let mut asm_jars: Vec<PathBuf> = std::fs::read_dir(&asm_dir)
            .map_err(|_| JvmError::Missing {
                what: "ASM jars",
                path: asm_dir.clone(),
            })?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("asm-") && n.ends_with(".jar"))
            })
            .collect();
        asm_jars.sort();
        let cfg = JvmConfig {
            java_home,
            client_jar,
            libraries: loader::library_jars(&lib_dir),
            asm_jars,
            java_src: crate_dir.clone(),
            build_dir: crate_dir.join("build"),
            proxies_conf: crate_dir.join("proxies.conf"),
            proxy_dir: cache.join("proxies").join(haste_core::paths::MC_VERSION),
            jvm_options: Vec::new(),
            verbose: std::env::var_os("HASTE_JVM_VERBOSE").is_some(),
        };
        cfg.check()?;
        Ok(cfg)
    }

    fn check(&self) -> Result<()> {
        for (what, p) in [
            ("JDK", &self.java_home),
            ("client jar", &self.client_jar),
            ("proxies.conf", &self.proxies_conf),
        ] {
            if !p.exists() {
                return Err(JvmError::Missing {
                    what,
                    path: p.clone(),
                });
            }
        }
        if self.asm_jars.is_empty() {
            return Err(JvmError::Missing {
                what: "ASM jars",
                path: PathBuf::new(),
            });
        }
        Ok(())
    }

    fn libjvm(&self) -> Result<PathBuf> {
        let names = [
            "lib/server/libjvm.so",
            "lib/server/libjvm.dylib",
            "bin/server/jvm.dll",
        ];
        names
            .iter()
            .map(|n| self.java_home.join(n))
            .find(|p| p.is_file())
            .ok_or(JvmError::Missing {
                what: "libjvm",
                path: self.java_home.join(names[0]),
            })
    }
}

/// How long each startup phase took.
#[derive(Clone, Copy, Debug, Default)]
pub struct StartTimings {
    /// `build-java.sh` (zero when the build was up to date).
    pub java_build: Duration,
    /// `JNI_CreateJavaVM`.
    pub jvm_create: Duration,
    /// `Boot.boot`: proxy generation (if stale), class loader, `haste.gui.Bridge`.
    pub boot: Duration,
}

pub(crate) struct Inner {
    pub(crate) config: JvmConfig,
    pub(crate) vm: JavaVM,
    pub(crate) boot: Global<JClass<'static>>,
    pub(crate) timings: StartTimings,
    pub(crate) gen_summary: String,
    pub(crate) gui: GuiPipeline,
}

/// The embedded JVM. Cheap to clone; there is one per process (a second [`JvmHost::start`] with the
/// same config returns the running host).
#[derive(Clone)]
pub struct JvmHost {
    inner: Arc<Inner>,
}

static HOST: Mutex<Option<JvmHost>> = Mutex::new(None);

impl JvmHost {
    /// Starts the JVM with: tier-3 class loader over the player's client jar (allow-list of
    /// stateless packages/classes), generated proxies for tier-1 types, Haste's runtime classes.
    /// Never the whole jar on the class path.
    pub fn start(cfg: JvmConfig) -> anyhow::Result<JvmHost> {
        let mut slot = HOST.lock();
        if let Some(h) = slot.as_ref() {
            if h.inner.config == cfg {
                return Ok(h.clone());
            }
            return Err(JvmError::AlreadyStarted.into());
        }
        let host = Self::start_inner(cfg)?;
        *slot = Some(host.clone());
        Ok(host)
    }

    fn start_inner(cfg: JvmConfig) -> Result<JvmHost> {
        cfg.check()?;
        let t = Instant::now();
        let built = build_java(&cfg)?;
        let java_build = if built { t.elapsed() } else { Duration::ZERO };

        let boot_cp = std::iter::once(cfg.build_dir.join("boot"))
            .chain(cfg.asm_jars.iter().cloned())
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(":");
        let mut args = InitArgsBuilder::new()
            .version(JNIVersion::V21)
            .option(format!("-Djava.class.path={boot_cp}"))
            .option("-Djava.awt.headless=true")
            .option("-Xshare:auto")
            .option("-Xss4m")
            // Link game classes lazily: with verification on, the verifier loads every type a method
            // body could assign (all screens PauseScreen can open, their listeners, ...) before any of
            // it runs, and the first refused one fails the class. The vanilla classes are the player's
            // own jar and the proxies are generated by us. See NOTES.md.
            .option("-XX:+UnlockDiagnosticVMOptions")
            .option("-XX:-BytecodeVerificationRemote");
        for o in &cfg.jvm_options {
            args = args.option(o.clone());
        }
        let args = args.build().map_err(|e| JvmError::Start(e.to_string()))?;
        let libjvm = cfg.libjvm()?;
        let t = Instant::now();
        let vm = JavaVM::with_libjvm(args, || Ok(libjvm.clone()))
            .map_err(|e| JvmError::Start(e.to_string()))?;
        let jvm_create = t.elapsed();

        let t = Instant::now();
        let (boot, bridge, gen_summary) = vm.attach_current_thread(|env| -> Result<_> {
            crate::natives::register(env)?;
            let boot_cls = env.find_class(jni_str!("haste/loader/Boot"))?;
            let conf = JString::from_str(env, boot_config(&cfg))?;
            let bridge = env
                .call_static_method(
                    &boot_cls,
                    jni_str!("boot"),
                    jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
                    &[JValue::Object(&conf)],
                )
                .map_err(|e| caught(env, e))?
                .l()?;
            let summary = call_static_string(env, &boot_cls, "genSummary")?;
            Ok((
                env.new_global_ref(&boot_cls)?,
                env.new_global_ref(&bridge)?,
                summary,
            ))
        })?;
        let boot_time = t.elapsed();
        let gui = vm.attach_current_thread(|env| GuiPipeline::new(env, bridge))?;
        let timings = StartTimings {
            java_build,
            jvm_create,
            boot: boot_time,
        };
        tracing::info!(?timings, %gen_summary, "JVM started");
        Ok(JvmHost {
            inner: Arc::new(Inner {
                config: cfg,
                vm,
                boot,
                timings,
                gen_summary,
                gui,
            }),
        })
    }

    /// The batched GUI pipeline (vanilla screens).
    pub fn gui(&self) -> &GuiPipeline {
        &self.inner.gui
    }

    /// Startup phase timings.
    pub fn timings(&self) -> StartTimings {
        self.inner.timings
    }

    /// The proxy generator's summary from this start ("generated N classes ..." or "up to date").
    pub fn proxy_summary(&self) -> &str {
        &self.inner.gen_summary
    }

    /// The configuration the JVM was started with.
    pub fn config(&self) -> &JvmConfig {
        &self.inner.config
    }

    /// Runs `f` with a JNI environment on the current thread (attaching it permanently if needed).
    pub fn with_env<T>(&self, f: impl FnOnce(&mut Env) -> Result<T>) -> Result<T> {
        self.inner.vm.attach_current_thread(f)
    }

    /// Loads (and initialises) a class through the game class loader: a real tier-3 class from the
    /// jar, a generated proxy, or `haste.gui.*`. Fails (and logs) for refused classes; the Java
    /// exception is cleared and returned as the error.
    pub fn load_class<'local>(
        &self,
        env: &mut Env<'local>,
        binary_name: &str,
    ) -> Result<JClass<'local>> {
        let name = JString::from_str(env, binary_name)?;
        let cls = env
            .call_static_method(
                &*self.inner.boot,
                jni_str!("loadClass"),
                jni_sig!("(Ljava/lang/String;)Ljava/lang/Class;"),
                &[JValue::Object(&name)],
            )
            .map_err(|e| caught(env, e))?
            .l()?;
        // Safety: Boot.loadClass returns a java.lang.Class.
        Ok(unsafe { JClass::from_raw(env, cls.into_raw()) })
    }

    /// The class loader's log: everything served from the jar, as a proxy, or refused.
    pub fn class_log(&self) -> Result<ClassLog> {
        self.with_env(|env| {
            Ok(ClassLog::parse(&call_static_string(
                env,
                &self.inner.boot,
                "classLog",
            )?))
        })
    }

    /// Methods of generated proxies that were called without an implementation (sorted, distinct).
    pub fn unimplemented(&self) -> Result<Vec<String>> {
        self.with_env(|env| {
            let cls = env.find_class(jni_str!("haste/rt/Unimplemented"))?;
            let s = call_static_string(env, &cls, "seen")?;
            Ok(s.lines()
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect())
        })
    }

    /// Clears the Unimplemented log (tests use it to attribute entries to one screen).
    pub fn clear_unimplemented(&self) -> Result<()> {
        self.with_env(|env| {
            let cls = env.find_class(jni_str!("haste/rt/Unimplemented"))?;
            env.call_static_method(&cls, jni_str!("clear"), jni_sig!("()V"), &[])?;
            Ok(())
        })
    }
}

/// Turns a pending Java exception into `Error::CaughtJavaException` (class, message, stack) and
/// clears it, so the caller can handle the failure and keep using the thread.
pub(crate) fn caught(env: &mut Env, e: jni::errors::Error) -> JvmError {
    if !matches!(e, jni::errors::Error::JavaException) {
        return e.into();
    }
    let Some(t) = env.exception_occurred() else {
        return e.into();
    };
    env.exception_clear();
    let described = (|| -> Result<String> {
        let cls = env.find_class(jni_str!("haste/rt/Errors"))?;
        let s = env
            .call_static_method(
                &cls,
                jni_str!("describe"),
                jni_sig!("(Ljava/lang/Throwable;)Ljava/lang/String;"),
                &[JValue::Object(&t)],
            )?
            .l()?;
        jstring_to_string(env, s)
    })();
    env.exception_clear();
    match described {
        Ok(s) => JvmError::Java(s),
        Err(_) => e.into(),
    }
}

/// Calls a static `()Ljava/lang/String;` method by name.
pub(crate) fn call_static_string(env: &mut Env, cls: &JClass, name: &str) -> Result<String> {
    let name = jni::strings::JNIString::new(name);
    let v = env
        .call_static_method(cls, &name, jni_sig!("()Ljava/lang/String;"), &[])?
        .l()?;
    jstring_to_string(env, v)
}

/// Converts a (possibly null) `java.lang.String` reference to a Rust string ("" for null).
pub(crate) fn jstring_to_string(env: &mut Env, obj: JObject) -> Result<String> {
    if obj.is_null() {
        return Ok(String::new());
    }
    // Safety: callers pass String references.
    let s = unsafe { JString::from_raw(env, obj.into_raw()) };
    Ok(s.try_to_string(env)?)
}

fn boot_config(cfg: &JvmConfig) -> String {
    let mut s = String::new();
    let mut kv = |k: &str, v: &str| {
        s.push_str(k);
        s.push('=');
        s.push_str(v);
        s.push('\n');
    };
    kv("clientJar", &cfg.client_jar.display().to_string());
    kv("gameDir", &cfg.build_dir.join("game").display().to_string());
    kv("proxyDir", &cfg.proxy_dir.display().to_string());
    kv("proxiesConf", &cfg.proxies_conf.display().to_string());
    kv("verbose", if cfg.verbose { "true" } else { "false" });
    for l in &cfg.libraries {
        kv("lib", &l.display().to_string());
    }
    for p in loader::ALLOW_PACKAGES {
        kv("allowPackage", p);
    }
    for c in loader::ALLOW_CLASSES {
        kv("allowClass", c);
    }
    for d in loader::DENY {
        kv("deny", d);
    }
    s
}

/// Runs `build-java.sh` for `boot` and `game` when their sources changed. Returns whether it built.
fn build_java(cfg: &JvmConfig) -> Result<bool> {
    let java = cfg.java_src.join("java").join("haste");
    let script = cfg.java_src.join("build-java.sh");
    let boot_dirs = [java.join("loader"), java.join("proxy"), java.join("rt")];
    let boot_stamp = source_stamp(&boot_dirs, &[&script]);
    let built_boot = build_one(cfg, "boot", &boot_stamp)?;
    // game classes compile against the boot classes and the client jar
    let game_stamp = format!(
        "{}\n{boot_stamp}",
        source_stamp(&[java.join("gui")], &[&script, &cfg.client_jar])
    );
    let built_game = build_one(cfg, "game", &game_stamp)?;
    Ok(built_boot || built_game)
}

fn build_one(cfg: &JvmConfig, what: &str, stamp: &str) -> Result<bool> {
    let stamp_file = cfg.build_dir.join(format!(".stamp-{what}"));
    if cfg.build_dir.join(what).is_dir()
        && std::fs::read_to_string(&stamp_file).is_ok_and(|s| s == stamp)
    {
        return Ok(false);
    }
    let join = |v: &[PathBuf]| {
        v.iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(":")
    };
    let out = Command::new("bash")
        .arg(cfg.java_src.join("build-java.sh"))
        .arg(what)
        .arg(&cfg.java_home)
        .arg(&cfg.client_jar)
        .arg(&cfg.build_dir)
        .arg(join(&cfg.asm_jars))
        .arg(join(&cfg.libraries))
        .output()
        .map_err(|e| JvmError::JavaBuild(e.to_string()))?;
    if !out.status.success() {
        return Err(JvmError::JavaBuild(format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    std::fs::write(&stamp_file, stamp).map_err(|e| JvmError::JavaBuild(e.to_string()))?;
    Ok(true)
}

/// A cheap fingerprint of source trees and single files: every file's path, size and mtime.
fn source_stamp(dirs: &[PathBuf], files: &[&Path]) -> String {
    fn walk(d: &Path, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else {
                out.push(fingerprint(&p));
            }
        }
    }
    fn fingerprint(p: &Path) -> String {
        let m = std::fs::metadata(p).ok();
        let mtime = m
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        format!("{}|{}|{}", p.display(), m.map_or(0, |m| m.len()), mtime)
    }
    let mut v = Vec::new();
    for d in dirs {
        walk(d, &mut v);
    }
    v.extend(files.iter().map(|f| fingerprint(f)));
    v.sort();
    v.join("\n")
}
