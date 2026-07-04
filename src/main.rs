use std::collections::{BTreeSet, VecDeque};
use std::io::{self, Read};
use std::num::NonZeroUsize;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use heck::ToUpperCamelCase;
use htmlswap::{
    Adapter, AdapterArtifact, AdapterContext, AdapterError, ArtifactBuilder, CompileAssets,
    CompiledFragment, Compiler, CompilerBuildError, CompilerCache, CompilerCacheFileSet,
    CompilerOptions, CompilerParallelism, CompilerResourceOptions, DEFAULT_RESOURCE_MAX_BYTES,
    DcComponentFragment, Diagnostics, EmitContext, EmitError, Emitter, Frontend, GeneratedFile,
    GpuiAdapter, GpuiAdapterOptions, GpuiComponentsAdapter, GpuiComponentsAdapterOptions, Importer,
    LayerId, LayoutDebugOptions, RenderNode, RenderPlan, RoundTripComparison, RoundTripOptions,
    RouteConfig, RouteTarget, RustFormatOptions, SourceAsset, SourceFrontendKind, SourceMap,
    StyleProperty, SvelteAdapter, SvelteAdapterOptions, TargetArtifact, TextEmitter, ThemeEmission,
    UiRole, compare_roundtrip_plans, inline_dc_component_imports,
    instrument_layout_snapshot_html_with_sources,
};
const DEFAULT_RESOURCE_TIMEOUT_MS: u64 = 10_000;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), CliError> {
    match Cli::parse().command {
        Command::Compile(command) => command.run(),
        Command::Import(command) => command.run(),
        Command::LayoutSnapshot(command) => command.run(),
        Command::RoundtripCheck(command) => command.run(),
    }
}

#[derive(Debug, Parser)]
#[command(name = "htmlswap", version, about = "Compile HTML into target UI code")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Compile(CompileCommand),
    Import(ImportCommand),
    LayoutSnapshot(LayoutSnapshotCommand),
    RoundtripCheck(RoundtripCheckCommand),
}

#[derive(Debug, Args)]
struct CompileCommand {
    /// HTML input path. Omit or pass '-' to read from stdin.
    input: Option<PathBuf>,

    /// External CSS file. May be passed multiple times.
    #[arg(long = "css", value_name = "PATH")]
    stylesheets: Vec<PathBuf>,

    /// External JavaScript or TypeScript file. May be passed multiple times.
    #[arg(long = "js", value_name = "PATH")]
    scripts: Vec<PathBuf>,

    /// Output path. Omit to write to stdout.
    #[arg(short, long, value_name = "PATH", conflicts_with = "out_dir")]
    output: Option<PathBuf>,

    /// Output directory for all generated adapter files.
    #[arg(long = "out-dir", value_name = "DIR")]
    out_dir: Option<PathBuf>,

    /// Target adapter.
    #[arg(long, value_enum, default_value = "gpui")]
    adapter: AdapterKind,

    /// Generated component name.
    #[arg(long, default_value = "HtmlswapView")]
    component_name: String,

    /// Source dialect to lower before adapter emission.
    #[arg(long = "source", value_enum, default_value = "html")]
    source: SourceFrontendKindArg,

    /// Route override in the form role:button=gpui, component:titlebar=gpui-components,
    /// region:editor.project=gpui-components, tag:input=gpui-components,
    /// style:display=gpui, or event:click=gpui.
    #[arg(long = "route", value_name = "TARGET=LAYER")]
    routes: Vec<RouteOverrideArg>,

    /// Worker policy: auto, sequential, or a positive worker count.
    #[arg(long, default_value = "auto")]
    jobs: JobsArg,

    /// Do not fetch http(s) resources discovered in HTML or imported modules.
    #[arg(long, alias = "no-resolve-external-scripts")]
    no_resolve_remote_resources: bool,

    /// Remote resource fetch timeout in milliseconds.
    #[arg(long, alias = "external-script-timeout-ms", default_value_t = DEFAULT_RESOURCE_TIMEOUT_MS)]
    resource_timeout_ms: u64,

    /// Maximum remote resource response size in bytes.
    #[arg(long, alias = "external-script-max-bytes", default_value_t = DEFAULT_RESOURCE_MAX_BYTES)]
    resource_max_bytes: usize,

    /// Reuse parsed HTML/CSS/JS by content across compiles.
    #[arg(long)]
    cache: bool,

