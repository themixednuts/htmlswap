use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use arcstr::ArcStr;
use tempfile::Builder;
use wait_timeout::ChildExt;

use crate::diagnostics::{Compilation, Diagnostic, Diagnostics};
use crate::style_provider::{
    GeneratedStyleSource, StyleProvider, StyleProviderInput, StyleProviderOutput,
};

pub const DEFAULT_TAILWIND_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_TAILWIND_MAX_INPUT_BYTES: usize = 2 * 1024 * 1024;
pub const DEFAULT_TAILWIND_MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
pub const DEFAULT_TAILWIND_STYLESHEET: &str = "@import \"tailwindcss\" source(none);";
pub const DEFAULT_TAILWIND_SOURCE_NAME: &str = "tailwindcss-v4:generated.css";

/// Input passed to a Tailwind v4 engine.
#[derive(Debug, Clone, Copy)]
pub struct TailwindInput<'a> {
    pub stylesheet: &'a str,
    pub candidates: &'a [String],
}

/// The replaceable engine behind [`TailwindProvider`].
///
/// The supported implementation is [`TailwindCli`]. A future native engine can
/// implement this trait without changing the compiler or adapter APIs.
pub trait TailwindEngine: Send + Sync {
    fn name(&self) -> &str;

    fn compile(&self, input: TailwindInput<'_>) -> Result<ArcStr, TailwindError>;
}

/// Expands literal HTML classes and an explicit safelist through Tailwind v4.
pub struct TailwindProvider {
    engine: Arc<dyn TailwindEngine>,
    stylesheet: ArcStr,
    safelist: BTreeSet<String>,
    source_name: String,
}

impl TailwindProvider {
    #[must_use]
    pub fn new(engine: impl TailwindEngine + 'static) -> Self {
        Self::with_engine_arc(Arc::new(engine))
    }

    #[must_use]
    pub fn with_engine_arc(engine: Arc<dyn TailwindEngine>) -> Self {
        Self {
            engine,
            stylesheet: ArcStr::from(DEFAULT_TAILWIND_STYLESHEET),
            safelist: BTreeSet::new(),
            source_name: DEFAULT_TAILWIND_SOURCE_NAME.to_owned(),
        }
    }

    /// Replaces the CSS-first Tailwind configuration.
    ///
    /// Relative imports, `@config`, `@plugin`, and `@source` paths resolve from
    /// the CLI engine's working directory. The provider always adds its exact
    /// HTML candidates as an additional explicit source.
    #[must_use]
    pub fn with_stylesheet(mut self, stylesheet: impl Into<ArcStr>) -> Self {
        self.stylesheet = stylesheet.into();
        self
    }

    #[must_use]
    pub fn with_safelist<I, S>(mut self, candidates: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.safelist.extend(candidates.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn with_source_name(mut self, source_name: impl Into<String>) -> Self {
        self.source_name = source_name.into();
        self
    }

    #[must_use]
    pub fn engine(&self) -> &dyn TailwindEngine {
        self.engine.as_ref()
    }
}

impl Default for TailwindProvider {
    fn default() -> Self {
        Self::new(TailwindCli::default())
    }
}

impl std::fmt::Debug for TailwindProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TailwindProvider")
            .field("engine", &self.engine.name())
            .field("stylesheet", &self.stylesheet)
            .field("safelist", &self.safelist)
            .field("source_name", &self.source_name)
            .finish()
    }
}

impl StyleProvider for TailwindProvider {
    fn name(&self) -> &str {
        "tailwindcss-v4"
    }

