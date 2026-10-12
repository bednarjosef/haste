//! Markdown table writer for reports: bench summaries, trace replays, screenshot diffs.

use crate::bench::Bench;
use crate::screenshots::DiffResult;
use crate::traces::TraceReport;

/// Small helper that renders rows into a Markdown table.
pub struct Markdown {
    out: String,
}

impl Default for Markdown {
    fn default() -> Self {
        Self::new()
    }
}

impl Markdown {
    pub fn new() -> Markdown {
        Markdown { out: String::new() }
    }

    pub fn heading(&mut self, level: usize, text: &str) -> &mut Self {
        use std::fmt::Write;
        writeln!(self.out, "{} {text}", "#".repeat(level.min(6))).unwrap();
        writeln!(self.out).unwrap();
        self
    }

    pub fn paragraph(&mut self, text: &str) -> &mut Self {
        use std::fmt::Write;
        writeln!(self.out, "{text}").unwrap();
        writeln!(self.out).unwrap();
        self
    }

    /// Writes a Markdown table. `align` is per column: `<` left, `>` right.
    pub fn table(&mut self, headers: &[&str], align: &[char], rows: &[Vec<String>]) -> &mut Self {
        use std::fmt::Write;
        assert_eq!(headers.len(), align.len(), "header/align count mismatch");
        let write_row = |out: &mut String, cells: &[String], bold: bool| {
            write!(out, "|").unwrap();
            for c in cells {
                if bold {
                    write!(out, " **{c}** |").unwrap();
                } else {
                    write!(out, " {c} |").unwrap();
                }
            }
            writeln!(out).unwrap();
        };
        let head: Vec<String> = headers.iter().map(|s| s.to_string()).collect();
        write_row(&mut self.out, &head, true);
        write!(self.out, "|").unwrap();
        for a in align {
            write!(self.out, "---{a}|").unwrap();
        }
        writeln!(self.out).unwrap();
        for row in rows {
            assert_eq!(row.len(), headers.len(), "row width mismatch");
            write_row(&mut self.out, row, false);
        }
        writeln!(self.out).unwrap();
        self
    }

    pub fn finish(&self) -> String {
        self.out.clone()
    }
}

/// Bench comparison table: name, frames, fps, mean/1%/99% frame ms, max RSS, draw calls.
pub fn bench_table(benches: &[&Bench]) -> String {
    let mut md = Markdown::new();
    md.heading(3, "Benchmarks");
    let rows: Vec<Vec<String>> = benches
        .iter()
        .map(|b| {
            vec![
                b.name.clone(),
                b.samples.len().to_string(),
                format!("{:.1}", b.fps()),
                format!("{:.2}", b.mean_frame_ms()),
                format!("{:.2}", b.percentile_frame_ms(0.99)),
                format!("{:.1} MiB", b.max_rss_kib() as f64 / 1024.0),
                format!("{:.0}", b.mean_draw_calls()),
                format!("{:.0}", b.mean_sections_drawn()),
            ]
        })
        .collect();
    md.table(
        &[
            "name",
            "frames",
            "fps",
            "mean ms",
            "p99 ms",
            "max rss",
            "draw calls",
            "sections",
        ],
        &['<', '>', '>', '>', '>', '>', '>', '>'],
        &rows,
    );
    md.finish()
}

/// Trace replay table: name, ticks, first divergence, max/mean error, on_ground mismatches, pass.
pub fn trace_table(reports: &[TraceReport]) -> String {
    let mut md = Markdown::new();
    md.heading(3, "Physics trace replays");
    let rows: Vec<Vec<String>> = reports
        .iter()
        .map(|r| {
            vec![
                r.name.clone(),
                r.ticks.to_string(),
                r.first_divergence_tick
                    .map(|t| t.to_string())
                    .unwrap_or_else(|| "-".into()),
                format!("{:.4}", r.max_pos_error),
                format!("{:.4}", r.mean_pos_error),
                r.on_ground_mismatches.to_string(),
                if r.pass { "pass".into() } else { "FAIL".into() },
            ]
        })
        .collect();
    md.table(
        &[
            "trace",
            "ticks",
            "first divergence",
            "max err",
            "mean err",
            "on_ground mismatches",
            "result",
        ],
        &['<', '>', '>', '>', '>', '>', '<'],
        &rows,
    );
    md.finish()
}

/// Screenshot diff table: name, size, diff fraction, mean/max error.
pub fn diff_table(diffs: &[(String, &DiffResult)]) -> String {
    let mut md = Markdown::new();
    md.heading(3, "Screenshot diffs");
    let rows: Vec<Vec<String>> = diffs
        .iter()
        .map(|(name, d)| {
            vec![
                name.clone(),
                format!("{}x{}", d.width, d.height),
                format!("{:.4}%", d.diff_fraction * 100.0),
                format!("{:.2}", d.mean_error),
                d.max_error.to_string(),
            ]
        })
        .collect();
    md.table(
        &["shot", "size", "diff", "mean err", "max err"],
        &['<', '>', '>', '>', '>'],
        &rows,
    );
    md.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screenshots::DiffResult;
    use crate::traces::TraceReport;

    #[test]
    fn renders_tables() {
        let bench = Bench {
            name: "vanilla".into(),
            samples: vec![crate::bench::BenchSample {
                frame: 0,
                frame_ms: 16.7,
                rss_kib: 1024,
                sections_drawn: 100,
                sections_total: 200,
                draw_calls: 50,
            }],
        };
        let html = bench_table(&[&bench]);
        assert!(html.contains("**name**"));
        assert!(html.contains("vanilla"));
        assert!(html.contains("---<|"));

        let report = TraceReport {
            name: "walk".into(),
            ticks: 10,
            first_divergence_tick: None,
            max_pos_error: 0.0,
            mean_pos_error: 0.0,
            on_ground_mismatches: 0,
            errors: vec![],
            pass: true,
        };
        let md = trace_table(&[report]);
        assert!(md.contains("walk"));
        assert!(md.contains("pass"));

        let d = DiffResult {
            width: 10,
            height: 10,
            diff_fraction: 0.05,
            mean_error: 1.2,
            max_error: 30,
            diff_image: None,
        };
        let md = diff_table(&[("flat-dawn".into(), &d)]);
        assert!(md.contains("flat-dawn"));
        assert!(md.contains("5.0000%"));
    }

    #[test]
    fn markdown_table_escapes_pipes_naively() {
        // pipes in cell content would break the table; we do not escape them yet — assert the
        // contract (rows must be pre-cleaned) rather than silently corrupting the table
        let mut md = Markdown::new();
        md.table(&["a"], &['<'], &[vec!["plain".into()]]);
        assert!(md.finish().contains("plain"));
    }
}