    /// Recompile when any input asset changes.
    #[arg(long)]
    watch: bool,

    /// Watch polling interval in milliseconds.
    #[arg(long, default_value_t = 250)]
    poll_ms: u64,

    /// Disable generated source comments in adapters.
    #[arg(long)]
    no_source_comments: bool,

    /// Emit the original source HTML opening tag above each generated element when supported.
    #[arg(long)]
    debug_source_html: bool,

    /// Emit stable source-derived runtime IDs for layout comparison builds.
    #[arg(long)]
    debug_layout_ids: bool,

    /// Disable rustfmt-wrapper formatting for Rust adapters.
    #[arg(long)]
    no_format: bool,

    /// Disable generated dependency comment header.
    #[arg(long)]
    no_dependency_header: bool,

    /// Disable generated Rust imports for Rust adapters.
    #[arg(long)]
    no_imports: bool,
}

impl CompileCommand {
    fn run(&self) -> Result<(), CliError> {
        if self.watch && self.reads_stdin() {
            return Err(CliError::WatchRequiresFileInput);
        }

        let cache = (self.cache || self.watch).then(CompilerCache::new);
        let compiler = self.compiler(cache.clone())?;
        self.compile_once(&compiler)?;

        if !self.watch {
            return Ok(());
        }

        let mut files =
            CompilerCacheFileSet::read(self.watch_paths()).map_err(CliError::WatchInput)?;
        loop {
            thread::sleep(Duration::from_millis(self.poll_ms));
            if files.refresh_changed().map_err(CliError::WatchInput)? {
                self.compile_once(&compiler)?;
            }
        }
    }

    fn compiler(&self, cache: Option<CompilerCache>) -> Result<Compiler, CliError> {
        let options = CompilerOptions::new()
            .with_parallelism(self.jobs.parallelism())
            .with_resources(
                CompilerResourceOptions::new()
                    .with_remote_resolution(!self.no_resolve_remote_resources)
                    .with_timeout(Duration::from_millis(self.resource_timeout_ms))
                    .with_max_bytes(self.resource_max_bytes),
            )
            .with_source_frontend(self.source.source_frontend());
        let compiler = Compiler::try_with_options(options)?.with_frontend(self.source.frontend());
        Ok(match cache {
            Some(cache) => compiler.with_cache(cache),
            None => compiler,
        })
    }

    fn compile_once(&self, compiler: &Compiler) -> Result<(), CliError> {
        if self.source == SourceFrontendKindArg::Jsx
            && self.adapter == AdapterKind::Svelte
            && let Some(input_path) = self.input_path()
            && input_path.is_dir()
        {
            let artifact = self.render_jsx_svelte_project(compiler, input_path)?;
            self.write_artifact(&artifact)?;
            return Ok(());
        }

        let source = self.read_input()?;
        let assets = self.read_assets()?;
        let source_name = self.input_path().map(source_name_for_path);
        let compiled = compiler.compile_fragment_named(source_name, source, &assets);
        print_diagnostics(&compiled.diagnostics, &compiled.value.sources);

        let artifact = if self.source == SourceFrontendKindArg::Dc {
            if let Some(input_path) = self.input_path() {
                match self.adapter {
                    AdapterKind::Gpui | AdapterKind::GpuiComponents => {
                        self.render_gpui_dc_bundle(compiler, input_path, &assets, &compiled.value)?
                    }
                    AdapterKind::Svelte => {
                        self.render_svelte_bundle(compiler, input_path, &assets, &compiled.value)?
                    }
                    AdapterKind::Text => self.render(&compiled.value)?,
                }
            } else {
                self.render(&compiled.value)?
            }
        } else {
            self.render(&compiled.value)?
        };
        self.write_artifact(&artifact)?;
        Ok(())
    }

    fn render_jsx_svelte_project(
        &self,
        compiler: &Compiler,
        input_path: &Path,
    ) -> Result<AdapterArtifact, CliError> {
        let mut sources = Vec::new();
        for path in jsx_project_input_paths(input_path)? {
            sources.push((source_name_for_path(&path), read_to_string(&path)?));
        }
        let compiled = compiler.compile_jsx_svelte_project(sources, self.svelte_options());
        print_diagnostics(&compiled.diagnostics, &SourceMap::new());
        Ok(compiled.value)
    }

