use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use arcstr::ArcStr;
use url::Url;

use crate::diagnostics::{Compilation, Diagnostic, Diagnostics};
use crate::source::{SourceId, SourceKind, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    Stylesheet,
    Script,
    Image,
    Other,
}

impl ResourceKind {
    #[must_use]
    pub const fn source_kind(self) -> SourceKind {
        match self {
            Self::Stylesheet => SourceKind::Css,
            Self::Script => SourceKind::JavaScript,
            Self::Image | Self::Other => SourceKind::Unknown,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stylesheet => "stylesheet",
            Self::Script => "script",
            Self::Image => "image",
            Self::Other => "resource",
        }
    }
}

impl fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRequest {
    pub kind: ResourceKind,
    pub specifier: ArcStr,
    pub referrer: Option<ResourceReferrer>,
    pub span: Option<Span>,
}

impl ResourceRequest {
    #[must_use]
    pub fn new(kind: ResourceKind, specifier: impl Into<ArcStr>) -> Self {
        Self {
            kind,
            specifier: specifier.into(),
            referrer: None,
            span: None,
        }
    }

    #[must_use]
    pub fn with_referrer(mut self, referrer: impl Into<Option<ResourceReferrer>>) -> Self {
        self.referrer = referrer.into();
        self
    }

    #[must_use]
    pub fn with_span(mut self, span: impl Into<Option<Span>>) -> Self {
        self.span = span.into();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceReferrer {
    pub source_id: SourceId,
    pub name: Option<ArcStr>,
}

impl ResourceReferrer {
    #[must_use]
    pub fn new(source_id: SourceId, name: impl Into<Option<ArcStr>>) -> Self {
        Self {
            source_id,
            name: name.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceSource {
    pub kind: ResourceKind,
    pub name: Option<String>,
    pub contents: ArcStr,
}

impl ResourceSource {
    #[must_use]
    pub fn new(
        kind: ResourceKind,
        name: impl Into<Option<String>>,
        contents: impl Into<ArcStr>,
    ) -> Self {
        Self {
            kind,
            name: name.into(),
            contents: contents.into(),
        }
    }
}

pub trait ResourceResolver: Send + Sync {
    fn can_resolve(&self, request: &ResourceRequest) -> bool;

    fn resolve(&self, request: &ResourceRequest) -> Compilation<Option<ResourceSource>>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoopResourceResolver;

impl NoopResourceResolver {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ResourceResolver for NoopResourceResolver {
    fn can_resolve(&self, _request: &ResourceRequest) -> bool {
        false
    }

    fn resolve(&self, _request: &ResourceRequest) -> Compilation<Option<ResourceSource>> {
        Compilation::clean(None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpResourceResolverOptions {
    pub timeout: Duration,
    pub max_bytes: usize,
}

impl HttpResourceResolverOptions {
    #[must_use]
    pub fn new(timeout: Duration, max_bytes: usize) -> Self {
        Self { timeout, max_bytes }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileSystemResourceResolverOptions {
    pub max_bytes: usize,
}

impl FileSystemResourceResolverOptions {
    #[must_use]
    pub const fn new(max_bytes: usize) -> Self {
        Self { max_bytes }
    }
}

#[derive(Debug, Clone)]
pub struct DefaultResourceResolver {
    http: Option<HttpResourceResolver>,
    file_system: Option<FileSystemResourceResolver>,
}

impl DefaultResourceResolver {
    #[must_use]
    pub fn new(
        http: impl Into<Option<HttpResourceResolver>>,
        file_system: impl Into<Option<FileSystemResourceResolver>>,
    ) -> Self {
        Self {
            http: http.into(),
            file_system: file_system.into(),
        }
    }
}

impl ResourceResolver for DefaultResourceResolver {
    fn can_resolve(&self, request: &ResourceRequest) -> bool {
        self.http
            .as_ref()
            .is_some_and(|resolver| resolver.can_resolve(request))
            || self
                .file_system
                .as_ref()
                .is_some_and(|resolver| resolver.can_resolve(request))
    }

    fn resolve(&self, request: &ResourceRequest) -> Compilation<Option<ResourceSource>> {
        if let Some(http) = &self.http
            && http.can_resolve(request)
        {
            return http.resolve(request);
        }

        if let Some(file_system) = &self.file_system
            && file_system.can_resolve(request)
        {
            return file_system.resolve(request);
        }

        Compilation::clean(None)
    }
}

#[derive(Debug, Clone)]
pub struct HttpResourceResolver {
    options: HttpResourceResolverOptions,
    agent: ureq::Agent,
}

impl HttpResourceResolver {
    #[must_use]
    pub fn new(options: HttpResourceResolverOptions) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(options.timeout))
            .build()
            .new_agent();
        Self { options, agent }
    }
}

#[derive(Debug, Clone)]
pub struct FileSystemResourceResolver {
    options: FileSystemResourceResolverOptions,
}

impl FileSystemResourceResolver {
    #[must_use]
    pub const fn new(options: FileSystemResourceResolverOptions) -> Self {
        Self { options }
    }
}

impl ResourceResolver for FileSystemResourceResolver {
    fn can_resolve(&self, request: &ResourceRequest) -> bool {
        request_path(request).is_some()
    }

    fn resolve(&self, request: &ResourceRequest) -> Compilation<Option<ResourceSource>> {
        let mut diagnostics = Diagnostics::new();
        let Some(path) = request_path(request) else {
            return Compilation::new(None, diagnostics);
        };

        match read_bounded_text(&path, self.options.max_bytes) {
            Ok(contents) => Compilation::new(
                Some(ResourceSource::new(
                    request.kind,
                    Some(file_source_name(&path)),
                    ArcStr::from(contents),
                )),
                diagnostics,
            ),
            Err(error) => {
                diagnostics.push(Diagnostic::warning(
                    format!(
                        "failed to resolve {} `{}`: {error}",
                        request.kind,
                        path.display()
                    ),
                    request.span,
                ));
                Compilation::new(None, diagnostics)
            }
        }
    }
}

impl ResourceResolver for HttpResourceResolver {
    fn can_resolve(&self, request: &ResourceRequest) -> bool {
        request_url(request).is_some()
    }

    fn resolve(&self, request: &ResourceRequest) -> Compilation<Option<ResourceSource>> {
        let mut diagnostics = Diagnostics::new();
        let Some(url) = request_url(request) else {
            return Compilation::new(None, diagnostics);
        };

        match fetch_url(&self.agent, &url, self.options.max_bytes as u64) {
            Ok(contents) => Compilation::new(
                Some(ResourceSource::new(
                    request.kind,
                    Some(url.to_string()),
                    ArcStr::from(contents),
                )),
                diagnostics,
            ),
            Err(error) => {
                diagnostics.push(Diagnostic::warning(
                    format!("failed to resolve {} `{}`: {error}", request.kind, url),
                    request.span,
                ));
                Compilation::new(None, diagnostics)
            }
        }
    }
}

fn read_bounded_text(path: &Path, max_bytes: usize) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(io::Error::other(format!(
            "resource exceeds maximum size of {max_bytes} bytes"
        )));
    }

    String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn fetch_url(agent: &ureq::Agent, url: &Url, max_bytes: u64) -> Result<String, ureq::Error> {
    let bytes = agent
        .get(url.as_str())
        .call()?
        .body_mut()
        .with_config()
        .limit(max_bytes)
        .read_to_vec()?;

    String::from_utf8(bytes)
        .map_err(|error| ureq::Error::Io(io::Error::new(io::ErrorKind::InvalidData, error)))
}

fn request_path(request: &ResourceRequest) -> Option<PathBuf> {
    let specifier = request.specifier.trim();
    if specifier.is_empty() {
        return None;
    }

    if let Ok(url) = Url::parse(specifier) {
        return file_url_path(&url);
    }

    let specifier_path = Path::new(specifier);
    if specifier_path.is_absolute() {
        return Some(specifier_path.to_path_buf());
    }

    let base = match &request.referrer {
        Some(referrer) => referrer.name.as_deref().and_then(referrer_directory)?,
        None => std::env::current_dir().ok()?,
    };

    Some(base.join(specifier_path))
}

fn referrer_directory(name: &str) -> Option<PathBuf> {
    if let Ok(url) = Url::parse(name) {
        return file_url_path(&url).and_then(|path| path.parent().map(Path::to_path_buf));
    }

    let path = Path::new(name);
    path.is_absolute()
        .then(|| path.parent().map(Path::to_path_buf))
        .flatten()
}

fn request_url(request: &ResourceRequest) -> Option<Url> {
    let specifier = request.specifier.trim();
    if let Ok(url) = Url::parse(specifier)
        && is_http_url(&url)
    {
        return Some(url);
    }

    request
        .referrer
        .as_ref()
        .and_then(|referrer| referrer.name.as_deref())
        .and_then(|name| Url::parse(name).ok())
        .filter(is_http_url)
        .and_then(|base| base.join(specifier).ok())
        .filter(is_http_url)
}

fn is_http_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

fn file_url_path(url: &Url) -> Option<PathBuf> {
    (url.scheme() == "file")
        .then(|| url.to_file_path().ok())
        .flatten()
}

fn file_source_name(path: &Path) -> String {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };

    Url::from_file_path(&absolute)
        .map(|url| url.to_string())
        .unwrap_or_else(|_| path.display().to_string())
}
