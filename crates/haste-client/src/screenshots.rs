//! F2 screenshots: file naming and saving in vanilla's layout (`screenshots/` under the
//! config dir, `yyyy-MM-dd_HH.mm.ss.png` in local time, `_1`, `_2`, ... on collision).

use std::path::{Path, PathBuf};

use anyhow::Context;

/// The `screenshots` directory under haste's config dir, created on demand.
pub fn dir() -> Option<PathBuf> {
    haste_setup::config::config_path().map(|mut p| {
        p.set_file_name("screenshots");
        p
    })
}

/// The PNG file name for `utc_secs` in local time, `yyyy-MM-dd_HH.mm.ss` (vanilla's shape);
/// if it exists, `_1`, `_2`, ... are appended before `.png`.
pub fn file_name(dir: &Path, utc_secs: i64) -> String {
    let (year, mon, day, h, min, s) = local_ymdhms(utc_secs);
    let mut name = format!("{year:04}-{mon:02}-{day:02}_{h:02}.{min:02}.{s:02}");
    let mut n = 0;
    while dir.join(format!("{name}.png")).exists() {
        n += 1;
        name = format!("{year:04}-{mon:02}-{day:02}_{h:02}.{min:02}.{s:02}_{n}");
    }
    format!("{name}.png")
}

/// Local time via `libc::localtime_r` (the tree has no date crate and adding one needs a
/// download; `libc` is already a dependency of winit).
fn local_ymdhms(utc_secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    #[cfg(unix)]
    {
        let t = utc_secs as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
            tracing::warn!("localtime_r failed; screenshot name falls back to UTC");
        }
        (
            tm.tm_year + 1900,
            (tm.tm_mon + 1) as u32,
            tm.tm_mday as u32,
            tm.tm_hour as u32,
            tm.tm_min as u32,
            tm.tm_sec as u32,
        )
    }
    #[cfg(not(unix))]
    {
        // Windows: no libc in the tree; fall back to a day-count split (UTC).
        let days = utc_secs.div_euclid(86400);
        let rem = utc_secs.rem_euclid(86400);
        let (y, m, d) = civil_from_days(days);
        (
            y,
            m,
            d,
            (rem / 3600) as u32,
            ((rem % 3600) / 60) as u32,
            (rem % 60) as u32,
        )
    }
}

/// Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm), for the non-unix
/// fallback.
#[cfg(not(unix))]
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y } as i32, m, d)
}

/// Saves `image` as `<config dir>/screenshots/<name>.png`, off the caller's thread is the
/// caller's business. Returns the file name without extension for the log line.
pub fn save(image: image::RgbaImage) -> anyhow::Result<String> {
    let dir = dir().context("no platform config dir on this system")?;
    std::fs::create_dir_all(&dir).context("creating the screenshots dir")?;
    let name = file_name(&dir, now_unix_secs());
    let path = dir.join(&name);
    image
        .save(&path)
        .with_context(|| format!("saving {path:?}"))?;
    Ok(name)
}

fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Saves a script `screenshot NAME` action's frame as `target/shots/script-NAME.png`.
pub fn save_script_shot(image: &image::RgbaImage, name: &str) -> anyhow::Result<()> {
    let path = std::path::Path::new("target/shots").join(format!("script-{name}.png"));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    image
        .save(&path)
        .with_context(|| format!("saving {}", path.display()))?;
    println!("script screenshot saved to {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("haste-screenshot-{tag}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn names_are_vanilla_shaped() {
        let d = tmp_dir("shape");
        let name = file_name(&d, 0);
        // Local time: the date part may shift by a timezone, so only check the shape.
        // `^\d{4}-\d{2}-\d{2}_\d{2}\.\d{2}\.\d{2}\.png$`
        let parts: Vec<&str> = name.trim_end_matches(".png").split('_').collect();
        assert_eq!(parts.len(), 2);
        let date: Vec<&str> = parts[0].split('-').collect();
        assert_eq!(date.len(), 3);
        assert!(date.iter().all(|p| p.len() == 2 || p.len() == 4));
        let clock: Vec<&str> = parts[1].split('.').collect();
        assert_eq!(clock.len(), 3);
        assert!(clock.iter().all(|p| p.len() == 2));
    }

    #[test]
    fn collisions_get_numbered_suffixes() {
        let d = tmp_dir("collision");
        let first = file_name(&d, 1_700_000_000);
        // One `_` separator (date_clock), no numbered suffix yet.
        assert_eq!(first.matches('_').count(), 1, "no suffix expected: {first}");
        std::fs::write(d.join(&first), b"").unwrap();
        let second = file_name(&d, 1_700_000_000);
        assert_eq!(second, first.replace(".png", "_1.png"));
        std::fs::write(d.join(&second), b"").unwrap();
        let third = file_name(&d, 1_700_000_000);
        assert_eq!(third, first.replace(".png", "_2.png"));
    }

    #[test]
    fn different_timestamps_do_not_collide() {
        let d = tmp_dir("distinct");
        let a = file_name(&d, 1_700_000_000);
        let b = file_name(&d, 1_700_000_001);
        // Either distinct names, or the second gets a suffix (can happen only across a
        // whole-second boundary in a timezone with an offset of 30 minutes... which
        // cannot produce equal names, so they must differ).
        assert_ne!(a, b);
    }
}