    fn render(&self, fragment: &CompiledFragment) -> Result<AdapterArtifact, CliError> {
        match self.adapter {
            AdapterKind::Text => {
                let mut emit_context = EmitContext::new();
                let output = TextEmitter
                    .emit(&fragment.plan, &mut emit_context)
                    .map_err(CliError::Emit)?;
                Ok(AdapterArtifact::new(
                    vec![GeneratedFile::new("htmlswap.txt", output)],
                    Vec::new(),
                ))
            }
            AdapterKind::Gpui => {
                let mut adapter_context = AdapterContext::new();
                let output = GpuiAdapter::new(self.gpui_options())
                    .adapt(fragment, &mut adapter_context)
                    .map_err(CliError::Adapter)?;
                print_diagnostics(adapter_context.diagnostics(), &fragment.sources);
                Ok(output.artifact)
            }
            AdapterKind::GpuiComponents => {
                let mut adapter_context = AdapterContext::new();
                let mut gpui = self.gpui_options();
                gpui.theme = ThemeEmission::PreferTheme;
                let output = GpuiComponentsAdapter::new(GpuiComponentsAdapterOptions {
                    gpui,
                    routes: self.routes(),
                })
                .adapt(fragment, &mut adapter_context)
                .map_err(CliError::Adapter)?;
                print_diagnostics(adapter_context.diagnostics(), &fragment.sources);
                Ok(output.artifact)
            }
            AdapterKind::Svelte => {
                let mut adapter_context = AdapterContext::new();
                let output = SvelteAdapter::new(self.svelte_options())
                    .adapt(fragment, &mut adapter_context)
                    .map_err(CliError::Adapter)?;
                print_diagnostics(adapter_context.diagnostics(), &fragment.sources);
                Ok(output.artifact)
            }
        }
    }