    fn expand(&self, input: StyleProviderInput<'_>) -> Compilation<StyleProviderOutput> {
        let mut diagnostics = Diagnostics::new();
        let mut candidates = input
            .classes
            .names()
            .map(str::to_owned)
            .chain(self.safelist.iter().cloned())
            .collect::<BTreeSet<_>>();

        candidates.retain(|candidate| {
            if candidate.is_empty() || candidate.chars().any(char::is_whitespace) {
                diagnostics.push(Diagnostic::error(
                    format!("invalid Tailwind class candidate `{candidate}`"),
                    None,
                ));
                false
            } else {
                true
            }
        });

        if diagnostics.has_errors() {
            return Compilation::new(StyleProviderOutput::new(), diagnostics);
        }

        let candidates = candidates.into_iter().collect::<Vec<_>>();
        match self.engine.compile(TailwindInput {
            stylesheet: &self.stylesheet,
            candidates: &candidates,
        }) {
            Ok(css) => Compilation::new(
                StyleProviderOutput::new()
                    .with_stylesheet(GeneratedStyleSource::new(self.source_name.clone(), css)),
                diagnostics,
            ),
            Err(error) => {
                diagnostics.push(Diagnostic::error(
                    format!("tailwind style provider failed: {error}"),
                    None,
                ));
                Compilation::new(StyleProviderOutput::new(), diagnostics)
            }
        }
    }
}

/// A bounded, one-shot Tailwind v4 CLI engine.
///
/// The command is executed directly without a shell. It receives CSS over
/// stdin, writes CSS to an isolated temporary file, and is killed if it exceeds
/// the configured timeout. No package installation or network access is
/// attempted by htmlswap; point `executable` at a preinstalled CLI or shim.
#[derive(Debug, Clone)]
pub struct TailwindCli {
    executable: PathBuf,
    arguments: Vec<OsString>,
    working_directory: Option<PathBuf>,
    timeout: Duration,
    max_input_bytes: usize,
    max_output_bytes: usize,
}

impl TailwindCli {
    #[must_use]
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            arguments: Vec::new(),
            working_directory: None,
            timeout: DEFAULT_TAILWIND_TIMEOUT,
            max_input_bytes: DEFAULT_TAILWIND_MAX_INPUT_BYTES,
            max_output_bytes: DEFAULT_TAILWIND_MAX_OUTPUT_BYTES,
        }
    }

    #[must_use]
    pub fn with_argument(mut self, argument: impl Into<OsString>) -> Self {
        self.arguments.push(argument.into());
        self
    }

    #[must_use]
    pub fn with_arguments<I, S>(mut self, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.arguments.extend(arguments.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn with_working_directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.working_directory = Some(directory.into());
        self
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[must_use]
    pub fn with_max_input_bytes(mut self, max_input_bytes: usize) -> Self {
        self.max_input_bytes = max_input_bytes;
        self
    }

    #[must_use]
    pub fn with_max_output_bytes(mut self, max_output_bytes: usize) -> Self {
        self.max_output_bytes = max_output_bytes;
        self
    }

    fn working_directory(&self) -> Result<PathBuf, TailwindError> {
        match &self.working_directory {
            Some(directory) => Ok(directory.clone()),
            None => std::env::current_dir().map_err(TailwindError::CurrentDirectory),
        }
    }

    fn compile_inner(&self, input: TailwindInput<'_>) -> Result<ArcStr, TailwindError> {
        let directory = self.working_directory()?;
        if !directory.is_dir() {
            return Err(TailwindError::InvalidWorkingDirectory(directory));
        }

        let workspace = Builder::new()
            .prefix("htmlswap-tailwind-")
            .tempdir()
            .map_err(TailwindError::CreateWorkspace)?;
        let candidates_path = workspace.path().join("candidates.html");
        let output_path = workspace.path().join("output.css");
        let stderr_path = workspace.path().join("stderr.log");

        let candidate_source = input.candidates.join("\n");
        let stylesheet = format!(
            "{}\n@source \"{}\";\n",
            input.stylesheet,
            css_path(&candidates_path)
        );
        let input_bytes = candidate_source.len().checked_add(stylesheet.len()).ok_or(
            TailwindError::InputTooLarge {
                actual: usize::MAX,
                maximum: self.max_input_bytes,
            },
        )?;
        if input_bytes > self.max_input_bytes {
            return Err(TailwindError::InputTooLarge {
                actual: input_bytes,
                maximum: self.max_input_bytes,
            });
        }

        fs::write(&candidates_path, candidate_source).map_err(TailwindError::WriteCandidates)?;
        let stderr = File::create(&stderr_path).map_err(TailwindError::CreateStderr)?;
        let mut command = Command::new(&self.executable);
        command
            .args(&self.arguments)
            .arg("--input")
            .arg("-")
            .arg("--output")
            .arg(&output_path)
            .arg("--cwd")
            .arg(&directory)
            .arg("--optimize")
            .arg("--silent")
            .current_dir(&directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr));

        let mut child = command.spawn().map_err(|source| TailwindError::Spawn {
            executable: self.executable.clone(),
            source,
        })?;
        let child_stdin = child.stdin.take().ok_or(TailwindError::MissingStdin)?;
        let input_writer = write_stdin(child_stdin, stylesheet.into_bytes());

        let status = match child
            .wait_timeout(self.timeout)
            .map_err(TailwindError::Wait)?
        {
            Some(status) => status,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = input_writer.join();
                return Err(TailwindError::TimedOut(self.timeout));
            }
        };

        match input_writer.join() {
            Ok(Ok(())) => {}
            Ok(Err(source)) if status.success() => return Err(TailwindError::WriteStdin(source)),
            Ok(Err(_)) => {}
            Err(_) => return Err(TailwindError::InputWriterPanicked),
        }

        if !status.success() {
            return Err(TailwindError::Failed {
                executable: self.executable.clone(),
                status,
                stderr: read_diagnostic(&stderr_path),
            });
        }

        let metadata = fs::metadata(&output_path).map_err(TailwindError::ReadOutput)?;
        let actual = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
        if actual > self.max_output_bytes {
            return Err(TailwindError::OutputTooLarge {
                actual,
                maximum: self.max_output_bytes,
            });
        }

        let css = fs::read_to_string(output_path).map_err(TailwindError::ReadOutput)?;
        Ok(ArcStr::from(css))
    }
}

