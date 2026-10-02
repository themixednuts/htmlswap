use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt;
use std::fs;
use std::io;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use arcstr::ArcStr;

use crate::adapter::AdapterArtifact;
use crate::adapters::svelte::SvelteAdapterOptions;
use crate::assets::CompileAssets;
use crate::bundle::{BundleConfig, BundlePlan};
use crate::class::ClassIndex;
use crate::css::{
    StyleIndex, Stylesheet, StylesheetImport, inline_stylesheets, parse_stylesheet_with_offset,
    parse_stylesheet_with_source,
};
use crate::dc_bundle::apply_root_dc_style_bindings;
use crate::diagnostics::{Compilation, Diagnostic, Diagnostics};
use crate::frontend::Frontend;
use crate::ir::{HtmlAttribute, HtmlDocument, HtmlElement, HtmlNode};
use crate::jsx;
use crate::lower::{LowerContext, LowerOptions, lower_document_with_context};
use crate::parse::{parse_document_with_source, parse_fragment_with_source};
use crate::plan::{RenderAnnotation, RenderPlan};
use crate::resource::{
    DefaultResourceResolver, FileSystemResourceResolver, FileSystemResourceResolverOptions,
    HttpResourceResolver, HttpResourceResolverOptions, NoopResourceResolver, ResourceKind,
    ResourceReferrer, ResourceRequest, ResourceResolver, ResourceResolverChain, ResourceSource,
};
use crate::script::{
    ActionIndex, ScriptImport, ScriptModule, ScriptStyleIndex, parse_inline_script_with_offset,
    parse_script_with_source,
};
use crate::source::{SourceFile, SourceId, SourceKind, SourceMap};
use crate::style_provider::{StyleProvider, StyleProviderInput, StyleProviderPipeline};
use crate::work::{JobContext, JobRunner, JobRunnerBuildError};

#[derive(Clone)]
pub struct Compiler {
    options: CompilerOptions,
    jobs: JobContext,
    cache: Option<CompilerCache>,
    resolver: Arc<dyn ResourceResolver>,
    frontend: Arc<Frontend>,
    style_providers: StyleProviderPipeline,
}

impl Compiler {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn try_with_options(options: CompilerOptions) -> Result<Self, CompilerBuildError> {
        let jobs = JobContext::new(
            job_runner_from_parallelism(options.parallelism)?,
            Default::default(),
        );
        Ok(Self {
            resolver: resource_resolver_from_options(&options),
            frontend: Arc::new(Frontend::default()),
            style_providers: StyleProviderPipeline::new(),
            options,
            jobs,
            cache: None,
        })
    }

    #[must_use]
    pub fn compile_jsx_svelte_project<I, N, S>(
        &self,
        sources: I,
        options: SvelteAdapterOptions,
    ) -> Compilation<AdapterArtifact>
    where
        I: IntoIterator<Item = (N, S)>,
        N: Into<String>,
        S: Into<ArcStr>,
    {
        let sources = sources
            .into_iter()
            .map(|(name, source)| jsx::JsxProjectSource::new(name, source))
            .collect();
        jsx::compile_svelte_project(sources, options)
    }

    #[must_use]
    pub fn with_jobs(options: CompilerOptions, jobs: JobContext) -> Self {
        Self {
            resolver: resource_resolver_from_options(&options),
            frontend: Arc::new(Frontend::default()),
            style_providers: StyleProviderPipeline::new(),
            options,
            jobs,
            cache: None,
        }
    }

    #[must_use]
    pub fn with_cache(mut self, cache: CompilerCache) -> Self {
        self.cache = Some(cache);
        self
    }

    #[must_use]
    pub fn with_resource_resolver(mut self, resolver: impl ResourceResolver + 'static) -> Self {
        self.resolver = Arc::new(resolver);
        self
    }

    #[must_use]
    pub fn with_resource_resolver_arc(mut self, resolver: Arc<dyn ResourceResolver>) -> Self {
        self.resolver = resolver;
        self
    }

    /// Adds a resolver after the current resolver without replacing it.
    #[must_use]
    pub fn with_fallback_resource_resolver(
        mut self,
        resolver: impl ResourceResolver + 'static,
    ) -> Self {
        self.resolver = Arc::new(
            ResourceResolverChain::new()
                .with_resolver_arc(self.resolver.clone())
                .with_resolver(resolver),
        );
        self
    }

    #[must_use]
    pub fn with_frontend(mut self, frontend: Frontend) -> Self {
        self.frontend = Arc::new(frontend);
        self
    }

    #[must_use]
    pub fn with_frontend_arc(mut self, frontend: Arc<Frontend>) -> Self {
        self.frontend = frontend;
        self
    }

    /// Appends a generated-style capability to the compiler pipeline.
    #[must_use]
    pub fn with_style_provider(mut self, provider: impl StyleProvider + 'static) -> Self {
        self.style_providers.push(provider);
        self
    }

    /// Appends a type-erased generated-style capability to the compiler pipeline.
    #[must_use]
    pub fn with_style_provider_arc(mut self, provider: Arc<dyn StyleProvider>) -> Self {
        self.style_providers.push_arc(provider);
        self
    }

    #[must_use]
    pub fn style_providers(&self) -> &StyleProviderPipeline {
        &self.style_providers
    }

    #[must_use]
    pub fn options(&self) -> &CompilerOptions {
        &self.options
    }

    #[must_use]
    pub fn jobs(&self) -> &JobContext {
        &self.jobs
    }

    #[must_use]
    pub fn cache(&self) -> Option<&CompilerCache> {
        self.cache.as_ref()
    }

    #[must_use]
    pub fn compile_fragment(
        &self,
        source: impl Into<ArcStr>,
        assets: &CompileAssets,
    ) -> Compilation<CompiledFragment> {
        self.compile_fragment_named(None, source, assets)
    }

    #[must_use]
    pub fn compile_fragment_named(
        &self,
        name: impl Into<Option<String>>,
        source: impl Into<ArcStr>,
        assets: &CompileAssets,
    ) -> Compilation<CompiledFragment> {
        compile_fragment_with_jobs_and_cache(
            name.into(),
            source.into(),
            assets,
            &self.jobs,
            self.resolver.as_ref(),
            self.frontend.as_ref(),
            self.options.source_frontend,
            self.options.source_policy,
            &self.options.bundle,
            self.cache.as_ref(),
            &self.style_providers,
        )
    }
}

impl Default for Compiler {
    fn default() -> Self {
        Self::try_with_options(CompilerOptions::default())
            .expect("default compiler options should be valid")
    }
}