    fn render_svelte_bundle(
        &self,
        compiler: &Compiler,
        input_path: &Path,
        assets: &CompileAssets,
        root_fragment: &CompiledFragment,
    ) -> Result<AdapterArtifact, CliError> {
        let root_dir = input_path.parent().unwrap_or_else(|| Path::new("."));
        let mut adapter_context = AdapterContext::new();
        let mut artifact = ArtifactBuilder::new();
        artifact.add_artifact(
            self.render_svelte_fragment(root_fragment, &self.component_name)?,
            &mut adapter_context,
        );

        let mut seen = BTreeSet::new();
        seen.insert(normalize_component_path(input_path));
        let mut queue = VecDeque::new();
        queue_component_imports(root_dir, &root_fragment.plan, &mut seen, &mut queue)?;

        while let Some(path) = queue.pop_front() {
            let source = read_to_string(&path)?;
            let source_name = Some(source_name_for_path(&path));
            let compiled = compiler.compile_fragment_named(source_name, source, assets);
            print_diagnostics(&compiled.diagnostics, &compiled.value.sources);

            let component_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".dc.html"))
                .map(svelte_component_identifier)
                .unwrap_or_else(|| "HtmlswapComponent".to_owned());
            artifact.add_artifact(
                self.render_svelte_fragment(&compiled.value, &component_name)?,
                &mut adapter_context,
            );
            queue_component_imports(root_dir, &compiled.value.plan, &mut seen, &mut queue)?;
        }

        print_diagnostics(adapter_context.diagnostics(), &root_fragment.sources);
        Ok(artifact.finish())
    }

    fn render_gpui_dc_bundle(
        &self,
        compiler: &Compiler,
        input_path: &Path,
        assets: &CompileAssets,
        root_fragment: &CompiledFragment,
    ) -> Result<AdapterArtifact, CliError> {
        let expanded = compile_dc_bundle(compiler, input_path, assets, root_fragment)?;
        self.render(&expanded)
    }

    fn render_svelte_fragment(
        &self,
        fragment: &CompiledFragment,
        component_name: &str,
    ) -> Result<AdapterArtifact, CliError> {
        let mut adapter_context = AdapterContext::new();
        let output = SvelteAdapter::new(self.svelte_options_for_component(component_name))
            .adapt(fragment, &mut adapter_context)
            .map_err(CliError::Adapter)?;
        print_diagnostics(adapter_context.diagnostics(), &fragment.sources);
        Ok(output.artifact)
    }

    fn gpui_options(&self) -> GpuiAdapterOptions {
        let format = RustFormatOptions {
            enabled: !self.no_format,
            ..RustFormatOptions::default()
        };
        GpuiAdapterOptions {
            component_name: self.component_name.as_str().into(),
            include_dependency_header: !self.no_dependency_header,
            include_imports: !self.no_imports,
            emit_source_comments: !self.no_source_comments,
            emit_debug_source_html_comments: self.debug_source_html,
            emit_debug_layout_ids: self.debug_layout_ids,
            theme: ThemeEmission::Literal,
            format,
        }
    }

    fn svelte_options(&self) -> SvelteAdapterOptions {
        self.svelte_options_for_component(&self.component_name)
    }

    fn svelte_options_for_component(&self, component_name: &str) -> SvelteAdapterOptions {
        SvelteAdapterOptions {
            component_name: component_name.into(),
            include_dependency_header: !self.no_dependency_header,
            emit_source_comments: !self.no_source_comments,
            ..SvelteAdapterOptions::default()
        }
    }

    fn routes(&self) -> RouteConfig {
        let mut routes = GpuiComponentsAdapterOptions::components().routes;
        for route in &self.routes {
            routes = routes.with_override(route.target.clone(), route.layer.clone());
        }
        routes
    }

    fn read_input(&self) -> Result<String, CliError> {
        match self.input_path() {
            Some(path) => read_to_string(path),
            None => {
                let mut source = String::new();
                io::stdin()
                    .read_to_string(&mut source)
                    .map_err(CliError::Stdin)?;
                Ok(source)
            }
        }
    }

    fn read_assets(&self) -> Result<CompileAssets, CliError> {
        let mut assets = CompileAssets::new();
        for path in &self.stylesheets {
            assets.stylesheets.push(read_source_asset(path)?);
        }
        for path in &self.scripts {
            assets.scripts.push(read_source_asset(path)?);
        }
        Ok(assets)
    }

    fn write_artifact(&self, artifact: &AdapterArtifact) -> Result<(), CliError> {
        if let Some(root) = &self.out_dir {
            return write_artifact_dir(root, artifact);
        }

        let file = single_generated_file(artifact)?;
        if let Some(output) = &self.output {
            write_file(output, &file.contents)
        } else {
            print!("{}", file.contents);
            Ok(())
        }
    }

    fn input_path(&self) -> Option<&Path> {
        self.input.as_deref().filter(|path| path.as_os_str() != "-")
    }

    fn reads_stdin(&self) -> bool {
        self.input_path().is_none()
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        self.input_path()
            .into_iter()
            .chain(self.stylesheets.iter().map(PathBuf::as_path))
            .chain(self.scripts.iter().map(PathBuf::as_path))
            .map(Path::to_path_buf)
            .collect()
    }
}

#[derive(Debug, Args)]
struct LayoutSnapshotCommand {
    /// HTML input path. Omit or pass '-' to read from stdin.
    input: Option<PathBuf>,

    /// External CSS file. May be passed multiple times.
    #[arg(long = "css", value_name = "PATH")]
    stylesheets: Vec<PathBuf>,

    /// External JavaScript or TypeScript file. May be passed multiple times.
    #[arg(long = "js", value_name = "PATH")]
    scripts: Vec<PathBuf>,

    /// Output path. Omit to write to stdout.
    #[arg(short, long, value_name = "PATH")]
    output: Option<PathBuf>,

    /// Source dialect to lower before layout debug instrumentation.
    #[arg(long = "source", value_enum, default_value = "html")]
    source: SourceFrontendKindArg,

    /// Worker policy: auto, sequential, or a positive worker count.
    #[arg(long, default_value = "auto")]
    jobs: JobsArg,

    /// Do not fetch http(s) resources discovered in HTML or imported modules.
    #[arg(long, alias = "no-resolve-external-scripts")]
    no_resolve_remote_resources: bool,

    /// Remote resource fetch timeout in milliseconds.
    #[arg(long, alias = "external-script-timeout-ms", default_value_t = DEFAULT_RESOURCE_TIMEOUT_MS)]
    resource_timeout_ms: u64,

    /// Maximum remote resource response size in bytes.
    #[arg(long, alias = "external-script-max-bytes", default_value_t = DEFAULT_RESOURCE_MAX_BYTES)]
    resource_max_bytes: usize,

