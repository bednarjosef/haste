//! Where the haste-jvm crate directory (its `proxies.conf` and Java sources) is found at runtime.
//! A cached haste-jvm keeps the path of the checkout it was compiled in, and every worktree shares
//! one compile cache, so the directory is looked up again: `HASTE_ROOT`, then the checkout the
//! running executable sits in, then the compile-time path.
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::error::{JvmError, Result};

/// The crate directory, relative to a checkout's root.
const CRATE_REL: &str = "crates/haste-jvm";
/// The file that marks a usable crate directory.
const PROXY_CONF: &str = "proxies.conf";

/// The first candidate crate directory that contains `proxies.conf`: `$HASTE_ROOT/crates/haste-jvm`,
/// then the first ancestor of `exe` with `crates/haste-jvm/proxies.conf` (e.g. the checkout that
/// built `target/release/haste`), then `compiled` (`CARGO_MANIFEST_DIR`). When none has it, the
/// error names every `proxies.conf` tried.
pub(crate) fn jvm_crate_dir(
    haste_root: Option<&OsStr>,
    exe: Option<&Path>,
    compiled: &Path,
) -> Result<PathBuf> {
    let mut tried = Vec::new();
    if let Some(root) = haste_root {
        let dir = Path::new(root).join(CRATE_REL);
        if has_proxies_conf(&dir) {
            return Ok(dir);
        }
        tried.push(dir.join(PROXY_CONF).display().to_string());
    }
    if let Some(exe) = exe {
        let found = exe
            .ancestors()
            .map(|a| a.join(CRATE_REL))
            .find(|d| has_proxies_conf(d));
        if let Some(dir) = found {
            return Ok(dir);
        }
        tried.push(format!("{CRATE_REL}/{PROXY_CONF} above {}", exe.display()));
    }
    if has_proxies_conf(compiled) {
        return Ok(compiled.to_path_buf());
    }
    tried.push(compiled.join(PROXY_CONF).display().to_string());
    Err(JvmError::Missing {
        what: PROXY_CONF,
        path: PathBuf::from(tried.join(", ")),
    })
}

/// The lookup for a running process: `HASTE_ROOT`, the executable, and this crate's build path.
pub(crate) fn from_environment() -> Result<PathBuf> {
    let exe = std::env::current_exe().ok();
    jvm_crate_dir(
        std::env::var_os("HASTE_ROOT").as_deref(),
        exe.as_deref(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
    )
}

fn has_proxies_conf(dir: &Path) -> bool {
    dir.join(PROXY_CONF).is_file()
}

#[cfg(test)]
mod tests;