impl Default for TailwindCli {
    fn default() -> Self {
        Self::new("tailwindcss")
    }
}

impl TailwindEngine for TailwindCli {
    fn name(&self) -> &str {
        "tailwindcss-v4-cli"
    }

    fn compile(&self, input: TailwindInput<'_>) -> Result<ArcStr, TailwindError> {
        self.compile_inner(input)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TailwindError {
    #[error("read current working directory: {0}")]
    CurrentDirectory(#[source] io::Error),

    #[error("tailwind working directory `{}` is not a directory", .0.display())]
    InvalidWorkingDirectory(PathBuf),

    #[error("create isolated Tailwind workspace: {0}")]
    CreateWorkspace(#[source] io::Error),

    #[error("tailwind input is {actual} bytes, exceeding the {maximum}-byte limit")]
    InputTooLarge { actual: usize, maximum: usize },

    #[error("write Tailwind candidate source: {0}")]
    WriteCandidates(#[source] io::Error),

    #[error("create Tailwind diagnostic output: {0}")]
    CreateStderr(#[source] io::Error),

    #[error("spawn Tailwind CLI `{}`: {source}", executable.display())]
    Spawn {
        executable: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("tailwind CLI stdin was not available")]
    MissingStdin,

    #[error("write Tailwind CLI input: {0}")]
    WriteStdin(#[source] io::Error),

    #[error("tailwind CLI input writer panicked")]
    InputWriterPanicked,

    #[error("wait for Tailwind CLI: {0}")]
    Wait(#[source] io::Error),

    #[error("tailwind CLI exceeded its {0:?} timeout")]
    TimedOut(Duration),

    #[error(
        "tailwind CLI `{}` failed with {status}: {stderr}",
        executable.display()
    )]
    Failed {
        executable: PathBuf,
        status: ExitStatus,
        stderr: String,
    },

    #[error("read Tailwind CSS output: {0}")]
    ReadOutput(#[source] io::Error),

    #[error("tailwind output is {actual} bytes, exceeding the {maximum}-byte limit")]
    OutputTooLarge { actual: usize, maximum: usize },

    #[error("tailwind engine failed: {message}")]
    Engine { message: String },
}

impl TailwindError {
    #[must_use]
    pub fn engine(message: impl Into<String>) -> Self {
        Self::Engine {
            message: message.into(),
        }
    }
}

fn write_stdin(
    mut stdin: impl Write + Send + 'static,
    bytes: Vec<u8>,
) -> thread::JoinHandle<io::Result<()>> {
    thread::spawn(move || stdin.write_all(&bytes))
}

fn css_path(path: &Path) -> String {
    path.as_os_str()
        .to_string_lossy()
        .replace('\\', "/")
        .replace('"', "\\\"")
}

fn read_diagnostic(path: &Path) -> String {
    const MAX_DIAGNOSTIC_BYTES: usize = 64 * 1024;

    let Ok(bytes) = fs::read(path) else {
        return "no diagnostic output".to_owned();
    };
    let truncated = bytes.len() > MAX_DIAGNOSTIC_BYTES;
    let bytes = &bytes[..bytes.len().min(MAX_DIAGNOSTIC_BYTES)];
    let message = String::from_utf8_lossy(bytes).trim().to_owned();
    match (message.is_empty(), truncated) {
        (true, _) => "no diagnostic output".to_owned(),
        (false, true) => format!("{message} … [truncated]"),
        (false, false) => message,
    }
}