    /// Do not inject the browser snapshot helper script.
    #[arg(long)]
    no_script: bool,
}

impl LayoutSnapshotCommand {
    fn run(&self) -> Result<(), CliError> {
        let compiler = self.compiler()?;
        let source = self.read_input()?;
        let assets = self.read_assets()?;
        let source_name = self.input_path().map(source_name_for_path);
        let compiled = compiler.compile_fragment_named(source_name, source.clone(), &assets);
        print_diagnostics(&compiled.diagnostics, &compiled.value.sources);

        let output = instrument_layout_snapshot_html_with_sources(
            &source,
            &compiled.value.plan,
            Some(&compiled.value.sources),
            &LayoutDebugOptions {
                inject_script: !self.no_script,
            },
        );

        if let Some(path) = &self.output {
            write_file(path, &output)
        } else {
            print!("{output}");
            Ok(())
        }
    }

    fn compiler(&self) -> Result<Compiler, CliError> {
        let options = CompilerOptions::new()
            .with_parallelism(self.jobs.parallelism())
            .with_resources(
                CompilerResourceOptions::new()
                    .with_remote_resolution(!self.no_resolve_remote_resources)
                    .with_timeout(Duration::from_millis(self.resource_timeout_ms))
                    .with_max_bytes(self.resource_max_bytes),
            )
            .with_source_frontend(self.source.source_frontend());
        Ok(Compiler::try_with_options(options)?.with_frontend(self.source.frontend()))
    }

    fn read_input(&self) -> Result<String, CliError> {
        match self.input_path() {
            Some(path) => read_to_string(path),
            None => {
                let mut source = String::new();
                io::stdin()
                    .read_to_string(&mut source)
                    .map_err(CliError::Stdin)?;
                Ok(source)
            }
        }
    }

    fn read_assets(&self) -> Result<CompileAssets, CliError> {
        let mut assets = CompileAssets::new();
        for path in &self.stylesheets {
            assets.stylesheets.push(read_source_asset(path)?);
        }
        for path in &self.scripts {
            assets.scripts.push(read_source_asset(path)?);
        }
        Ok(assets)
    }

    fn input_path(&self) -> Option<&Path> {
        self.input.as_deref().filter(|path| path.as_os_str() != "-")
    }
}

#[derive(Debug, Args)]
struct ImportCommand {
    /// Target source path to import back to HTML.
    input: PathBuf,

    /// Target adapter that produced the input.
    #[arg(long, value_enum, default_value = "gpui")]
    adapter: AdapterKind,

    /// Output path. Omit to write to stdout.
    #[arg(short, long, value_name = "PATH")]
    output: Option<PathBuf>,
}

impl ImportCommand {
    fn run(&self) -> Result<(), CliError> {
        let contents = read_to_string(&self.input)?;
        let target = TargetArtifact::new().with_file(source_name_for_path(&self.input), contents);
        let mut adapter_context = AdapterContext::new();
        let html = match self.adapter {
            AdapterKind::Text => return Err(CliError::UnsupportedImportAdapter("text")),
            AdapterKind::Gpui => GpuiAdapter::default()
                .import(&target, &mut adapter_context)
                .map_err(CliError::Adapter)?,
            AdapterKind::GpuiComponents => GpuiComponentsAdapter::default()
                .import(&target, &mut adapter_context)
                .map_err(CliError::Adapter)?,
            AdapterKind::Svelte => return Err(CliError::UnsupportedImportAdapter("svelte")),
        };
        print_diagnostics(adapter_context.diagnostics(), &SourceMap::new());
        if let Some(output) = &self.output {
            write_file(output, html.html())
        } else {
            print!("{}", html.html());
            Ok(())
        }
    }
}

#[derive(Debug, Args)]
struct RoundtripCheckCommand {
    /// Expected/original HTML path.
    expected: PathBuf,

    /// Actual/round-tripped HTML path.
    actual: PathBuf,

    /// Source dialect used to parse both HTML documents.
    #[arg(long = "source", value_enum, default_value = "html")]
    source: SourceFrontendKindArg,

    /// Maximum mismatches to print.
    #[arg(long, default_value_t = 50)]
    max_mismatches: usize,
}

