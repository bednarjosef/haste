//! Error type of the crate.
use std::path::PathBuf;

/// Errors from starting or talking to the JVM.
#[derive(Debug, thiserror::Error)]
pub enum JvmError {
    /// A file the host needs (JDK, client jar, libraries) is missing.
    #[error("missing {what}: {path}")]
    Missing {
        /// What was looked for.
        what: &'static str,
        /// Where.
        path: PathBuf,
    },
    /// Compiling the Java side (`build-java.sh`) failed.
    #[error("building haste-jvm's Java sources failed:\n{0}")]
    JavaBuild(String),
    /// The JVM could not be created.
    #[error("starting the JVM failed: {0}")]
    Start(String),
    /// `JvmHost::start` was called again with a different configuration (one JVM per process).
    #[error("the JVM is already running with a different configuration")]
    AlreadyStarted,
    /// A JNI call failed; Java exceptions carry their class, message and stack trace.
    #[error("JNI: {0}")]
    Jni(#[from] jni::errors::Error),
    /// A protocol error between Rust and the Java bridge (unexpected frame layout etc.).
    #[error("bridge: {0}")]
    Bridge(String),
}

/// Result alias.
pub type Result<T, E = JvmError> = std::result::Result<T, E>;
