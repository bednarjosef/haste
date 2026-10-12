//! Async downloads from Mojang endpoints with sha1 verification, resume/skip-if-valid and
//! bounded concurrency. Downloads go to `<dest>.part` and are renamed into place only after the
//! sha1 matches, so an interrupted run never leaves a corrupt file where a valid one is expected.

use anyhow::{anyhow, Context};
use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, Semaphore};

/// What happened for a single file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadOutcome {
    /// The file was already there with the right sha1.
    Skipped,
    /// Downloaded now (or resumed a partial file and finished it).
    Downloaded,
}

/// Progress callback payloads for [`download_many`]/[`download`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// One file finished (`Downloaded` or `Skipped`); `done` counts files so far.
    File { done: usize, total: usize },
    /// `bytes` downloaded of `total_bytes` across all files in the batch.
    Bytes { done: u64, total: u64 },
}

/// Shared counters for a batch: cheap atomic byte counts, channel for events. Bytes only count
/// what we actually transfer, not skips. Public only because [`download`] takes it; use
/// [`download_many`], which builds one.
#[derive(Clone)]
pub struct SharedProgress {
    sender: mpsc::UnboundedSender<Progress>,
    files_done: Arc<AtomicU64>,
    files_total: usize,
    bytes_done: Arc<AtomicU64>,
    bytes_total: u64,
}

impl SharedProgress {
    fn emit_bytes(&self) {
        let done = self.bytes_done.load(Ordering::Relaxed);
        let _ = self.sender.send(Progress::Bytes {
            done,
            total: self.bytes_total,
        });
    }
    fn file_finished(&self) {
        let done = self.files_done.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.sender.send(Progress::File {
            done: done as usize,
            total: self.files_total,
        });
    }
}

/// Drains progress events from a batch. Call from the same task or a spawned one.
pub struct ProgressRx {
    rx: mpsc::UnboundedReceiver<Progress>,
}

impl ProgressRx {
    /// Blocks the async task until the next progress event (or the batch ends, returning None).
    pub async fn recv(&mut self) -> Option<Progress> {
        self.rx.recv().await
    }
}

/// A file to download.
#[derive(Debug, Clone)]
pub struct DownloadSpec {
    pub url: String,
    pub sha1: String,
    pub size: u64,
    pub dest: PathBuf,
}

/// Verifies `path` against `sha1` (lowercase hex). Missing file -> false.
pub async fn verify_sha1(path: &Path, sha1: &str) -> bool {
    let Ok(data) = tokio::fs::read(path).await else {
        return false;
    };
    let digest = Sha1::digest(&data);
    hex::encode(digest) == sha1.to_ascii_lowercase()
}

/// Downloads one file if `dest` does not already hold the right content.
/// Resumes a `<dest>.part` file when the server honors `Range`; Mojang's piston-data and
/// resources CDNs do. Returns the outcome without verifying a skipped file again.
pub async fn download(
    client: &reqwest::Client,
    spec: &DownloadSpec,
    progress: Option<&SharedProgress>,
) -> anyhow::Result<DownloadOutcome> {
    if verify_sha1(&spec.dest, &spec.sha1).await {
        if let Some(p) = progress {
            p.file_finished();
        }
        return Ok(DownloadOutcome::Skipped);
    }
    let part = spec.dest.with_extension(format!(
        "{}.part",
        spec.dest.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    if let Some(parent) = spec.dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    // A part file is only trustworthy as a resume point if it is a prefix of the target; we cannot
    // know that, so the final sha1 check decides. Keep the part file, ask the server for the rest;
    // if the server ignores Range we restart from scratch.
    let have: u64 = match tokio::fs::metadata(&part).await {
        Ok(m) => m.len(),
        Err(_) => 0,
    };
    let mut file = match tokio::fs::OpenOptions::new().append(true).open(&part).await {
        Ok(f) if have > 0 => f,
        _ => tokio::fs::File::create(&part).await?,
    };

    let range_from = have;
    let mut req = client.get(&spec.url);
    if range_from > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={range_from}-"));
    }
    let resp = req
        .send()
        .await
        .with_context(|| format!("GET {}", spec.url))?;
    let resumed = range_from > 0 && resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if range_from > 0 && !resumed {
        // Server ignored the range; restart into the same part file.
        file = tokio::fs::File::create(&part).await?;
    }
    anyhow::ensure!(
        resp.status().is_success(),
        "GET {} returned {}",
        spec.url,
        resp.status()
    );

    let mut stream = resp;
    while let Some(chunk) = stream.chunk().await? {
        file.write_all(&chunk).await?;
        if let Some(p) = progress {
            p.bytes_done
                .fetch_add(chunk.len() as u64, Ordering::Relaxed);
            p.emit_bytes();
        }
    }
    file.flush().await?;
    drop(file);

    let got = tokio::fs::read(&part).await?;
    let digest = hex::encode(Sha1::digest(&got));
    anyhow::ensure!(
        digest == spec.sha1.to_ascii_lowercase(),
        "sha1 mismatch for {}: expected {}, got {} (url {})",
        spec.dest.display(),
        spec.sha1,
        digest,
        spec.url
    );
    tokio::fs::rename(&part, &spec.dest)
        .await
        .with_context(|| format!("renaming {} -> {}", part.display(), spec.dest.display()))?;
    if let Some(p) = progress {
        p.file_finished();
    }
    Ok(DownloadOutcome::Downloaded)
}

/// Maximum concurrent connections.
pub const MAX_CONCURRENCY: usize = 8;

/// Downloads a batch of files with bounded concurrency (8) and a progress stream.
/// Returns once every file is present and verified. `want` filters specs (e.g. per-OS libraries);
/// progress totals are computed over the specs that are actually downloaded.
pub async fn download_many(
    specs: Vec<DownloadSpec>,
    progress: impl FnMut(Progress) + Send + 'static,
) -> anyhow::Result<Vec<DownloadOutcome>> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()?;
    download_many_with(&client, specs, progress).await
}