impl RoundtripCheckCommand {
    fn run(&self) -> Result<(), CliError> {
        let compiler = Compiler::try_with_options(
            CompilerOptions::new().with_source_frontend(self.source.source_frontend()),
        )?
        .with_frontend(self.source.frontend());
        let assets = CompileAssets::new();
        let expected_source = read_to_string(&self.expected)?;
        let actual_source = read_to_string(&self.actual)?;
        let expected = compiler.compile_fragment_named(
            source_name_for_path(&self.expected),
            expected_source,
            &assets,
        );
        let actual = compiler.compile_fragment_named(
            source_name_for_path(&self.actual),
            actual_source,
            &assets,
        );
        print_diagnostics(&expected.diagnostics, &expected.value.sources);
        print_diagnostics(&actual.diagnostics, &actual.value.sources);

        let expected = if self.source == SourceFrontendKindArg::Dc {
            compile_dc_bundle(&compiler, &self.expected, &assets, &expected.value)?
        } else {
            expected.value
        };
        let actual = if self.source == SourceFrontendKindArg::Dc {
            compile_dc_bundle(&compiler, &self.actual, &assets, &actual.value)?
        } else {
            actual.value
        };

        let comparison = compare_roundtrip_plans(
            &expected.plan,
            &actual.plan,
            RoundTripOptions::new().with_max_mismatches(self.max_mismatches),
        );
        println!("{comparison}");
        if comparison.is_match() {
            Ok(())
        } else {
            Err(CliError::RoundTripMismatch(comparison))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum AdapterKind {
    Text,
    Gpui,
    GpuiComponents,
    Svelte,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum SourceFrontendKindArg {
    Html,
    Dc,
    Vue,
    Jsx,
}

impl SourceFrontendKindArg {
    fn frontend(self) -> Frontend {
        match self {
            Self::Html => Frontend::html(),
            Self::Dc => Frontend::dc(),
            Self::Vue => Frontend::vue(),
            Self::Jsx => Frontend::html(),
        }
    }

    const fn source_frontend(self) -> SourceFrontendKind {
        match self {
            Self::Html => SourceFrontendKind::Html,
            Self::Dc => SourceFrontendKind::Dc,
            Self::Vue => SourceFrontendKind::Vue,
            Self::Jsx => SourceFrontendKind::Jsx,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct JobsArg(CompilerParallelism);

impl JobsArg {
    const fn parallelism(&self) -> CompilerParallelism {
        self.0
    }
}

impl std::str::FromStr for JobsArg {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "auto" => Ok(Self(CompilerParallelism::auto())),
            "sequential" => Ok(Self(CompilerParallelism::sequential())),
            _ => {
                let threads = value.parse::<usize>().map_err(|_| {
                    "expected `auto`, `sequential`, or a positive worker count".to_owned()
                })?;
                let threads = NonZeroUsize::new(threads)
                    .ok_or_else(|| "worker count must be greater than zero".to_owned())?;
                Ok(Self(CompilerParallelism::rayon_threads(threads)))
            }
        }
    }
}

#[derive(Debug, Clone)]
struct RouteOverrideArg {
    target: RouteTarget,
    layer: LayerId,
}

impl std::str::FromStr for RouteOverrideArg {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (target, layer) = value
            .split_once('=')
            .ok_or_else(|| "expected route override in the form target=layer".to_owned())?;
        Ok(Self {
            target: parse_route_target(target)?,
            layer: layer.to_owned().into(),
        })
    }
}

fn parse_route_target(value: &str) -> Result<RouteTarget, String> {
    let (kind, value) = value
        .split_once(':')
        .ok_or_else(|| "expected route target in the form kind:value".to_owned())?;
    match kind {
        "role" => Ok(RouteTarget::Role(parse_role(value)?)),
        "component" => Ok(RouteTarget::component(value)),
        "region" => RouteTarget::region(value)
            .ok_or_else(|| format!("invalid region route target `{value}`")),
        "tag" => Ok(RouteTarget::tag(value)),
        "class" => Ok(RouteTarget::class(value)),
        "style" => Ok(RouteTarget::Style(StyleProperty::from(value))),
        "event" => Ok(RouteTarget::event(value)),
        _ => Err(format!("unknown route target kind `{kind}`")),
    }
}

fn parse_role(value: &str) -> Result<UiRole, String> {
    match value.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "container" | "div" => Ok(UiRole::Container),
        "inline" | "span" => Ok(UiRole::Inline),
        "paragraph" | "p" => Ok(UiRole::Paragraph),
        "button" => Ok(UiRole::Button),
        "text-input" | "input" => Ok(UiRole::TextInput),
        "select" => Ok(UiRole::Select),
        "option" => Ok(UiRole::Option),
        "link" | "a" => Ok(UiRole::Link),
        "image" | "img" => Ok(UiRole::Image),
        "heading" => Ok(UiRole::Heading(2)),
        "h1" => Ok(UiRole::Heading(1)),
        "h2" => Ok(UiRole::Heading(2)),
        "h3" => Ok(UiRole::Heading(3)),
        "h4" => Ok(UiRole::Heading(4)),
        "h5" => Ok(UiRole::Heading(5)),
        "h6" => Ok(UiRole::Heading(6)),
        "list" | "ul" => Ok(UiRole::List { ordered: false }),
        "ordered-list" | "ol" => Ok(UiRole::List { ordered: true }),
        "list-item" | "li" => Ok(UiRole::ListItem),
        "form" => Ok(UiRole::Form),
        "fieldset" => Ok(UiRole::Fieldset),
        "legend" => Ok(UiRole::Legend),
        "label" => Ok(UiRole::Label),
        "unknown" => Ok(UiRole::Unknown),
        other => Err(format!("unknown role route target `{other}`")),
    }
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("failed to read stdin: {0}")]
    Stdin(io::Error),
    #[error("failed to read `{path}`: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("failed to write `{path}`: {source}")]
    Write { path: PathBuf, source: io::Error },
    #[error("failed to watch input files: {0}")]
    WatchInput(io::Error),
    #[error("--watch requires a file input")]
    WatchRequiresFileInput,
    #[error("compiler setup failed: {0}")]
    CompilerBuild(#[from] CompilerBuildError),
    #[error("emit failed: {0}")]
    Emit(EmitError),
    #[error("adapter failed: {0}")]
    Adapter(AdapterError),
    #[error("adapter `{0}` import is not supported")]
    UnsupportedImportAdapter(&'static str),
    #[error("adapter generated no files")]
    NoGeneratedFiles,
    #[error("adapter generated {files} files; pass --out-dir to write a file tree")]
    MultipleGeneratedFiles { files: usize },
    #[error("generated path `{0}` escapes the output directory")]
    PathEscapesOutput(String),
    #[error("DC component `{component}` was not found next to `{root}`")]
    MissingDcComponent { component: String, root: PathBuf },
    #[error("roundtrip mismatch: {0}")]
    RoundTripMismatch(RoundTripComparison),
}

fn single_generated_file(artifact: &AdapterArtifact) -> Result<&GeneratedFile, CliError> {
    match artifact.files.as_slice() {
        [] => Err(CliError::NoGeneratedFiles),
        [file] => Ok(file),
        files => Err(CliError::MultipleGeneratedFiles { files: files.len() }),
    }
}

fn write_artifact_dir(root: &Path, artifact: &AdapterArtifact) -> Result<(), CliError> {
    if artifact.files.is_empty() {
        return Err(CliError::NoGeneratedFiles);
    }
    std::fs::create_dir_all(root).map_err(|source| CliError::Write {
        path: root.to_path_buf(),
        source,
    })?;
    for file in &artifact.files {
        let path = safe_output_path(root, &file.path)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| CliError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        write_file(&path, &file.contents)?;
    }
    Ok(())
}

fn safe_output_path(root: &Path, relative: &str) -> Result<PathBuf, CliError> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::Prefix(_) | Component::RootDir
            )
        })
    {
        return Err(CliError::PathEscapesOutput(relative.to_owned()));
    }
    Ok(root.join(path))
}

