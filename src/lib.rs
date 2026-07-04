pub use arcstr::{ArcStr, Substr};
pub use compact_str::CompactString;

pub mod adapter;
pub mod adapters;
pub mod assets;
pub mod bundle;
pub mod class;
pub mod compiler;
mod css;
pub mod dc_bundle;
pub mod diagnostics;
pub mod emit;
pub mod expr;
pub mod frontend;
pub mod ir;
mod jsx;
pub mod layout_debug;
mod lower;
mod material_symbols;
pub mod module_graph;
pub mod parse;
pub mod plan;
pub mod resource;
pub mod roundtrip;
mod script;
pub mod source;
mod style;
pub mod style_provider;
pub mod work;

pub use adapter::{
    Adapter, AdapterArtifact, AdapterContext, AdapterError, ArtifactBuilder, DependencySet,
    DependencySource, GeneratedFile, HtmlArtifact, Importer, Layer, LayerBinding, LayerClaim,
    LayerId, RouteConfig, RouteOverride, RouteTarget, RouteTargetRef, TargetArtifact,
    TargetDependency, TargetSourceFile,
};
pub use adapters::gpui::{
    GPUI_CRATE_VERSION, GPUI_LAYER_ID, GpuiAdapter, GpuiAdapterOptions, GpuiOutput,
    RustFormatOptions, ThemeEmission,
};
pub use adapters::gpui_components::{
    GPUI_COMPONENT_CRATE_VERSION, GPUI_COMPONENTS_LAYER_ID, GpuiComponentsAdapter,
    GpuiComponentsAdapterOptions,
};
pub use adapters::svelte::{
    SVELTE_LAYER_ID, SVELTE_PACKAGE_VERSION, SvelteAdapter, SvelteAdapterOptions, SvelteOutput,
};
pub use assets::{CompileAssets, SourceAsset};
pub use bundle::{
    BundleConfig, BundlePlan, OutputBoundary, OutputBoundaryRule, OutputDependency, OutputSource,
    OutputUnit, OutputUnitId, OutputUnitKind,
};
pub use class::{ClassCandidate, ClassIndex};
pub use compiler::{
    CompiledFragment, Compiler, CompilerBuildError, CompilerCache, CompilerCacheFileSet,
    CompilerCacheFileStamp, CompilerCacheStats, CompilerExternalScriptOptions, CompilerOptions,
    CompilerParallelism, CompilerResourceOptions, DEFAULT_EXTERNAL_SCRIPT_MAX_BYTES,
    DEFAULT_EXTERNAL_SCRIPT_TIMEOUT, DEFAULT_RESOURCE_MAX_BYTES, DEFAULT_RESOURCE_TIMEOUT,
    SourceFrontendKind,
};
pub use dc_bundle::{DcComponentFragment, inline_dc_component_imports};
pub use diagnostics::{Compilation, Diagnostic, Diagnostics, Severity};
pub use emit::{EmitContext, EmitError, Emitter, HtmlEmitter, TextEmitter};
pub use expr::{BindingPattern, Expr, ExprLiteral, ObjectEntry, TemplateSegment, TemplateString};
pub use frontend::{
    AttributeRewrite, DcDialect, DialectContext, ElementDirectives, Frontend, SourceDialect,
    VueDialect,
};
pub use ir::{HtmlAttribute, HtmlComment, HtmlDocument, HtmlElement, HtmlName, HtmlNode, HtmlText};
pub use layout_debug::{
    LayoutDebugOptions, instrument_layout_snapshot_html,
    instrument_layout_snapshot_html_for_source, instrument_layout_snapshot_html_with_sources,
    layout_debug_id_for_element, layout_debug_id_for_element_in_sources, layout_debug_id_for_span,
    layout_debug_id_for_span_in_sources, layout_source_key_for_span,
    layout_source_key_for_span_in_sources, layout_source_scope,
};
pub use lower::{LowerOptions, lower_document};
pub use module_graph::{
    ModuleChunk, ModuleEdge, ModuleEdgeKind, ModuleGraph, ModuleGraphInput, ModuleGraphOptions,
    ModuleGraphProvider, ModuleNode, ModuleNodeId,
};
pub use parse::{parse_document, parse_fragment};
pub use plan::{
    ActionBinding, ActionPayload, ComponentId, RegionId, RenderAccessibility, RenderActionArgument,
    RenderActionEffect, RenderActionHandler, RenderActionHandlerEffect, RenderActionInvocation,
    RenderActionPlan, RenderAnnotation, RenderAnnotationKind, RenderAttribute, RenderChoiceOption,
    RenderChoiceState, RenderControlFlow, RenderControlFlowHost, RenderControlFlowKind,
    RenderDensity, RenderDynamicStyleBinding, RenderElement, RenderFormBinding, RenderFormControl,
    RenderFormControlBinding, RenderFormControlType, RenderFormDataField, RenderFormSubmit,
    RenderHeadElement, RenderLoopLocal, RenderNode, RenderPlan, RenderPseudoElement, RenderRaw,
    RenderRoot, RenderScriptKind, RenderScriptReference, RenderSemanticExtra, RenderSemantics,
    RenderSize, RenderSourceIntent, RenderSourceLogic, RenderSourceProp, RenderStateBinding,
    RenderStateKind, RenderStateOwner, RenderStatePlan, RenderStateValueSource,
    RenderStyleCondition, RenderStyleVariant, RenderText, RenderTextInputState, RenderThemePlan,
    RenderThemeScope, RenderThemeToken, RenderThemeTokenValue, RenderToggleState, RenderTone,
    RenderValidation, RenderValidationConstraint, RenderVariant, SlotId, ThemeTokenKind, UiRole,
};
pub use resource::{
    DefaultResourceResolver, FileSystemResourceResolver, FileSystemResourceResolverOptions,
    HttpResourceResolver, HttpResourceResolverOptions, NoopResourceResolver, ResourceKind,
    ResourceReferrer, ResourceRequest, ResourceResolver, ResourceSource,
};
pub use roundtrip::{
    RoundTripComparison, RoundTripFeatureKind, RoundTripMismatch, RoundTripMismatchKind,
    RoundTripOptions, compare_roundtrip_plans,
};
pub use source::{
    GeneratedSourceMap, GeneratedSpan, LineColumn, SourceFile, SourceId, SourceKind, SourceMap,
    SourceMapping, SourceMappingKind, Span,
};
pub use style::{StyleDeclaration, StyleOrigin, StyleProperty, StyleToken, StyleValue};
pub use style_provider::{
    GeneratedStyleSource, StyleProvider, StyleProviderInput, StyleProviderOutput,
};
pub use work::{
    CancellationToken, JobBatch, JobContext, JobRunner, JobRunnerBuildError, JobRunnerPolicy,
};

#[must_use]
pub fn compile_fragment(source: impl Into<ArcStr>) -> Compilation<RenderPlan> {
    compile_fragment_with_assets(source, &CompileAssets::default())
}

#[must_use]
pub fn compile_fragment_with_assets(
    source: impl Into<ArcStr>,
    assets: &CompileAssets,
) -> Compilation<RenderPlan> {
    compiler::compile_fragment_with_sources(source.into(), assets).map(|compiled| compiled.plan)
}