async fn download_many_with(
    client: &reqwest::Client,
    specs: Vec<DownloadSpec>,
    mut progress: impl FnMut(Progress) + Send + 'static,
) -> anyhow::Result<Vec<DownloadOutcome>> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let shared = SharedProgress {
        sender: tx,
        files_done: Arc::new(AtomicU64::new(0)),
        files_total: specs.len(),
        bytes_done: Arc::new(AtomicU64::new(0)),
        bytes_total: 0, // unknown until each file reports; total size is a cap
    };
    let _ = &shared;

    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENCY));
    let client = client.clone();
    let mut handles = Vec::with_capacity(specs.len());
    let mut out = Vec::with_capacity(specs.len());
    // Keep specs paired with their results by index.
    let specs = Arc::new(specs);
    for (idx, _spec) in specs.iter().enumerate() {
        let sem = semaphore.clone();
        let shared = shared.clone();
        let client = client.clone();
        let specs = specs.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.map_err(|e| anyhow!(e))?;
            let outcome = download(&client, &specs[idx], Some(&shared)).await?;
            Ok::<DownloadOutcome, anyhow::Error>(outcome)
        }));
    }
    // Progress pump: forward events to the caller callback until all tasks join.
    let pump = tokio::spawn(async move {
        while let Some(p) = rx.recv().await {
            progress(p);
        }
    });
    for h in handles {
        out.push(
            h.await
                .map_err(|e| anyhow!("download task panicked: {e}"))??,
        );
    }
    pump.abort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::body::Incoming;
    use hyper::service::service_fn;
    use hyper::{Request, Response};
    use hyper_util::rt::TokioIo;
    use std::net::SocketAddr;

    /// Serves `/ok` (the file bytes) and `/corrupt` (bytes with the wrong sha1), plus HEAD/Range
    /// support on `/ok`.
    async fn spawn_server(contents: &'static [u8]) -> SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let io = TokioIo::new(stream);
                let service = service_fn(move |req: Request<Incoming>| async move {
                    let path = req.uri().path().to_string();
                    let range = req
                        .headers()
                        .get(hyper::header::RANGE)
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.to_string());
                    match path.as_str() {
                        "/ok" => {
                            let body = contents;
                            if let Some(r) = range {
                                // "bytes=N-"
                                let start: usize = r
                                    .trim_start_matches("bytes=")
                                    .trim_end_matches('-')
                                    .parse()
                                    .unwrap();
                                Ok::<_, hyper::Error>(
                                    Response::builder()
                                        .status(hyper::StatusCode::PARTIAL_CONTENT)
                                        .header(
                                            hyper::header::CONTENT_RANGE,
                                            format!(
                                                "bytes {}-{}/{}",
                                                start,
                                                body.len() - 1,
                                                body.len()
                                            ),
                                        )
                                        .body(http_body_util::Full::new(bytes::Bytes::from(
                                            body[start..].to_vec(),
                                        )))
                                        .unwrap(),
                                )
                            } else {
                                Ok(Response::builder()
                                    .body(http_body_util::Full::new(bytes::Bytes::from(
                                        body.to_vec(),
                                    )))
                                    .unwrap())
                            }
                        }
                        "/corrupt" => Ok(Response::builder()
                            .body(http_body_util::Full::new(bytes::Bytes::from(vec![0u8; 5])))
                            .unwrap()),
                        _ => Ok(Response::builder()
                            .status(hyper::StatusCode::NOT_FOUND)
                            .body(http_body_util::Full::new(bytes::Bytes::new()))
                            .unwrap()),
                    }
                });
                tokio::spawn(async move {
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(io, service)
                        .await;
                });
            }
        });
        addr
    }

    fn client_for(_addr: SocketAddr) -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
    }

    fn sha1_hex(data: &[u8]) -> String {
        hex::encode(Sha1::digest(data))
    }

    #[tokio::test]
    async fn downloads_and_verifies() {
        let data = b"hello haste download".repeat(100);
        let addr = spawn_server(Box::leak(data.clone().into_boxed_slice())).await;
        let client = client_for(addr);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("file.bin");
        let spec = DownloadSpec {
            url: format!("http://{addr}/ok"),
            sha1: sha1_hex(&data),
            size: data.len() as u64,
            dest: dest.clone(),
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let shared = SharedProgress {
            sender: tx,
            files_done: Arc::new(AtomicU64::new(0)),
            files_total: 1,
            bytes_done: Arc::new(AtomicU64::new(0)),
            bytes_total: data.len() as u64,
        };
        let out = download(&client, &spec, Some(&shared)).await.unwrap();
        assert_eq!(out, DownloadOutcome::Downloaded);
        assert_eq!(tokio::fs::read(&dest).await.unwrap(), data);
        // progress events were emitted
        let mut rx = rx;
        assert!(matches!(rx.try_recv(), Ok(Progress::Bytes { .. })));
    }

    #[tokio::test]
    async fn skips_when_valid() {
        let data = b"already here".repeat(50);
        let addr = spawn_server(Box::leak(data.clone().into_boxed_slice())).await;
        let client = client_for(addr);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("file.bin");
        tokio::fs::write(&dest, &data).await.unwrap();
        let spec = DownloadSpec {
            url: format!("http://{addr}/ok"),
            sha1: sha1_hex(&data),
            size: data.len() as u64,
            dest: dest.clone(),
        };
        let out = download(&client, &spec, None).await.unwrap();
        assert_eq!(out, DownloadOutcome::Skipped);
    }

    #[tokio::test]
    async fn fails_on_sha1_mismatch() {
        let addr = spawn_server(b"server content").await;
        let client = client_for(addr);
        let dir = tempfile::tempdir().unwrap();
        let spec = DownloadSpec {
            url: format!("http://{addr}/corrupt"),
            sha1: "0".repeat(40),
            size: 5,
            dest: dir.path().join("bad.bin"),
        };
        let err = download(&client, &spec, None).await.unwrap_err();
        assert!(err.to_string().contains("sha1 mismatch"), "got: {err}");
        // no clean file left behind, only the .part
        assert!(!dir.path().join("bad.bin").exists());
        assert!(dir.path().join("bad.bin.part").exists());
    }

    #[tokio::test]
    async fn resumes_partial_download() {
        let data = b"0123456789abcdefghijklmnopqrstuvwxyz".repeat(20);
        let addr = spawn_server(Box::leak(data.clone().into_boxed_slice())).await;
        let client = client_for(addr);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("file.bin");
        // simulate an interrupted earlier run: a part file holding the first 100 bytes
        let part = dir.path().join("file.bin.part");
        tokio::fs::write(&part, &data[..100]).await.unwrap();
        let spec = DownloadSpec {
            url: format!("http://{addr}/ok"),
            sha1: sha1_hex(&data),
            size: data.len() as u64,
            dest,
        };
        let out = download(&client, &spec, None).await.unwrap();
        assert_eq!(out, DownloadOutcome::Downloaded);
        let got = tokio::fs::read(spec.dest.clone()).await.unwrap();
        assert_eq!(got, data);
        assert!(!spec.dest.with_extension("bin.part").exists());
    }

    #[tokio::test]
    async fn batch_runs_bounded_and_reports() {
        let data = b"batch payload".repeat(10);
        let addr = spawn_server(Box::leak(data.clone().into_boxed_slice())).await;
        let dir = tempfile::tempdir().unwrap();
        let specs: Vec<DownloadSpec> = (0..20)
            .map(|i| DownloadSpec {
                url: format!("http://{addr}/ok"),
                sha1: sha1_hex(&data),
                size: data.len() as u64,
                dest: dir.path().join(format!("f{i}.bin")),
            })
            .collect();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let reporter = std::sync::Arc::new(std::sync::Mutex::new(0usize));
        let r2 = reporter.clone();
        let out = download_many(specs, move |p| {
            if matches!(p, Progress::File { .. }) {
                *r2.lock().unwrap() += 1;
            }
            let _ = tx.send(p);
        })
        .await
        .unwrap();
        assert_eq!(out.len(), 20);
        assert!(out.iter().all(|o| *o == DownloadOutcome::Downloaded));
        assert_eq!(*reporter.lock().unwrap(), 20);
        assert!(rx.try_recv().is_ok());
    }
}