impl fmt::Debug for Compiler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Compiler")
            .field("options", &self.options)
            .field("cache", &self.cache.as_ref().map(CompilerCache::stats))
            .field("frontend", &self.frontend)
            .field("style_providers", &self.style_providers)
            .finish()
    }
}

#[derive(Debug, Clone, Default)]
pub struct CompilerCache {
    inner: Arc<CompilerCacheInner>,
}

impl CompilerCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&self) {
        lock_cache(&self.inner.html).clear();
        lock_cache(&self.inner.stylesheets).clear();
        lock_cache(&self.inner.inline_stylesheets).clear();
        lock_cache(&self.inner.scripts).clear();
        lock_cache(&self.inner.inline_scripts).clear();
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stats() == CompilerCacheStats::default()
    }

    #[must_use]
    pub fn stats(&self) -> CompilerCacheStats {
        CompilerCacheStats {
            html_entries: lock_cache(&self.inner.html).len(),
            stylesheet_entries: lock_cache(&self.inner.stylesheets).len(),
            inline_stylesheet_entries: lock_cache(&self.inner.inline_stylesheets).len(),
            script_entries: lock_cache(&self.inner.scripts).len(),
            inline_script_entries: lock_cache(&self.inner.inline_scripts).len(),
        }
    }

    fn parse_html(&self, source: ArcStr, source_id: SourceId) -> Compilation<HtmlDocument> {
        let key = SourceCacheKey::new(SourceKind::Html, None, source.clone());
        if let Some(parsed) = lock_cache(&self.inner.html).get(&key).cloned() {
            return rebase_html_compilation(parsed, source_id);
        }

        let parsed = parse_html_with_source(source.as_str(), source_id);
        lock_cache(&self.inner.html).insert(key, parsed.clone());
        parsed
    }

    fn parse_stylesheet(
        &self,
        source: ArcStr,
        name: Option<&str>,
        source_id: SourceId,
    ) -> Compilation<Stylesheet> {
        let key = SourceCacheKey::new(SourceKind::Css, name, source.clone());
        if let Some(parsed) = lock_cache(&self.inner.stylesheets).get(&key).cloned() {
            return rebase_stylesheet_compilation(parsed, source_id);
        }

        let parsed = parse_stylesheet_with_source(source.as_str(), source_id);
        lock_cache(&self.inner.stylesheets).insert(key, parsed.clone());
        parsed
    }

    fn parse_inline_stylesheet(
        &self,
        source: ArcStr,
        source_id: SourceId,
        offset: usize,
    ) -> Compilation<Stylesheet> {
        let key = InlineSourceCacheKey::new(SourceKind::Css, source.clone(), offset);
        if let Some(parsed) = lock_cache(&self.inner.inline_stylesheets)
            .get(&key)
            .cloned()
        {
            return rebase_stylesheet_compilation(parsed, source_id);
        }

        let parsed = parse_stylesheet_with_offset(source.as_str(), source_id, offset);
        lock_cache(&self.inner.inline_stylesheets).insert(key, parsed.clone());
        parsed
    }

    fn parse_script(
        &self,
        source: ArcStr,
        name: Option<&str>,
        source_id: SourceId,
    ) -> Compilation<ScriptModule> {
        let key = SourceCacheKey::new(SourceKind::JavaScript, name, source.clone());
        if let Some(parsed) = lock_cache(&self.inner.scripts).get(&key).cloned() {
            return rebase_script_compilation(parsed, source_id);
        }

        let parsed = parse_script_with_source(source.as_str(), name, source_id);
        lock_cache(&self.inner.scripts).insert(key, parsed.clone());
        parsed
    }

    fn parse_inline_script(
        &self,
        source: ArcStr,
        source_id: SourceId,
        offset: usize,
    ) -> Compilation<ScriptModule> {
        let key = InlineSourceCacheKey::new(SourceKind::JavaScript, source.clone(), offset);
        if let Some(parsed) = lock_cache(&self.inner.inline_scripts).get(&key).cloned() {
            return rebase_script_compilation(parsed, source_id);
        }

        let parsed = parse_inline_script_with_offset(source.as_str(), source_id, offset);
        lock_cache(&self.inner.inline_scripts).insert(key, parsed.clone());
        parsed
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompilerCacheStats {
    pub html_entries: usize,
    pub stylesheet_entries: usize,
    pub inline_stylesheet_entries: usize,
    pub script_entries: usize,
    pub inline_script_entries: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerCacheFileStamp {
    pub path: PathBuf,
    pub len: u64,
    pub modified: Option<SystemTime>,
}

impl CompilerCacheFileStamp {
    pub fn read(path: impl AsRef<Path>) -> io::Result<Self> {
        let original = path.as_ref();
        let path = fs::canonicalize(original).unwrap_or_else(|_| original.to_path_buf());
        let metadata = fs::metadata(&path)?;
        Ok(Self {
            path,
            len: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }

    pub fn has_changed(&self) -> io::Result<bool> {
        let current = Self::read(&self.path)?;
        Ok(current.len != self.len || current.modified != self.modified)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompilerCacheFileSet {
    stamps: BTreeMap<PathBuf, CompilerCacheFileStamp>,
}

impl CompilerCacheFileSet {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read<I, P>(paths: I) -> io::Result<Self>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let mut file_set = Self::new();
        for path in paths {
            file_set.track(path)?;
        }
        Ok(file_set)
    }

    pub fn track(&mut self, path: impl AsRef<Path>) -> io::Result<()> {
        let stamp = CompilerCacheFileStamp::read(path)?;
        self.stamps.insert(stamp.path.clone(), stamp);
        Ok(())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stamps.is_empty()
    }

    pub fn stamps(&self) -> impl Iterator<Item = &CompilerCacheFileStamp> {
        self.stamps.values()
    }

    pub fn refresh_changed(&mut self) -> io::Result<bool> {
        let mut changed = false;
        let paths = self.stamps.keys().cloned().collect::<Vec<_>>();
        for path in paths {
            let current = CompilerCacheFileStamp::read(&path)?;
            if self.stamps.get(&path).is_none_or(|previous| {
                previous.len != current.len || previous.modified != current.modified
            }) {
                changed = true;
            }
            self.stamps.insert(path, current);
        }
        Ok(changed)
    }
}

#[derive(Debug, Default)]
struct CompilerCacheInner {
    html: Mutex<HashMap<SourceCacheKey, Compilation<HtmlDocument>>>,
    stylesheets: Mutex<HashMap<SourceCacheKey, Compilation<Stylesheet>>>,
    inline_stylesheets: Mutex<HashMap<InlineSourceCacheKey, Compilation<Stylesheet>>>,
    scripts: Mutex<HashMap<SourceCacheKey, Compilation<ScriptModule>>>,
    inline_scripts: Mutex<HashMap<InlineSourceCacheKey, Compilation<ScriptModule>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SourceCacheKey {
    kind: SourceKind,
    name: Option<String>,
    source: ArcStr,
}

impl SourceCacheKey {
    fn new(kind: SourceKind, name: Option<&str>, source: ArcStr) -> Self {
        Self {
            kind,
            name: name.map(str::to_owned),
            source,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct InlineSourceCacheKey {
    kind: SourceKind,
    source: ArcStr,
    offset: usize,
}

impl InlineSourceCacheKey {
    fn new(kind: SourceKind, source: ArcStr, offset: usize) -> Self {
        Self {
            kind,
            source,
            offset,
        }
    }
}

fn lock_cache<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompilerOptions {
    pub parallelism: CompilerParallelism,
    pub resources: CompilerResourceOptions,
    pub bundle: BundleConfig,
    pub source_frontend: SourceFrontendKind,
    pub source_policy: SourcePolicy,
}

impl CompilerOptions {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_parallelism(mut self, parallelism: CompilerParallelism) -> Self {
        self.parallelism = parallelism;
        self
    }

    #[must_use]
    pub fn with_external_scripts(
        mut self,
        external_scripts: CompilerExternalScriptOptions,
    ) -> Self {
        self.resources = external_scripts;
        self
    }

    #[must_use]
    pub fn with_external_script_resolution(mut self, resolve: bool) -> Self {
        self.resources.resolve_remote = resolve;
        self
    }

    #[must_use]
    pub fn with_resources(mut self, resources: CompilerResourceOptions) -> Self {
        self.resources = resources;
        self
    }

    #[must_use]
    pub fn with_remote_resource_resolution(mut self, resolve: bool) -> Self {
        self.resources.resolve_remote = resolve;
        self
    }

    #[must_use]
    pub fn with_bundle(mut self, bundle: BundleConfig) -> Self {
        self.bundle = bundle;
        self
    }

    #[must_use]
    pub fn with_source_frontend(mut self, source_frontend: SourceFrontendKind) -> Self {
        self.source_frontend = source_frontend;
        self
    }

    #[must_use]
    pub fn with_source_policy(mut self, source_policy: SourcePolicy) -> Self {
        self.source_policy = source_policy;
        self
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExtensionAttributes {
    #[default]
    Allow,
    Forbid,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Scripts {
    #[default]
    Allow,
    Forbid,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RemoteResources {
    #[default]
    Allow,
    Forbid,
}

/// Source features accepted before target semantics are lowered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourcePolicy {
    pub extension_attributes: ExtensionAttributes,
    pub scripts: Scripts,
    pub remote_resources: RemoteResources,
}

impl SourcePolicy {
    /// Accept ordinary local HTML and CSS while rejecting executable or dialect-specific input.
    #[must_use]
    pub const fn pure_html() -> Self {
        Self {
            extension_attributes: ExtensionAttributes::Forbid,
            scripts: Scripts::Forbid,
            remote_resources: RemoteResources::Forbid,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SourceFrontendKind {
    #[default]
    Html,
    Dc,
    Vue,
    Jsx,
}

pub const DEFAULT_RESOURCE_TIMEOUT: Duration = Duration::from_secs(10);
pub const DEFAULT_RESOURCE_MAX_BYTES: usize = 2 * 1024 * 1024;
pub const DEFAULT_EXTERNAL_SCRIPT_TIMEOUT: Duration = DEFAULT_RESOURCE_TIMEOUT;
pub const DEFAULT_EXTERNAL_SCRIPT_MAX_BYTES: usize = DEFAULT_RESOURCE_MAX_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompilerResourceOptions {
    pub resolve_remote: bool,
    pub resolve_files: bool,
    pub timeout: Duration,
    pub max_bytes: usize,
}

pub type CompilerExternalScriptOptions = CompilerResourceOptions;

impl CompilerResourceOptions {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_resolution(mut self, resolve: bool) -> Self {
        self.resolve_remote = resolve;
        self
    }

    #[must_use]
    pub fn with_remote_resolution(mut self, resolve: bool) -> Self {
        self.resolve_remote = resolve;
        self
    }

    #[must_use]
    pub fn with_file_resolution(mut self, resolve: bool) -> Self {
        self.resolve_files = resolve;
        self
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[must_use]
    pub fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes;
        self
    }
}

impl Default for CompilerResourceOptions {
    fn default() -> Self {
        Self {
            resolve_remote: true,
            resolve_files: true,
            timeout: DEFAULT_RESOURCE_TIMEOUT,
            max_bytes: DEFAULT_RESOURCE_MAX_BYTES,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CompilerParallelism {
    #[default]
    Auto,
    Sequential,
    Rayon {
        threads: Option<NonZeroUsize>,
    },
}

impl CompilerParallelism {
    #[must_use]
    pub const fn auto() -> Self {
        Self::Auto
    }

    #[must_use]
    pub const fn sequential() -> Self {
        Self::Sequential
    }

    #[must_use]
    pub const fn rayon() -> Self {
        Self::Rayon { threads: None }
    }

    #[must_use]
    pub const fn rayon_threads(threads: NonZeroUsize) -> Self {
        Self::Rayon {
            threads: Some(threads),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CompilerBuildError {
    #[error("failed to build compiler worker pool: {0}")]
    WorkerPool(#[from] JobRunnerBuildError),
}

fn job_runner_from_parallelism(
    parallelism: CompilerParallelism,
) -> Result<JobRunner, CompilerBuildError> {
    match parallelism {
        CompilerParallelism::Auto | CompilerParallelism::Rayon { threads: None } => {
            Ok(JobRunner::automatic())
        }
        CompilerParallelism::Sequential => Ok(JobRunner::inline()),
        CompilerParallelism::Rayon {
            threads: Some(threads),
        } => Ok(JobRunner::with_workers(threads.get())?),
    }
}

fn resource_resolver_from_options(options: &CompilerOptions) -> Arc<dyn ResourceResolver> {
    let resolve_remote = options.resources.resolve_remote
        && options.source_policy.remote_resources == RemoteResources::Allow;
    if !resolve_remote && !options.resources.resolve_files {
        return Arc::new(NoopResourceResolver::new());
    }

    let http = resolve_remote.then(|| {
        HttpResourceResolver::new(HttpResourceResolverOptions::new(
            options.resources.timeout,
            options.resources.max_bytes,
        ))
    });
    let file_system = options.resources.resolve_files.then(|| {
        FileSystemResourceResolver::new(FileSystemResourceResolverOptions::new(
            options.resources.max_bytes,
        ))
    });

    Arc::new(DefaultResourceResolver::new(http, file_system))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledFragment {
    pub plan: RenderPlan,
    pub sources: SourceMap,
    pub bundle: BundlePlan,
}

#[must_use]
pub(crate) fn compile_fragment_with_sources(
    source: ArcStr,
    assets: &CompileAssets,
) -> Compilation<CompiledFragment> {
    let options = CompilerOptions::default();
    let resolver = resource_resolver_from_options(&options);
    compile_fragment_with_jobs(
        None,
        source,
        assets,
        &JobContext::default(),
        resolver.as_ref(),
        &Frontend::default(),
        &StyleProviderPipeline::new(),
    )
}

fn compile_fragment_with_jobs(
    name: Option<String>,
    source: ArcStr,
    assets: &CompileAssets,
    jobs: &JobContext,
    resolver: &dyn ResourceResolver,
    frontend: &Frontend,
    style_providers: &StyleProviderPipeline,
) -> Compilation<CompiledFragment> {
    compile_fragment_with_jobs_and_cache(
        name,
        source,
        assets,
        jobs,
        resolver,
        frontend,
        SourceFrontendKind::Html,
        SourcePolicy::default(),
        &BundleConfig::default(),
        None,
        style_providers,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "central compile entry threads source, frontend, jobs, resolver, bundle, and cache context"
)]
fn compile_fragment_with_jobs_and_cache(
    name: Option<String>,
    source: ArcStr,
    assets: &CompileAssets,
    jobs: &JobContext,
    resolver: &dyn ResourceResolver,
    frontend: &Frontend,
    source_frontend: SourceFrontendKind,
    source_policy: SourcePolicy,
    bundle: &BundleConfig,
    cache: Option<&CompilerCache>,
    style_providers: &StyleProviderPipeline,
) -> Compilation<CompiledFragment> {
    let mut sources = SourceMap::new();
    if source_frontend == SourceFrontendKind::Jsx {
        let jsx_id = sources.add_file(SourceKind::JavaScript, name, source);
        let mut diagnostics = Diagnostics::new();
        if !style_providers.is_empty() {
            diagnostics.push(Diagnostic::error(
                "style providers require an HTML-based source frontend; JSX provider expansion is not supported",
                None,
            ));
        }
        if jobs.cancel().is_cancelled() {
            return cancelled_compilation(sources, diagnostics);
        }
        let source_file = sources
            .file(jsx_id)
            .expect("registered JSX source should exist");
        let lowered = jsx::lower_jsx_module(source_file.text(), source_file.name(), jsx_id);
        diagnostics.extend(lowered.diagnostics);
        let plan = lowered.value;
        return Compilation::new(
            CompiledFragment {
                bundle: BundlePlan::from_render_plan_with_config(&plan, &sources, bundle),
                plan,
                sources,
            },
            diagnostics,
        );
    }

    let html_id = sources.add_file(SourceKind::Html, name, source);

    let mut stylesheet_ids = assets
        .stylesheets
        .iter()
        .map(|stylesheet| {
            sources.add_file(
                SourceKind::Css,
                stylesheet.name.clone(),
                stylesheet.contents.clone(),
            )
        })
        .collect::<Vec<_>>();

    let mut script_ids = assets
        .scripts
        .iter()
        .map(|script| {
            sources.add_file(
                SourceKind::JavaScript,
                script.name.clone(),
                script.contents.clone(),
            )
        })
        .collect::<Vec<_>>();

    let mut diagnostics = Diagnostics::new();
    if jobs.cancel().is_cancelled() {
        return cancelled_compilation(sources, diagnostics);
    }

    let parsed = {
        let html = sources
            .file(html_id)
            .expect("registered HTML source should exist");
        cache.map_or_else(
            || parse_html_with_source(html.text(), html_id),
            |cache| cache.parse_html(html.buffer().clone(), html_id),
        )
    };
    diagnostics.extend(parsed.diagnostics);
    if jobs.cancel().is_cancelled() {
        return cancelled_compilation(sources, diagnostics);
    }

    diagnostics.extend(validate_source_policy(&parsed.value, assets, source_policy));
    if diagnostics.has_errors() {
        let plan = RenderPlan::new(Vec::new());
        let bundle = BundlePlan::from_render_plan_with_config(&plan, &sources, bundle);
        return Compilation::new(
            CompiledFragment {
                plan,
                sources,
                bundle,
            },
            diagnostics,
        );
    }

    if resolve_linked_stylesheet_sources(
        &parsed.value,
        &mut stylesheet_ids,
        html_id,
        &mut ResourceResolutionContext::new(&mut sources, resolver, jobs, &mut diagnostics),
    ) {
        return cancelled_compilation(sources, diagnostics);
    }

    if resolve_external_script_sources(
        &parsed.value,
        &mut script_ids,
        html_id,
        &mut ResourceResolutionContext::new(&mut sources, resolver, jobs, &mut diagnostics),
    ) {
        return cancelled_compilation(sources, diagnostics);
    }

    let classes = ClassIndex::collect(&parsed.value, html_id);
    let expanded_styles = style_providers.expand(StyleProviderInput {
        sources: &sources,
        document: &parsed.value,
        classes: &classes,
        resources: resolver,
    });
    diagnostics.extend(expanded_styles.diagnostics);

    let mut annotations = expanded_styles.value.annotations;
    for annotation in &mut annotations {
        if annotation
            .span
            .is_some_and(|span| sources.source_text(span).is_none())
        {
            diagnostics.push(Diagnostic::warning(
                "style provider returned an annotation with an invalid source span",
                None,
            ));
            annotation.span = None;
        }
    }
    for stylesheet in expanded_styles.value.stylesheets {
        if sources
            .files()
            .iter()
            .any(|source| source.name() == Some(stylesheet.name.as_str()))
        {
            diagnostics.push(Diagnostic::error(
                format!(
                    "generated stylesheet source `{}` conflicts with an existing source name",
                    stylesheet.name
                ),
                None,
            ));
            continue;
        }

        stylesheet_ids.push(sources.add_file(
            SourceKind::Css,
            Some(stylesheet.name),
            stylesheet.contents,
        ));
    }
    if jobs.cancel().is_cancelled() {
        return cancelled_compilation(sources, diagnostics);
    }

    let lowered = {
        let mut styles = StyleIndex::new();
        if parse_stylesheet_graph(
            &stylesheet_ids,
            ResourceResolutionContext::new(&mut sources, resolver, jobs, &mut diagnostics),
            cache,
            &mut annotations,
            &mut styles,
        ) {
            return cancelled_compilation(sources, diagnostics);
        }

        let inline_stylesheet_sources = inline_stylesheets(&parsed.value);
        let parsed_inline_stylesheets = jobs.runner().map_until_cancelled(
            &inline_stylesheet_sources,
            jobs.cancel(),
            |stylesheet| {
                let source_id = stylesheet.span.map(|span| span.source).unwrap_or(html_id);
                let offset = stylesheet.span.map_or(0, |span| span.start);
                cache.map_or_else(
                    || parse_stylesheet_with_offset(&stylesheet.source, source_id, offset),
                    |cache| {
                        cache.parse_inline_stylesheet(
                            ArcStr::from(stylesheet.source.as_str()),
                            source_id,
                            offset,
                        )
                    },
                )
            },
        );
        let was_cancelled = parsed_inline_stylesheets.was_cancelled();
        for parsed_stylesheet in parsed_inline_stylesheets.into_completed() {
            diagnostics.extend(parsed_stylesheet.diagnostics);
            let mut stylesheet = parsed_stylesheet.value;
            let imports = std::mem::take(&mut stylesheet.imports);
            let mut import_source_ids = Vec::new();
            if resolve_stylesheet_imports(
                imports,
                html_id,
                &mut import_source_ids,
                &mut ResourceResolutionContext::new(&mut sources, resolver, jobs, &mut diagnostics),
            ) || parse_stylesheet_graph(
                &import_source_ids,
                ResourceResolutionContext::new(&mut sources, resolver, jobs, &mut diagnostics),
                cache,
                &mut annotations,
                &mut styles,
            ) {
                return cancelled_compilation(sources, diagnostics);
            }
            annotations.append(&mut stylesheet.annotations);
            styles.extend_stylesheet(stylesheet);
        }
        if was_cancelled {
            return cancelled_compilation(sources, diagnostics);
        }

        let mut actions = ActionIndex::new();
        let mut script_styles = ScriptStyleIndex::new();
        let inline_script_sources = crate::script::inline_scripts(&parsed.value);
        let parsed_inline_scripts =
            jobs.runner()
                .map_until_cancelled(&inline_script_sources, jobs.cancel(), |script| {
                    cache.map_or_else(
                        || crate::script::parse_inline_script(script),
                        |cache| {
                            let source_id = script.span.map(|span| span.source).unwrap_or(html_id);
                            let offset = script.span.map_or(0, |span| span.start);
                            cache.parse_inline_script(
                                ArcStr::from(script.source.as_str()),
                                source_id,
                                offset,
                            )
                        },
                    )
                });
        let was_cancelled = parsed_inline_scripts.was_cancelled();
        let mut inline_import_ids = VecDeque::new();
        for parsed_script in parsed_inline_scripts.into_completed() {
            let imports = merge_script(
                parsed_script,
                &mut diagnostics,
                &mut annotations,
                &mut actions,
                &mut script_styles,
            );
            if resolve_script_import_sources(
                imports,
                html_id,
                &mut inline_import_ids,
                &mut ResourceResolutionContext::new(&mut sources, resolver, jobs, &mut diagnostics),
            ) {
                return cancelled_compilation(sources, diagnostics);
            }
        }
        script_ids.extend(inline_import_ids);
        if was_cancelled {
            return cancelled_compilation(sources, diagnostics);
        }

        if parse_script_graph(
            &script_ids,
            ResourceResolutionContext::new(&mut sources, resolver, jobs, &mut diagnostics),
            cache,
            &mut annotations,
            &mut actions,
            &mut script_styles,
        ) {
            return cancelled_compilation(sources, diagnostics);
        }

        let context = LowerContext {
            styles: (!styles.is_empty()).then_some(&styles),
            actions: (!actions.is_empty()).then_some(&actions),
            script_styles: (!script_styles.is_empty()).then_some(&script_styles),
            sources: Some(&sources),
            frontend: Some(frontend),
        };
        let lowered =
            lower_document_with_context(&parsed.value, &LowerOptions::default(), &context);
        let mut plan = lowered.value;
        apply_root_dc_style_bindings(&mut plan);
        plan.annotations.extend(annotations);
        sort_annotations(&mut plan.annotations);
        diagnostics.extend(lowered.diagnostics);
        if jobs.cancel().is_cancelled() {
            return cancelled_compilation(sources, diagnostics);
        }
        plan
    };

    Compilation::new(
        CompiledFragment {
            bundle: BundlePlan::from_render_plan_with_config(&lowered, &sources, bundle),
            plan: lowered,
            sources,
        },
        diagnostics,
    )
}

fn validate_source_policy(
    document: &HtmlDocument,
    assets: &CompileAssets,
    policy: SourcePolicy,
) -> Diagnostics {
    let mut diagnostics = Diagnostics::new();
    validate_policy_nodes(&document.nodes, policy, &mut diagnostics);

    if policy.scripts == Scripts::Forbid {
        diagnostics.extend(
            assets
                .scripts
                .iter()
                .map(|script| {
                    Diagnostic::error(
                        format!(
                            "pure HTML policy forbids script asset `{}`",
                            script.name.as_deref().unwrap_or("<unnamed>")
                        ),
                        None,
                    )
                })
                .collect(),
        );
    }
    if policy.remote_resources == RemoteResources::Forbid {
        diagnostics.extend(
            assets
                .stylesheets
                .iter()
                .filter(|stylesheet| contains_remote_reference(&stylesheet.contents))
                .map(|stylesheet| {
                    Diagnostic::error(
                        format!(
                            "pure HTML policy forbids remote reference in stylesheet `{}`",
                            stylesheet.name.as_deref().unwrap_or("<unnamed>")
                        ),
                        None,
                    )
                })
                .collect(),
        );
    }

    diagnostics
}

fn validate_policy_nodes(nodes: &[HtmlNode], policy: SourcePolicy, diagnostics: &mut Diagnostics) {
    for node in nodes {
        let HtmlNode::Element(element) = node else {
            continue;
        };
        let tag = element.name.local();

        if policy.scripts == Scripts::Forbid
            && matches!(tag, "script" | "iframe" | "object" | "embed")
        {
            diagnostics.push(Diagnostic::error(
                format!("pure HTML policy forbids active `<{tag}>` elements"),
                element.span,
            ));
        }

        for attribute in &element.attributes {
            let name = attribute.name.local();
            if policy.extension_attributes == ExtensionAttributes::Forbid
                && name.starts_with("data-htmlswap-")
            {
                diagnostics.push(Diagnostic::error(
                    format!("pure HTML policy forbids extension attribute `{name}`"),
                    attribute.span,
                ));
            }
            if policy.scripts == Scripts::Forbid
                && ((name.starts_with("on") && name.len() > 2)
                    || attribute
                        .value
                        .trim_start()
                        .to_ascii_lowercase()
                        .starts_with("javascript:"))
            {
                diagnostics.push(Diagnostic::error(
                    format!("pure HTML policy forbids executable attribute `{name}`"),
                    attribute.span,
                ));
            }
            if policy.remote_resources == RemoteResources::Forbid
                && contains_remote_reference(&attribute.value)
            {
                diagnostics.push(Diagnostic::error(
                    format!("pure HTML policy forbids remote reference in `{name}`"),
                    attribute.span,
                ));
            }
        }

        if policy.remote_resources == RemoteResources::Forbid
            && tag == "style"
            && element.children.iter().any(|child| {
                matches!(child, HtmlNode::Text(text) if contains_remote_reference(&text.value))
            })
        {
            diagnostics.push(Diagnostic::error(
                "pure HTML policy forbids remote reference in `<style>`",
                element.span,
            ));
        }

        validate_policy_nodes(&element.children, policy, diagnostics);
    }
}

fn contains_remote_reference(value: &str) -> bool {
    value
        .split(|character: char| {
            character.is_whitespace() || matches!(character, '(' | ')' | '\'' | '"' | ',' | ';')
        })
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .any(|token| {
            let token = token.to_ascii_lowercase();
            token.starts_with("http:") || token.starts_with("https:") || token.starts_with("//")
        })
}

fn parse_html_with_source(source: &str, source_id: SourceId) -> Compilation<HtmlDocument> {
    if looks_like_html_document(source) {
        parse_document_with_source(source, source_id)
    } else {
        parse_fragment_with_source(source, source_id)
    }
}

fn looks_like_html_document(source: &str) -> bool {
    let source = source.trim_start_matches(|ch: char| ch.is_whitespace() || ch == '\u{feff}');
    let prefix = source
        .chars()
        .take(256)
        .collect::<String>()
        .to_ascii_lowercase();
    prefix.starts_with("<!doctype")
        || prefix.starts_with("<html")
        || prefix.contains("<head")
        || prefix.contains("<body")
}

fn cancelled_compilation(
    sources: SourceMap,
    mut diagnostics: Diagnostics,
) -> Compilation<CompiledFragment> {
    diagnostics.push(Diagnostic::error("compilation cancelled", None));
    Compilation::new(
        CompiledFragment {
            plan: RenderPlan::new(Vec::new()),
            sources,
            bundle: BundlePlan::default(),
        },
        diagnostics,
    )
}

#[derive(Debug, Clone)]
struct StylesheetSource {
    source_id: SourceId,
    source: ArcStr,
    name: Option<String>,
}

fn parse_stylesheet_graph(
    root_stylesheet_ids: &[SourceId],
    resources: ResourceResolutionContext<'_>,
    cache: Option<&CompilerCache>,
    annotations: &mut Vec<RenderAnnotation>,
    styles: &mut StyleIndex,
) -> bool {
    let mut parser = StylesheetGraphParser {
        resources,
        cache,
        annotations,
        styles,
        parsed_source_ids: HashSet::new(),
    };
    parser.parse_roots(root_stylesheet_ids)
}

struct StylesheetGraphParser<'a> {
    resources: ResourceResolutionContext<'a>,
    cache: Option<&'a CompilerCache>,
    annotations: &'a mut Vec<RenderAnnotation>,
    styles: &'a mut StyleIndex,
    parsed_source_ids: HashSet<SourceId>,
}

impl StylesheetGraphParser<'_> {
    fn parse_roots(&mut self, root_stylesheet_ids: &[SourceId]) -> bool {
        for &source_id in root_stylesheet_ids {
            if self.parse_tree(source_id) {
                return true;
            }
        }

        false
    }

    fn parse_tree(&mut self, source_id: SourceId) -> bool {
        if !self.parsed_source_ids.insert(source_id) {
            return false;
        }
        if self.resources.jobs.cancel().is_cancelled() {
            return true;
        }

        let stylesheet = {
            let source_file = registered_source(self.resources.sources, source_id, "stylesheet");
            StylesheetSource {
                source_id,
                source: source_file.buffer().clone(),
                name: source_file.name().map(str::to_owned),
            }
        };
        let parsed = self.cache.map_or_else(
            || parse_stylesheet_with_source(stylesheet.source.as_str(), stylesheet.source_id),
            |cache| {
                cache.parse_stylesheet(
                    stylesheet.source.clone(),
                    stylesheet.name.as_deref(),
                    stylesheet.source_id,
                )
            },
        );
        self.resources.diagnostics.extend(parsed.diagnostics);
        let mut stylesheet = parsed.value;
        let imports = std::mem::take(&mut stylesheet.imports);
        let mut import_source_ids = Vec::new();
        if self.resolve_imports(imports, source_id, &mut import_source_ids) {
            return true;
        }
        for import_source_id in import_source_ids {
            if self.parse_tree(import_source_id) {
                return true;
            }
        }

        self.annotations.append(&mut stylesheet.annotations);
        self.styles.extend_stylesheet(stylesheet);
        false
    }

    fn resolve_imports(
        &mut self,
        imports: Vec<StylesheetImport>,
        referrer_id: SourceId,
        source_ids: &mut Vec<SourceId>,
    ) -> bool {
        resolve_stylesheet_imports(imports, referrer_id, source_ids, &mut self.resources)
    }
}

fn resolve_stylesheet_imports(
    imports: Vec<StylesheetImport>,
    referrer_id: SourceId,
    source_ids: &mut Vec<SourceId>,
    resources: &mut ResourceResolutionContext<'_>,
) -> bool {
    let external_resources = imports
        .into_iter()
        .map(|import| ExternalResource {
            kind: ResourceKind::Stylesheet,
            specifier: import.specifier,
            span: import.span,
        })
        .collect::<Vec<_>>();

    resources.resolve_sources(external_resources, SourceKind::Css, referrer_id, source_ids)
}

#[derive(Debug, Clone)]
struct ScriptSource {
    source_id: SourceId,
    source: ArcStr,
    name: Option<String>,
}

#[derive(Debug, Clone)]
struct ParsedScriptSource {
    source_id: SourceId,
    parsed: Compilation<ScriptModule>,
}

#[derive(Debug, Clone)]
struct ExternalResource {
    kind: ResourceKind,
    specifier: String,
    span: Option<crate::source::Span>,
}

struct ResourceResolutionContext<'a> {
    sources: &'a mut SourceMap,
    resolver: &'a dyn ResourceResolver,
    jobs: &'a JobContext,
    diagnostics: &'a mut Diagnostics,
}

impl<'a> ResourceResolutionContext<'a> {
    fn new(
        sources: &'a mut SourceMap,
        resolver: &'a dyn ResourceResolver,
        jobs: &'a JobContext,
        diagnostics: &'a mut Diagnostics,
    ) -> Self {
        Self {
            sources,
            resolver,
            jobs,
            diagnostics,
        }
    }

    fn resolve_sources(
        &mut self,
        resources: Vec<ExternalResource>,
        source_kind: SourceKind,
        referrer_id: SourceId,
        source_ids: &mut Vec<SourceId>,
    ) -> bool {
        let referrer = resource_referrer(self.sources, referrer_id);
        let mut seen = self
            .sources
            .files()
            .iter()
            .filter(|source| source.kind() == source_kind)
            .filter_map(|source| source.name().map(str::to_owned))
            .collect::<HashSet<_>>();
        let requests = resources
            .into_iter()
            .filter(|resource| seen.insert(resource.specifier.clone()))
            .map(|resource| {
                ResourceRequest::new(resource.kind, resource.specifier)
                    .with_referrer(referrer.clone())
                    .with_span(resource.span)
            })
            .filter(|request| self.resolver.can_resolve(request))
            .collect::<Vec<_>>();
        if requests.is_empty() {
            return false;
        }

        let resolved =
            self.jobs
                .runner()
                .map_until_cancelled(&requests, self.jobs.cancel(), |request| {
                    self.resolver.resolve(request)
                });
        let was_cancelled = resolved.was_cancelled();
        for resolved in resolved.into_completed() {
            self.diagnostics.extend(resolved.diagnostics);
            if let Some(source) = resolved.value
                && let Some(source_id) = register_resource_source(source, self.sources)
            {
                source_ids.push(source_id);
            }
        }

        was_cancelled
    }
}

fn parse_script_graph(
    root_script_ids: &[SourceId],
    resources: ResourceResolutionContext<'_>,
    cache: Option<&CompilerCache>,
    annotations: &mut Vec<RenderAnnotation>,
    actions: &mut ActionIndex,
    script_styles: &mut ScriptStyleIndex,
) -> bool {
    let mut parser = ScriptGraphParser {
        resources,
        cache,
        annotations,
        actions,
        script_styles,
        parsed_source_ids: HashSet::new(),
    };
    parser.parse_roots(root_script_ids)
}

struct ScriptGraphParser<'a> {
    resources: ResourceResolutionContext<'a>,
    cache: Option<&'a CompilerCache>,
    annotations: &'a mut Vec<RenderAnnotation>,
    actions: &'a mut ActionIndex,
    script_styles: &'a mut ScriptStyleIndex,
    parsed_source_ids: HashSet<SourceId>,
}

impl ScriptGraphParser<'_> {
    fn parse_roots(&mut self, root_script_ids: &[SourceId]) -> bool {
        let mut pending = root_script_ids.iter().copied().collect::<VecDeque<_>>();

        while !pending.is_empty() {
            let pending_ids = pending
                .drain(..)
                .filter(|source_id| self.parsed_source_ids.insert(*source_id))
                .collect::<Vec<_>>();
            if pending_ids.is_empty() {
                continue;
            }

            let script_sources = pending_ids
                .iter()
                .map(|&source_id| {
                    let source_file =
                        registered_source(self.resources.sources, source_id, "script");
                    ScriptSource {
                        source_id,
                        source: source_file.buffer().clone(),
                        name: source_file.name().map(str::to_owned),
                    }
                })
                .collect::<Vec<_>>();
            let parsed_scripts = self.resources.jobs.runner().map_until_cancelled(
                &script_sources,
                self.resources.jobs.cancel(),
                |script| {
                    let parsed = self.cache.map_or_else(
                        || {
                            parse_script_with_source(
                                script.source.as_str(),
                                script.name.as_deref(),
                                script.source_id,
                            )
                        },
                        |cache| {
                            cache.parse_script(
                                script.source.clone(),
                                script.name.as_deref(),
                                script.source_id,
                            )
                        },
                    );
                    ParsedScriptSource {
                        source_id: script.source_id,
                        parsed,
                    }
                },
            );
            let was_cancelled = parsed_scripts.was_cancelled();
            for parsed_script in parsed_scripts.into_completed() {
                let imports = merge_script(
                    parsed_script.parsed,
                    self.resources.diagnostics,
                    self.annotations,
                    self.actions,
                    self.script_styles,
                );
                if resolve_script_import_sources(
                    imports,
                    parsed_script.source_id,
                    &mut pending,
                    &mut self.resources,
                ) {
                    return true;
                }
            }
            if was_cancelled {
                return true;
            }
        }

        false
    }
}

fn resolve_script_import_sources(
    imports: Vec<ScriptImport>,
    referrer_id: SourceId,
    pending: &mut VecDeque<SourceId>,
    resources: &mut ResourceResolutionContext<'_>,
) -> bool {
    let external_resources = imports
        .into_iter()
        .map(|import| ExternalResource {
            kind: ResourceKind::Script,
            specifier: import.specifier.to_string(),
            span: import.span,
        })
        .collect::<Vec<_>>();
    let mut source_ids = Vec::new();
    let was_cancelled = resources.resolve_sources(
        external_resources,
        SourceKind::JavaScript,
        referrer_id,
        &mut source_ids,
    );
    pending.extend(source_ids);
    was_cancelled
}

fn resolve_linked_stylesheet_sources(
    document: &HtmlDocument,
    stylesheet_ids: &mut Vec<SourceId>,
    html_id: SourceId,
    resources: &mut ResourceResolutionContext<'_>,
) -> bool {
    let external_resources = crate::css::linked_stylesheets(document)
        .into_iter()
        .map(|stylesheet| ExternalResource {
            kind: ResourceKind::Stylesheet,
            specifier: stylesheet.href,
            span: stylesheet.span,
        })
        .collect::<Vec<_>>();

    resources.resolve_sources(external_resources, SourceKind::Css, html_id, stylesheet_ids)
}

fn resolve_external_script_sources(
    document: &HtmlDocument,
    script_ids: &mut Vec<SourceId>,
    html_id: SourceId,
    resources: &mut ResourceResolutionContext<'_>,
) -> bool {
    let external_resources = crate::script::external_scripts(document)
        .into_iter()
        .map(|script| ExternalResource {
            kind: ResourceKind::Script,
            specifier: script.src,
            span: script.span,
        })
        .collect::<Vec<_>>();

    resources.resolve_sources(
        external_resources,
        SourceKind::JavaScript,
        html_id,
        script_ids,
    )
}

fn resource_referrer(sources: &SourceMap, source_id: SourceId) -> Option<ResourceReferrer> {
    let source = sources.file(source_id)?;
    Some(ResourceReferrer::new(
        source_id,
        source.name().map(ArcStr::from),
    ))
}

fn register_resource_source(source: ResourceSource, sources: &mut SourceMap) -> Option<SourceId> {
    let source_kind = source.kind.source_kind();
    if source
        .name
        .as_deref()
        .and_then(|name| source_id_by_name(sources, source_kind, name))
        .is_some()
    {
        return None;
    }

    Some(sources.add_file(source_kind, source.name, source.contents))
}

fn source_id_by_name(sources: &SourceMap, kind: SourceKind, name: &str) -> Option<SourceId> {
    sources
        .files()
        .iter()
        .find(|source| source.kind() == kind && source.name() == Some(name))
        .map(SourceFile::id)
}

fn registered_source<'a>(
    sources: &'a SourceMap,
    source_id: SourceId,
    label: &str,
) -> &'a SourceFile {
    sources
        .file(source_id)
        .unwrap_or_else(|| panic!("registered {label} source should exist"))
}

fn merge_script(
    parsed: Compilation<ScriptModule>,
    diagnostics: &mut Diagnostics,
    annotations: &mut Vec<RenderAnnotation>,
    actions: &mut ActionIndex,
    script_styles: &mut ScriptStyleIndex,
) -> Vec<ScriptImport> {
    diagnostics.extend(parsed.diagnostics);
    let mut script = parsed.value;
    annotations.append(&mut script.annotations);
    let imports = std::mem::take(&mut script.imports);
    script_styles.extend_module(&mut script);
    actions.extend_module(script);
    imports
}

fn sort_annotations(annotations: &mut [RenderAnnotation]) {
    annotations.sort_by_key(|annotation| {
        annotation
            .span
            .map(|span| (span.source.index(), span.start, span.end))
            .unwrap_or((usize::MAX, usize::MAX, usize::MAX))
    });
}

fn rebase_html_compilation(
    parsed: Compilation<HtmlDocument>,
    source_id: SourceId,
) -> Compilation<HtmlDocument> {
    Compilation::new(
        rebase_html_document(parsed.value, source_id),
        rebase_diagnostics(parsed.diagnostics, source_id),
    )
}

fn rebase_stylesheet_compilation(
    parsed: Compilation<Stylesheet>,
    source_id: SourceId,
) -> Compilation<Stylesheet> {
    Compilation::new(
        rebase_stylesheet(parsed.value, source_id),
        rebase_diagnostics(parsed.diagnostics, source_id),
    )
}

fn rebase_script_compilation(
    parsed: Compilation<ScriptModule>,
    source_id: SourceId,
) -> Compilation<ScriptModule> {
    Compilation::new(
        rebase_script(parsed.value, source_id),
        rebase_diagnostics(parsed.diagnostics, source_id),
    )
}

fn rebase_diagnostics(diagnostics: Diagnostics, source_id: SourceId) -> Diagnostics {
    diagnostics
        .into_iter()
        .map(|mut diagnostic| {
            diagnostic.span = diagnostic.span.map(|span| span.with_source(source_id));
            diagnostic
        })
        .collect()
}

fn rebase_html_document(mut document: HtmlDocument, source_id: SourceId) -> HtmlDocument {
    rebase_html_nodes(&mut document.nodes, source_id);
    document
}

fn rebase_html_nodes(nodes: &mut [HtmlNode], source_id: SourceId) {
    for node in nodes {
        match node {
            HtmlNode::Element(element) => rebase_html_element(element, source_id),
            HtmlNode::Text(text) => {
                text.span = text.span.map(|span| span.with_source(source_id));
            }
            HtmlNode::Comment(comment) => {
                comment.span = comment.span.map(|span| span.with_source(source_id));
            }
        }
    }
}

fn rebase_html_element(element: &mut HtmlElement, source_id: SourceId) {
    element.span = element.span.map(|span| span.with_source(source_id));
    rebase_html_attributes(&mut element.attributes, source_id);
    rebase_html_nodes(&mut element.children, source_id);
}

fn rebase_html_attributes(attributes: &mut [HtmlAttribute], source_id: SourceId) {
    for attribute in attributes {
        attribute.span = attribute.span.map(|span| span.with_source(source_id));
    }
}

fn rebase_stylesheet(mut stylesheet: Stylesheet, source_id: SourceId) -> Stylesheet {
    rebase_annotations(&mut stylesheet.annotations, source_id);
    for rule in &mut stylesheet.rules {
        rule.span = rule.span.map(|span| span.with_source(source_id));
        for declaration in &mut rule.declarations {
            declaration.span = declaration.span.map(|span| span.with_source(source_id));
        }
    }
    for import in &mut stylesheet.imports {
        import.span = import.span.map(|span| span.with_source(source_id));
    }
    let rebase = |span: &mut Option<crate::source::Span>| {
        *span = span.map(|span| span.with_source(source_id))
    };
    for keyframes in &mut stylesheet.motion.keyframes {
        rebase(&mut keyframes.span);
        for frame in &mut keyframes.frames {
            for declaration in &mut frame.declarations {
                rebase(&mut declaration.span);
            }
        }
    }
    let view_transition = &mut stylesheet.motion.view_transition;
    rebase(&mut view_transition.span);
    for rule in &mut view_transition.rules {
        rebase(&mut rule.span);
        for declaration in &mut rule.declarations {
            rebase(&mut declaration.span);
        }
    }
    stylesheet
}

fn rebase_script(mut script: ScriptModule, source_id: SourceId) -> ScriptModule {
    rebase_annotations(&mut script.annotations, source_id);
    for action in &mut script.actions {
        action.span = action.span.map(|span| span.with_source(source_id));
        for invocation in &mut action.handler.invocations {
            invocation.span = invocation.span.map(|span| span.with_source(source_id));
            invocation.action_span = invocation
                .action_span
                .map(|span| span.with_source(source_id));
        }
        for effect in &mut action.handler.effects {
            match effect {
                crate::plan::RenderActionHandlerEffect::PreventDefault { span }
                | crate::plan::RenderActionHandlerEffect::StopPropagation { span } => {
                    if let Some(span) = span {
                        *span = span.with_source(source_id);
                    }
                }
            }
        }
    }
    for import in &mut script.imports {
        import.span = import.span.map(|span| span.with_source(source_id));
    }
    for style_object in &mut script.style_objects {
        style_object.span = style_object.span.map(|span| span.with_source(source_id));
        for declaration in &mut style_object.declarations {
            declaration.span = declaration.span.map(|span| span.with_source(source_id));
        }
    }
    script
}

fn rebase_annotations(annotations: &mut [RenderAnnotation], source_id: SourceId) {
    for annotation in annotations {
        annotation.span = annotation.span.map(|span| span.with_source(source_id));
    }
}