fn compile_dc_bundle(
    compiler: &Compiler,
    input_path: &Path,
    assets: &CompileAssets,
    root_fragment: &CompiledFragment,
) -> Result<CompiledFragment, CliError> {
    let root_dir = input_path.parent().unwrap_or_else(|| Path::new("."));
    let mut seen = BTreeSet::new();
    seen.insert(normalize_component_path(input_path));
    let mut queue = VecDeque::new();
    queue_component_imports(root_dir, &root_fragment.plan, &mut seen, &mut queue)?;

    let mut components = Vec::new();
    while let Some(path) = queue.pop_front() {
        let source = read_to_string(&path)?;
        let source_name = Some(source_name_for_path(&path));
        let compiled = compiler.compile_fragment_named(source_name, source, assets);
        print_diagnostics(&compiled.diagnostics, &compiled.value.sources);

        let component_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".dc.html"))
            .unwrap_or("HtmlswapComponent")
            .to_owned();
        queue_component_imports(root_dir, &compiled.value.plan, &mut seen, &mut queue)?;
        components.push(DcComponentFragment::new(component_name, compiled.value));
    }

    Ok(inline_dc_component_imports(
        root_fragment.clone(),
        components,
    ))
}

fn queue_component_imports(
    root_dir: &Path,
    plan: &RenderPlan,
    seen: &mut BTreeSet<PathBuf>,
    queue: &mut VecDeque<PathBuf>,
) -> Result<(), CliError> {
    let mut components = BTreeSet::new();
    collect_dc_component_imports(&plan.nodes, &mut components);
    for component in components {
        let path = resolve_dc_component_path(root_dir, &component)?;
        if seen.insert(normalize_component_path(&path)) {
            queue.push_back(path);
        }
    }
    Ok(())
}

