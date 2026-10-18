//! Memory diagnostics for the bench summaries: peak and settled RSS from `/proc/self`,
//! and the GPU memory resident for this process (the amdgpu/i915 `drm-resident-vram` +
//! `drm-resident-gtt` bytes over the process's DRM clients, the same numbers
//! `scripts/bench-vs-vanilla.sh` samples from outside).

/// Peak RSS (`VmHWM`) in KiB.
pub(super) fn peak_rss_kib() -> u64 {
    proc_status_u64("VmHWM")
}

fn proc_status_u64(key: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with(key))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(0)
}

/// `(Rss, Pss_Anon, Pss_File)` in KiB from `/proc/self/smaps_rollup`: the settled picture
/// split into anonymous (heap, stacks) and file-backed (jars, assets) pages.
pub(super) fn smaps_rollup_kib() -> (u64, u64, u64) {
    std::fs::read_to_string("/proc/self/smaps_rollup")
        .map(|s| {
            let f = |key: &str| {
                s.lines()
                    .find(|l| l.starts_with(key))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0)
            };
            (f("Rss:"), f("Pss_Anon:"), f("Pss_File:"))
        })
        .unwrap_or((0, 0, 0))
}

/// GPU memory resident for this process, in bytes: VRAM plus GTT (the GPU's share of
/// system RAM on an integrated GPU). Every open file of one DRM client repeats the
/// client's numbers, so each client's total counts once (the key is `client, field`).
pub(super) fn gpu_resident_bytes() -> u64 {
    let mut total = 0u64;
    let mut seen: std::collections::HashSet<(String, String)> = Default::default();
    for fd in std::fs::read_dir("/proc/self/fdinfo")
        .into_iter()
        .flatten()
        .flatten()
    {
        let Ok(text) = std::fs::read_to_string(fd.path()) else {
            continue;
        };
        let mut client = String::new();
        for line in text.lines() {
            let mut fields = line.split_whitespace();
            let (Some(key), Some(value), unit) = (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let Some(value) = value.parse::<u64>().ok() else {
                continue;
            };
            let bytes = match unit {
                Some("MiB") => value * 1024 * 1024,
                Some("GiB") => value * 1024 * 1024 * 1024,
                _ => value * 1024, // plain counts and KiB (bytes for `bytes` fields)
            };
            match key {
                "drm-client-id:" => client = value.to_string(),
                "drm-resident-vram:" | "drm-resident-gtt:"
                    if seen.insert((client.clone(), key.to_string())) =>
                {
                    total += bytes;
                }
                _ => {}
            }
        }
    }
    total
}

/// The bytes as MiB for the summary lines.
pub(super) fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

/// Debug: the anonymous writable mappings from `/proc/self/smaps`, grouped by size, with
/// their count and summed resident pages, so a runaway pool of mapped buffers shows up as
/// a heap of same-sized mappings.
pub(super) fn print_top_anon_mappings() {
    let Ok(text) = std::fs::read_to_string("/proc/self/smaps") else {
        return;
    };
    // (name, size) -> (count, resident KiB)
    let mut mappings: std::collections::HashMap<(&str, u64), (usize, u64)> = Default::default();
    let mut current: Option<(&str, u64)> = None; // (name, size) of the mapping being read
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Rss:") {
            if let Some((name, size)) = current {
                let rss: u64 = rest
                    .split_whitespace()
                    .next()
                    .unwrap_or("0")
                    .parse()
                    .unwrap_or(0);
                mappings.get_mut(&(name, size)).unwrap().1 += rss;
            }
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        // A mapping header: `start-end perm offset dev inode [name]`. Key: value lines
        // (Rss, Size, ...) keep the current mapping; only a new header resets it.
        if !(fields.len() >= 5
            && fields[0].contains('-')
            && fields[1].len() == 4
            && fields[1].ends_with('p'))
        {
            continue;
        }
        current = None;
        if !fields[1].starts_with("rw") {
            continue;
        }
        let mut parts = fields[0].split('-');
        let (Some(start), Some(end)) = (parts.next(), parts.next()) else {
            continue;
        };
        let (Ok(start), Ok(end)) = (u64::from_str_radix(start, 16), u64::from_str_radix(end, 16))
        else {
            continue;
        };
        // The name is the 7th field; six fields means the mapping has none (anonymous).
        let name = if fields.len() >= 7 {
            fields[6]
        } else {
            "[none]"
        };
        let size = end - start;
        mappings.entry((name, size)).or_insert((0, 0)).0 += 1;
        current = Some((name, size));
    }
    let mut rows: Vec<_> = mappings.into_iter().collect();
    rows.sort_by_key(|&((_, size), _)| std::cmp::Reverse(size));
    for ((name, size), (count, rss)) in rows.iter().take(16) {
        println!(
            "bench memory: mapping {name} {:.1} MiB x{count} ({:.1} MiB resident)",
            *size as f64 / (1024.0 * 1024.0),
            *rss as f64 / 1024.0
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_nonzero_rss() {
        let (rss, anon, file) = smaps_rollup_kib();
        assert!(rss > 0 && anon + file > 0);
        assert!(peak_rss_kib() * 1024 >= rss);
    }
}
