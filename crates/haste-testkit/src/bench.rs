//! Benchmark recording: JSON lines of frame times + RSS + section counts, written by the
//! client recorder (`/haste bench`) and by Haste itself; summarised into Markdown tables.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// One recorded sample (one per frame).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct BenchSample {
    /// Frame index.
    pub frame: u32,
    /// Wall time of the frame in milliseconds.
    pub frame_ms: f32,
    /// Resident set size in KiB, read from /proc/self/status (VmRSS) by the recorder.
    pub rss_kib: u64,
    /// Chunk sections drawn this frame.
    pub sections_drawn: u32,
    /// Chunk sections resident (loaded) at this frame.
    pub sections_total: u32,
    /// Draw calls this frame.
    pub draw_calls: u32,
}

/// A loaded/recorded bench run.
#[derive(Clone, Debug)]
pub struct Bench {
    pub name: String,
    pub samples: Vec<BenchSample>,
}

impl Bench {
    /// Loads a JSON-lines bench file (one `BenchSample` per line).
    pub fn load(path: &Path) -> Result<Bench> {
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("bench")
            .to_string();
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut samples = Vec::new();
        for (i, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let s: BenchSample = serde_json::from_str(line)
                .with_context(|| format!("{}: line {}", path.display(), i + 1))?;
            samples.push(s);
        }
        Ok(Bench { name, samples })
    }

    pub fn percentile_frame_ms(&self, p: f64) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let mut times: Vec<f32> = self.samples.iter().map(|s| s.frame_ms).collect();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let idx = ((times.len() as f64 - 1.0) * p.clamp(0.0, 1.0)).round() as usize;
        times[idx.min(times.len() - 1)]
    }

    pub fn mean_frame_ms(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        self.samples.iter().map(|s| s.frame_ms).sum::<f32>() / self.samples.len() as f32
    }

    /// FPS estimate from the mean frame time.
    pub fn fps(&self) -> f64 {
        let mean = self.mean_frame_ms();
        if mean <= 0.0 {
            0.0
        } else {
            1000.0 / mean as f64
        }
    }

    pub fn max_rss_kib(&self) -> u64 {
        self.samples.iter().map(|s| s.rss_kib).max().unwrap_or(0)
    }

    pub fn mean_draw_calls(&self) -> f64 {
        if self.samples.is_empty() {
            0.0
        } else {
            self.samples
                .iter()
                .map(|s| s.draw_calls as f64)
                .sum::<f64>()
                / self.samples.len() as f64
        }
    }

    pub fn mean_sections_drawn(&self) -> f64 {
        if self.samples.is_empty() {
            0.0
        } else {
            self.samples
                .iter()
                .map(|s| s.sections_drawn as f64)
                .sum::<f64>()
                / self.samples.len() as f64
        }
    }

    /// Writes a JSON-lines bench file (the recorder-side sink).
    pub fn write_jsonl(&self, path: &Path) -> Result<()> {
        let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
        for s in &self.samples {
            writeln!(f, "{}", serde_json::to_string(s)?)?;
        }
        f.flush()?;
        Ok(())
    }
}

/// Streaming writer for live recording (one line per frame, flushed on finish).
pub struct BenchWriter {
    inner: std::io::BufWriter<std::fs::File>,
}

impl BenchWriter {
    pub fn create(path: &Path) -> Result<BenchWriter> {
        Ok(BenchWriter {
            inner: std::io::BufWriter::new(std::fs::File::create(path)?),
        })
    }

    pub fn push(&mut self, s: &BenchSample) -> Result<()> {
        writeln!(self.inner, "{}", serde_json::to_string(s)?)?;
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        self.inner.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonl_round_trip_and_stats() {
        let dir = std::env::temp_dir().join(format!("haste-testkit-bench-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vanilla.jsonl");
        let bench = Bench {
            name: "vanilla".into(),
            samples: (0..100)
                .map(|i| BenchSample {
                    frame: i,
                    frame_ms: if i < 50 { 16.7 } else { 33.4 },
                    rss_kib: 500_000 + i as u64 * 10,
                    sections_drawn: 900,
                    sections_total: 4000,
                    draw_calls: 200,
                })
                .collect(),
        };
        bench.write_jsonl(&path).unwrap();
        let back = Bench::load(&path).unwrap();
        assert_eq!(back.name, "vanilla");
        assert_eq!(back.samples.len(), 100);
        assert_eq!(back.samples[7].rss_kib, 500_070);
        // half 16.7, half 33.4 -> median at the boundary is 33.4 (index rounds up)
        assert!((back.percentile_frame_ms(0.5) - 33.4).abs() < 1e-4);
        assert!((back.percentile_frame_ms(0.0) - 16.7).abs() < 1e-4);
        assert!((back.percentile_frame_ms(1.0) - 33.4).abs() < 1e-4);
        assert!((back.mean_frame_ms() - 25.05).abs() < 0.01);
        assert_eq!(back.max_rss_kib(), 500_990);
        assert!((back.fps() - 1000.0 / 25.05).abs() < 0.01);
        assert!((back.mean_draw_calls() - 200.0).abs() < 1e-9);
        assert!((back.mean_sections_drawn() - 900.0).abs() < 1e-9);
    }

    #[test]
    fn bench_writer_streams() {
        let dir = std::env::temp_dir().join(format!("haste-testkit-benchw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("live.jsonl");
        let mut w = BenchWriter::create(&path).unwrap();
        for i in 0..3 {
            w.push(&BenchSample {
                frame: i,
                frame_ms: 16.0,
                rss_kib: 1,
                sections_drawn: 1,
                sections_total: 1,
                draw_calls: 1,
            })
            .unwrap();
        }
        w.finish().unwrap();
        let bench = Bench::load(&path).unwrap();
        assert_eq!(bench.samples.len(), 3);
        assert_eq!(bench.samples[2].frame, 2);
    }
}