fn collect_dc_component_imports(nodes: &[RenderNode], components: &mut BTreeSet<String>) {
    for node in nodes {
        let RenderNode::Element(element) = node else {
            continue;
        };
        if element.source_tag == "dc-import"
            && let Some(intent) = element.source_intent.as_deref()
            && let Some(component) = &intent.component
        {
            components.insert(component.to_string());
        }
        collect_dc_component_imports(&element.children, components);
    }
}

fn resolve_dc_component_path(root_dir: &Path, component: &str) -> Result<PathBuf, CliError> {
    let exact = root_dir.join(format!("{component}.dc.html"));
    if exact.exists() {
        return Ok(exact);
    }

    let expected = format!("{component}.dc.html");
    let entries = std::fs::read_dir(root_dir).map_err(|source| CliError::Read {
        path: root_dir.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| CliError::Read {
            path: root_dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(&expected))
        {
            return Ok(path);
        }
    }

    Err(CliError::MissingDcComponent {
        component: component.to_owned(),
        root: root_dir.to_path_buf(),
    })
}

fn jsx_project_input_paths(root: &Path) -> Result<Vec<PathBuf>, CliError> {
    let mut paths = Vec::new();
    let entries = std::fs::read_dir(root).map_err(|source| CliError::Read {
        path: root.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| CliError::Read {
            path: root.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsx") {
            continue;
        }
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                matches!(
                    name,
                    "design-canvas.jsx" | "tweaks-panel.jsx" | "theme-tweaks.jsx"
                )
            })
        {
            continue;
        }
        paths.push(path);
    }
    paths.sort();
    Ok(paths)
}

fn normalize_component_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn svelte_component_identifier(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        let mut output = String::with_capacity(value.len());
        for (index, ch) in value.chars().enumerate() {
            if index == 0 {
                if ch.is_ascii_digit() {
                    output.push('_');
                    output.push(ch);
                } else {
                    output.push(ch.to_ascii_uppercase());
                }
            } else {
                output.push(ch);
            }
        }
        return output;
    }

    value.to_upper_camel_case()
}

fn read_source_asset(path: &Path) -> Result<SourceAsset, CliError> {
    Ok(SourceAsset::new(
        Some(source_name_for_path(path)),
        read_to_string(path)?,
    ))
}

fn read_to_string(path: &Path) -> Result<String, CliError> {
    std::fs::read_to_string(path).map_err(|source| CliError::Read {
        path: path.to_path_buf(),
        source,
    })
}

fn write_file(path: &Path, contents: &str) -> Result<(), CliError> {
    std::fs::write(path, contents).map_err(|source| CliError::Write {
        path: path.to_path_buf(),
        source,
    })
}

fn source_name_for_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn print_diagnostics(diagnostics: &Diagnostics, sources: &SourceMap) {
    for diagnostic in diagnostics {
        if let Some(span) = diagnostic.span
            && let Some(file) = sources.file(span.source)
            && let Some(line_column) = file.line_column(span.start)
        {
            let name = file.name().unwrap_or("<input>");
            eprintln!(
                "{}:{}:{}: {}: {}",
                name, line_column.line, line_column.column, diagnostic.severity, diagnostic.message
            );
            continue;
        }
        eprintln!("{}: {}", diagnostic.severity, diagnostic.message);
    }
}
