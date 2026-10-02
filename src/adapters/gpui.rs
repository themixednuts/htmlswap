use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use compact_str::CompactString;
use heck::ToSnakeCase;
use lightningcss::traits::Parse;
use lightningcss::values::color::CssColor;
use rustfmt_wrapper::config::{Config as RustfmtConfig, Edition, NewlineStyle};
use rustfmt_wrapper::rustfmt_config;
use smallvec::SmallVec;

use crate::SourceMap;
use crate::adapter::{
    Adapter, AdapterArtifact, AdapterContext, AdapterError, DependencySet, GeneratedFile,
    HtmlArtifact, Importer, LayerClaim, RouteConfig, RouteTargetRef, TargetArtifact,
    TargetDependency,
};
use crate::adapters::gpui_reverse::{GpuiImportMode, import_gpui_target};
use crate::compiler::CompiledFragment;
use crate::diagnostics::Diagnostic;
use crate::emit::render_html_fragment;
use crate::expr::{Expr, ExprLiteral, TemplateSegment, TemplateString};
use crate::layout_debug::{
    layout_debug_id_for_element_in_sources, layout_source_key_for_span_in_sources,
};
use crate::material_symbols::material_symbol_svg_files;
use crate::plan::{
    ActionBinding, ActionPayload, RenderActionArgument, RenderActionHandlerEffect,
    RenderAnnotation, RenderChoiceState, RenderControlFlowHost, RenderControlFlowKind,
    RenderDynamicStyleBinding, RenderElement, RenderFormControlType, RenderFormDataField,
    RenderNode, RenderPlan, RenderPseudoElement, RenderRaw, RenderStateBinding, RenderStateKind,
    RenderStateOwner, RenderStateValueSource, RenderStyleCondition, RenderStyleVariant, RenderText,
    RenderTextInputState, RenderThemePlan, RenderToggleState, UiRole,
};
use crate::source::{GeneratedSourceMap, GeneratedSpan, SourceMapping, SourceMappingKind, Span};
use crate::style::{StyleDeclaration, StyleProperty, StyleToken, StyleValue};

pub const GPUI_CRATE_VERSION: &str = "0.2.2";
pub const GPUI_LAYER_ID: &str = "gpui";
const GPUI_COMPONENT_THEME_CRATE_VERSION: &str = "0.5.1";
const GPUI_MATERIAL_SYMBOL_ICON_HELPER: &str = r#"struct HtmlswapMaterialSymbolIcon {
    name: String,
}

fn htmlswap_material_symbol_icon(name: impl AsRef<str>) -> HtmlswapMaterialSymbolIcon {
    HtmlswapMaterialSymbolIcon {
        name: name.as_ref().trim().to_owned(),
    }
}

impl gpui::RenderOnce for HtmlswapMaterialSymbolIcon {
    fn render(self, window: &mut gpui::Window, _cx: &mut gpui::App) -> impl gpui::IntoElement {
        let symbol = htmlswap_material_symbol_asset_name(&self.name);
        let text_size = window.text_style().font_size.to_pixels(window.rem_size());

        gpui::svg()
            .flex_none()
            .size(text_size)
            .path(format!("icons/dc/{symbol}.svg"))
    }
}

fn htmlswap_material_symbol_asset_name(name: &str) -> &str {
    match name.trim() {
        "android" => "smart_toy",
        "arrow_up" => "arrow_upward",
        "bug_report" => "warning",
        "language" | "public" => "lan",
        "phone_iphone" => "smartphone",
        "space" => "space_bar",
        "" => "help",
        name if htmlswap_is_safe_material_symbol_name(name) => name,
        _ => "help",
    }
}

fn htmlswap_is_safe_material_symbol_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}
"#;

const GPUI_DYNAMIC_COLOR_HELPER: &str = r#"fn htmlswap_dynamic_color(value: impl AsRef<str>) -> gpui::Rgba {
    let value = value.as_ref().trim();
    if let Some(color) = htmlswap_dynamic_hex_color(value) {
        return color;
    }

    match value.to_ascii_lowercase().as_str() {
        "transparent" => gpui::rgba(0x00000000),
        "black" => gpui::rgb(0x000000),
        "white" => gpui::rgb(0xFFFFFF),
        "red" => gpui::rgb(0xFF0000),
        "green" => gpui::rgb(0x008000),
        "blue" => gpui::rgb(0x0000FF),
        _ => gpui::rgb(0x000000),
    }
}

fn htmlswap_dynamic_hex_color(value: &str) -> Option<gpui::Rgba> {
    let hex = value.strip_prefix('#')?;
    match hex.len() {
        3 => {
            let mut expanded = String::with_capacity(6);
            for ch in hex.chars() {
                expanded.push(ch);
                expanded.push(ch);
            }
            u32::from_str_radix(&expanded, 16).ok().map(gpui::rgb)
        }
        4 => {
            let mut expanded = String::with_capacity(8);
            for ch in hex.chars() {
                expanded.push(ch);
                expanded.push(ch);
            }
            u32::from_str_radix(&expanded, 16).ok().map(gpui::rgba)
        }
        6 => u32::from_str_radix(hex, 16).ok().map(gpui::rgb),
        8 => u32::from_str_radix(hex, 16).ok().map(gpui::rgba),
        _ => None,
    }
}
"#;

const GPUI_DYNAMIC_LENGTH_HELPER: &str = r#"fn htmlswap_dynamic_definite_length(value: impl AsRef<str>) -> gpui::DefiniteLength {
    let value = value.as_ref().trim();
    if let Some(number) = value.strip_suffix("px").and_then(|value| value.trim().parse::<f32>().ok()) {
        return gpui::px(number).into();
    }
    if let Some(number) = value.strip_suffix("rem").and_then(|value| value.trim().parse::<f32>().ok()) {
        return gpui::rems(number).into();
    }
    if let Some(number) = value.strip_suffix("em").and_then(|value| value.trim().parse::<f32>().ok()) {
        return gpui::rems(number).into();
    }
    if let Some(number) = value.strip_suffix('%').and_then(|value| value.trim().parse::<f32>().ok()) {
        return gpui::relative(number / 100.0).into();
    }
    if let Ok(number) = value.parse::<f32>() {
        return gpui::px(number).into();
    }
    gpui::px(0.0).into()
}

fn htmlswap_dynamic_length(value: impl AsRef<str>) -> gpui::Length {
    let value = value.as_ref().trim();
    if value.eq_ignore_ascii_case("auto") || value.eq_ignore_ascii_case("none") {
        return gpui::Length::Auto;
    }
    htmlswap_dynamic_definite_length(value).into()
}
"#;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThemeEmission {
    #[default]
    Literal,
    PreferTheme,
    RequireTheme,
    ExtractTheme,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuiAdapterOptions {
    pub component_name: CompactString,
    pub include_dependency_header: bool,
    pub include_imports: bool,
    pub emit_source_comments: bool,
    pub emit_debug_source_html_comments: bool,
    pub emit_debug_layout_ids: bool,
    pub theme: ThemeEmission,
    pub format: RustFormatOptions,
}

impl Default for GpuiAdapterOptions {
    fn default() -> Self {
        Self::gpui()
    }
}

impl GpuiAdapterOptions {
    #[must_use]
    pub fn gpui() -> Self {
        Self {
            component_name: "HtmlswapView".into(),
            include_dependency_header: true,
            include_imports: true,
            emit_source_comments: true,
            emit_debug_source_html_comments: false,
            emit_debug_layout_ids: false,
            theme: ThemeEmission::Literal,
            format: RustFormatOptions::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustFormatOptions {
    pub enabled: bool,
    pub hard_tabs: bool,
    pub tab_spaces: usize,
    pub max_width: usize,
}

impl Default for RustFormatOptions {
    fn default() -> Self {
        Self::cargo_fmt_tabs()
    }
}

impl RustFormatOptions {
    #[must_use]
    pub fn cargo_fmt_tabs() -> Self {
        Self {
            enabled: true,
            hard_tabs: true,
            tab_spaces: 4,
            max_width: 100,
        }
    }

    #[must_use]
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::cargo_fmt_tabs()
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct GpuiAdapter {
    options: GpuiAdapterOptions,
}

impl GpuiAdapter {
    #[must_use]
    pub fn new(options: GpuiAdapterOptions) -> Self {
        Self { options }
    }

    #[must_use]
    pub fn options(&self) -> &GpuiAdapterOptions {
        &self.options
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuiOutput {
    pub artifact: AdapterArtifact,
}

impl GpuiOutput {
    #[must_use]
    pub fn code(&self) -> &str {
        self.artifact
            .files
            .first()
            .map_or("", |file| file.contents.as_str())
    }

    #[must_use]
    pub fn dependencies(&self) -> &[TargetDependency] {
        &self.artifact.dependencies
    }
}

impl Adapter for GpuiAdapter {
    type Output = GpuiOutput;

    fn adapt(
        &self,
        fragment: &CompiledFragment,
        cx: &mut AdapterContext,
    ) -> Result<Self::Output, AdapterError> {
        let routes = RouteConfig::new().with_base(GPUI_LAYER_ID);
        let layers = [gpui_base_layer()];
        adapt_with_layers(&self.options, &routes, &layers, fragment, cx)
    }
}

impl Importer for GpuiAdapter {
    type Source = TargetArtifact;
    type Output = HtmlArtifact;

    fn import(
        &self,
        source: &Self::Source,
        cx: &mut AdapterContext,
    ) -> Result<Self::Output, AdapterError> {
        import_gpui_target(source, cx, GpuiImportMode::Gpui)
    }
}

pub(crate) fn adapt_with_layers<'a>(
    options: &GpuiAdapterOptions,
    routes: &RouteConfig,
    available_layers: &'a [&'a dyn GpuiTargetLayer],
    fragment: &CompiledFragment,
    cx: &mut AdapterContext,
) -> Result<GpuiOutput, AdapterError> {
    if !is_rust_identifier(&options.component_name) {
        return Err(AdapterError::Message {
            message: format!(
                "`{}` is not a valid Rust type identifier",
                options.component_name
            ),
        });
    }

    routes.validate(cx);
    let active_layers = active_layers(available_layers, routes);
    let mut codegen = GpuiCodegen::new(options, routes, available_layers, &active_layers, cx);
    let rendered = codegen.render(fragment);
    let formatted = format_generated_rust(rendered.code, &options.format)?;
    let marked = strip_source_map_markers(&formatted, &rendered.source_markers)?;
    let marked = reformat_marker_free_rust(marked, &options.format)?;

    let mut files = vec![
        GeneratedFile::new(
            format!("{}.rs", options.component_name.to_ascii_lowercase()),
            marked.code,
        )
        .with_source_map(marked.source_map),
    ];
    files.extend(rendered.asset_files);

    Ok(GpuiOutput {
        artifact: AdapterArtifact::new(files, rendered.dependencies),
    })
}

fn format_generated_rust(
    source: String,
    options: &RustFormatOptions,
) -> Result<String, AdapterError> {
    if !options.enabled {
        return Ok(source);
    }

    let config = RustfmtConfig {
        edition: Some(Edition::Edition2024),
        hard_tabs: Some(options.hard_tabs),
        tab_spaces: Some(options.tab_spaces),
        max_width: Some(options.max_width),
        newline_style: Some(NewlineStyle::Unix),
        ..Default::default()
    };

    rustfmt_config(config, source).map_err(|error| AdapterError::Message {
        message: format!("failed to format generated Rust with rustfmt: {error}"),
    })
}

fn reformat_marker_free_rust(
    marked: MarkedRust,
    options: &RustFormatOptions,
) -> Result<MarkedRust, AdapterError> {
    if !options.enabled {
        return Ok(marked);
    }

    let reformatted = format_generated_rust(marked.code.clone(), options)?;
    if reformatted == marked.code {
        return Ok(marked);
    }

    let source_map =
        remap_source_map_after_reformat(&marked.code, &reformatted, &marked.source_map)?;
    Ok(MarkedRust {
        code: reformatted,
        source_map,
    })
}

fn remap_source_map_after_reformat(
    before: &str,
    after: &str,
    source_map: &GeneratedSourceMap,
) -> Result<GeneratedSourceMap, AdapterError> {
    let before_counts = non_whitespace_prefix_counts(before);
    let after_offsets = NonWhitespaceOffsets::new(after);
    let mappings = source_map
        .mappings()
        .iter()
        .map(|mapping| {
            let start = after_offsets.start_offset(
                before_counts
                    .get(mapping.generated.start)
                    .copied()
                    .unwrap_or_else(|| before_counts.last().copied().unwrap_or_default()),
            );
            let end = after_offsets
                .end_offset(
                    before_counts
                        .get(mapping.generated.end)
                        .copied()
                        .unwrap_or_else(|| before_counts.last().copied().unwrap_or_default()),
                )
                .max(start);
            SourceMapping::new(
                GeneratedSpan::new(start, end),
                mapping.original,
                mapping.kind,
            )
        })
        .collect();
    Ok(GeneratedSourceMap::new(mappings))
}

fn non_whitespace_prefix_counts(source: &str) -> Vec<usize> {
    let mut counts = Vec::with_capacity(source.len() + 1);
    counts.push(0);
    let mut count = 0;
    for byte in source.bytes() {
        if !byte.is_ascii_whitespace() {
            count += 1;
        }
        counts.push(count);
    }
    counts
}

struct NonWhitespaceOffsets {
    before_offsets: Vec<usize>,
    after_offsets: Vec<usize>,
}

impl NonWhitespaceOffsets {
    fn new(source: &str) -> Self {
        let total = source
            .bytes()
            .filter(|byte| !byte.is_ascii_whitespace())
            .count();
        let mut before_offsets = vec![source.len(); total + 1];
        let mut after_offsets = vec![source.len(); total + 1];
        after_offsets[0] = 0;

        let mut seen = 0;
        for (index, byte) in source.bytes().enumerate() {
            if byte.is_ascii_whitespace() {
                continue;
            }
            before_offsets[seen] = index;
            seen += 1;
            after_offsets[seen] = index + 1;
        }
        before_offsets[seen] = source.len();

        Self {
            before_offsets,
            after_offsets,
        }
    }

    fn start_offset(&self, count: usize) -> usize {
        self.before_offsets
            .get(count)
            .copied()
            .unwrap_or_else(|| self.before_offsets.last().copied().unwrap_or_default())
    }

    fn end_offset(&self, count: usize) -> usize {
        self.after_offsets
            .get(count)
            .copied()
            .unwrap_or_else(|| self.after_offsets.last().copied().unwrap_or_default())
    }
}

fn strip_source_map_markers(
    source: &str,
    markers: &[GeneratedSourceMarker],
) -> Result<MarkedRust, AdapterError> {
    let marker_lookup = markers
        .iter()
        .map(|marker| (marker.id, *marker))
        .collect::<BTreeMap<_, _>>();
    let mut starts = BTreeMap::<usize, usize>::new();
    let mut source_map = GeneratedSourceMap::new(Vec::new());
    let mut code = String::with_capacity(source.len());
    let mut index = 0;

    while index < source.len() {
        if let Some(id) = read_source_map_marker(source, &mut index, SOURCE_MAP_START)? {
            strip_standalone_marker_line(&mut code, source, &mut index);
            starts.insert(id, code.len());
            continue;
        }

        if let Some(id) = read_source_map_marker(source, &mut index, SOURCE_MAP_END)? {
            strip_standalone_marker_line(&mut code, source, &mut index);
            trim_trailing_horizontal_whitespace(&mut code);
            let Some(start) = starts.remove(&id) else {
                return Err(AdapterError::Message {
                    message: format!("source map end marker `{id}` has no start marker"),
                });
            };
            let Some(marker) = marker_lookup.get(&id) else {
                return Err(AdapterError::Message {
                    message: format!("source map marker `{id}` has no metadata"),
                });
            };
            source_map.push(SourceMapping::new(
                GeneratedSpan::new(start, code.len()),
                marker.original,
                marker.kind,
            ));
            continue;
        }

        let ch = source[index..]
            .chars()
            .next()
            .expect("index is inside a valid string slice");
        code.push(ch);
        index += ch.len_utf8();
    }

    if let Some(id) = starts.keys().next() {
        return Err(AdapterError::Message {
            message: format!("source map start marker `{id}` has no end marker"),
        });
    }

    Ok(MarkedRust { code, source_map })
}

fn strip_standalone_marker_line(code: &mut String, source: &str, index: &mut usize) {
    let line_start = code.rfind('\n').map_or(0, |position| position + 1);
    if !code[line_start..]
        .chars()
        .all(|ch| matches!(ch, ' ' | '\t'))
    {
        return;
    }

    let Some(rest) = source.get(*index..) else {
        return;
    };

    if let Some(stripped) = rest.strip_prefix("\r\n") {
        code.truncate(line_start);
        *index = source.len() - stripped.len();
    } else if let Some(stripped) = rest.strip_prefix('\n') {
        code.truncate(line_start);
        *index = source.len() - stripped.len();
    }
}

fn trim_trailing_horizontal_whitespace(code: &mut String) {
    while matches!(code.as_bytes().last(), Some(b' ' | b'\t')) {
        code.pop();
    }
}

fn read_source_map_marker(
    source: &str,
    index: &mut usize,
    prefix: &str,
) -> Result<Option<usize>, AdapterError> {
    let Some(rest) = source.get(*index..) else {
        return Ok(None);
    };
    if !rest.starts_with(prefix) {
        return Ok(None);
    }

    let id_start = *index + prefix.len();
    let Some(id_end_offset) = source[id_start..].find(SOURCE_MAP_CLOSE) else {
        return Err(AdapterError::Message {
            message: "unterminated source map marker".to_owned(),
        });
    };
    let id_end = id_start + id_end_offset;
    let id = source[id_start..id_end]
        .parse::<usize>()
        .map_err(|error| AdapterError::Message {
            message: format!("invalid source map marker id: {error}"),
        })?;
    *index = id_end + SOURCE_MAP_CLOSE.len();
    Ok(Some(id))
}

pub(crate) trait GpuiTargetLayer {
    fn id(&self) -> &'static str;
    fn claim(&self) -> LayerClaim;
    fn dependencies(&self) -> Vec<TargetDependency>;
    fn write_imports(&self, output: &mut String);
    fn write_helpers(&self, _output: &mut String) {}
    fn element_spec(
        &self,
        element: &RenderElement,
        context: &mut GpuiElementContext<'_>,
    ) -> Option<GpuiElementSpec>;
}

pub(crate) struct GpuiElementContext<'a> {
    next_id: &'a mut usize,
    emit_debug_layout_ids: bool,
    sources: Option<&'a SourceMap>,
    state_fields: &'a BTreeMap<CompactString, String>,
    state_field_backends: &'a mut BTreeMap<CompactString, GpuiStateFieldBackend>,
    used_state_ids: &'a mut BTreeSet<CompactString>,
    cx: &'a mut AdapterContext,
}

impl GpuiElementContext<'_> {
    #[must_use]
    pub(crate) fn generated_id(&mut self, prefix: &str) -> String {
        let id = format!("htmlswap_{prefix}_{}", self.next_id);
        *self.next_id += 1;
        id
    }

    #[must_use]
    pub(crate) fn element_id_or_generated_id(
        &mut self,
        element: &RenderElement,
        prefix: &str,
    ) -> String {
        element_id(element).map(str::to_owned).unwrap_or_else(|| {
            if self.emit_debug_layout_ids
                && let Some(id) = layout_debug_id_for_element_in_sources(element, self.sources)
            {
                return id;
            }
            self.generated_id(prefix)
        })
    }

    #[must_use]
    pub(crate) fn state_field(&mut self, state: &RenderStateBinding) -> Option<String> {
        self.state_field_with_backend(state, GpuiStateFieldBackend::ComponentTextInput)
    }

    pub(crate) fn state_field_with_backend(
        &mut self,
        state: &RenderStateBinding,
        backend: GpuiStateFieldBackend,
    ) -> Option<String> {
        let field = self.state_fields.get(&state.id)?;
        self.used_state_ids.insert(state.id.clone());
        if matches!(&state.kind, RenderStateKind::TextInput(_)) {
            self.state_field_backends.insert(state.id.clone(), backend);
        }
        Some(format!("self.{field}"))
    }

    pub(crate) fn warn(&mut self, message: impl Into<String>, span: Option<Span>) {
        self.cx.push(Diagnostic::warning(message, span));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GpuiStateFieldBackend {
    GpuiTextInput,
    ComponentTextInput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GpuiElementSpec {
    expression: String,
    id_attribute: IdAttributeEmission,
    title_attribute: TitleAttributeEmission,
    children: ChildEmission,
    styles: StyleEmission,
    style_variants: StyleEmission,
    actions: ActionEmission,
    state_actions: StateActionEmission,
    accessibility: AccessibilityEmission,
    consumed_attributes: SmallVec<[&'static str; 8]>,
    unsupported_action_events: SmallVec<[&'static str; 4]>,
}

impl GpuiElementSpec {
    #[must_use]
    pub(crate) fn new(expression: impl Into<String>) -> Self {
        Self {
            expression: expression.into(),
            id_attribute: IdAttributeEmission::Method,
            title_attribute: TitleAttributeEmission::Warn,
            children: ChildEmission::Children,
            styles: StyleEmission::Methods,
            style_variants: StyleEmission::Methods,
            actions: ActionEmission::Events,
            state_actions: StateActionEmission::Comment,
            accessibility: AccessibilityEmission::Comment,
            consumed_attributes: SmallVec::new(),
            unsupported_action_events: SmallVec::new(),
        }
    }

    #[must_use]
    pub(crate) fn with_id_attribute(mut self, id_attribute: IdAttributeEmission) -> Self {
        self.id_attribute = id_attribute;
        self
    }

    #[must_use]
    pub(crate) fn with_title_attribute(mut self, title_attribute: TitleAttributeEmission) -> Self {
        self.title_attribute = title_attribute;
        self
    }

    #[must_use]
    pub(crate) fn with_children(mut self, children: ChildEmission) -> Self {
        self.children = children;
        self
    }

    #[must_use]
    pub(crate) fn with_style_variants(mut self, style_variants: StyleEmission) -> Self {
        self.style_variants = style_variants;
        self
    }

    #[must_use]
    pub(crate) fn with_actions(mut self, actions: ActionEmission) -> Self {
        self.actions = actions;
        self
    }

    #[must_use]
    pub(crate) fn with_state_actions(mut self, state_actions: StateActionEmission) -> Self {
        self.state_actions = state_actions;
        self
    }

    #[must_use]
    pub(crate) fn with_accessibility(mut self, accessibility: AccessibilityEmission) -> Self {
        self.accessibility = accessibility;
        self
    }

    #[must_use]
    pub(crate) fn with_consumed_attribute(mut self, attribute: &'static str) -> Self {
        self.consumed_attributes.push(attribute);
        self
    }

    #[must_use]
    pub(crate) fn with_consumed_attributes(
        mut self,
        attributes: impl IntoIterator<Item = &'static str>,
    ) -> Self {
        self.consumed_attributes.extend(attributes);
        self
    }

    #[must_use]
    pub(crate) fn with_unsupported_action_events(
        mut self,
        events: impl IntoIterator<Item = &'static str>,
    ) -> Self {
        self.unsupported_action_events.extend(events);
        self
    }

    #[must_use]
    fn consumes_attribute(&self, attribute: &str) -> bool {
        self.consumed_attributes.contains(&attribute)
    }

    #[must_use]
    fn supports_action_event(&self, event: &str) -> bool {
        !self.unsupported_action_events.contains(&event)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdAttributeEmission {
    Method,
    Consumed,
    Warn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TitleAttributeEmission {
    Warn,
    ComponentTooltip,
    GpuiTooltip,
    GpuiTooltipWrapper,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildEmission {
    Children,
    TextAsLabel,
    MaterialIcon,
    TextInputPlaceholder,
    TitleBarChildren,
    FormFields,
    FieldsetChildren,
    Consumed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StyleEmission {
    Methods,
    Comment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActionEmission {
    Events,
    Delegate,
    Comment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StateActionEmission {
    Methods,
    Comment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccessibilityEmission {
    Methods,
    Comment,
}

#[must_use]
pub(crate) fn component_text_input_host_expression(input_expression: &str) -> String {
    format!("gpui::div().child({input_expression}.w_full().h_full())")
}

struct GpuiBaseLayer;

impl GpuiTargetLayer for GpuiBaseLayer {
    fn id(&self) -> &'static str {
        GPUI_LAYER_ID
    }

    fn claim(&self) -> LayerClaim {
        LayerClaim::All
    }

    fn dependencies(&self) -> Vec<TargetDependency> {
        vec![TargetDependency::crates_io("gpui", GPUI_CRATE_VERSION)]
    }

    fn write_imports(&self, output: &mut String) {
        output.push_str("use gpui::{AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window};\n");
        output.push_str("use gpui::prelude::FluentBuilder as _;\n");
    }

    fn element_spec(
        &self,
        element: &RenderElement,
        _context: &mut GpuiElementContext<'_>,
    ) -> Option<GpuiElementSpec> {
        if is_material_symbol_element(element) {
            return Some(
                GpuiElementSpec::new("gpui::div()")
                    .with_children(ChildEmission::MaterialIcon)
                    .with_actions(ActionEmission::Events)
                    .with_accessibility(AccessibilityEmission::Comment),
            );
        }

        let children = if element.role == UiRole::TextInput {
            ChildEmission::TextInputPlaceholder
        } else {
            ChildEmission::Children
        };

        if element.role == UiRole::TextInput
            && let Some(state) = &element.state
            && let Some(state_field) =
                _context.state_field_with_backend(state, GpuiStateFieldBackend::GpuiTextInput)
        {
            return Some(
                GpuiElementSpec::new(format!("gpui::div().child({state_field}.clone())"))
                    .with_children(ChildEmission::Consumed)
                    .with_title_attribute(TitleAttributeEmission::GpuiTooltip)
                    .with_actions(ActionEmission::Events)
                    .with_state_actions(StateActionEmission::Methods)
                    .with_accessibility(AccessibilityEmission::Methods)
                    .with_consumed_attributes([
                        "type",
                        "value",
                        "placeholder",
                        "disabled",
                        "required",
                        "name",
                        "aria-label",
                        "rows",
                    ]),
            );
        }

        if element.role == UiRole::Image
            && let Some(src) = attribute_value(element, "src")
        {
            return Some(
                GpuiElementSpec::new(format!("gpui::img({})", rust_string(src)))
                    .with_children(ChildEmission::Consumed)
                    .with_consumed_attribute("src"),
            );
        }

        Some(
            GpuiElementSpec::new("gpui::div()")
                .with_children(children)
                .with_actions(ActionEmission::Events)
                .with_title_attribute(TitleAttributeEmission::GpuiTooltip)
                .with_accessibility(AccessibilityEmission::Methods),
        )
    }
}

static GPUI_BASE_LAYER: GpuiBaseLayer = GpuiBaseLayer;

pub(crate) fn gpui_base_layer() -> &'static dyn GpuiTargetLayer {
    &GPUI_BASE_LAYER
}

struct RenderedGpui {
    code: String,
    asset_files: Vec<GeneratedFile>,
    dependencies: Vec<TargetDependency>,
    source_markers: Vec<GeneratedSourceMarker>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GeneratedSourceMarker {
    id: usize,
    original: Span,
    kind: SourceMappingKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MarkedRust {
    code: String,
    source_map: GeneratedSourceMap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MappedComment {
    text: String,
    span: Option<Span>,
    kind: SourceMappingKind,
}

impl MappedComment {
    fn new(text: impl Into<String>, span: Option<Span>, kind: SourceMappingKind) -> Self {
        Self {
            text: text.into(),
            span,
            kind,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct StackedChild<'a> {
    node: &'a RenderNode,
    source_index: usize,
    paint_index: usize,
    z_index: Option<i32>,
    emit_order_hint: bool,
}

#[derive(Debug, Clone, Default)]
struct RenderScope {
    locals: BTreeSet<CompactString>,
    loop_indices: Vec<String>,
    text_transform: TextTransform,
    inside_title_bar_drag_area: bool,
}

impl RenderScope {
    fn with_local(&self, local: impl Into<CompactString>) -> Self {
        let mut next = self.clone();
        next.locals.insert(local.into());
        next
    }

    fn with_loop_local(&self, local: impl Into<CompactString>, index: impl Into<String>) -> Self {
        let mut next = self.with_local(local);
        next.loop_indices.push(index.into());
        next
    }

    fn with_text_transform(&self, text_transform: TextTransform) -> Self {
        let mut next = self.clone();
        next.text_transform = text_transform;
        next
    }

    fn inside_title_bar_drag_area(&self) -> Self {
        let mut next = self.clone();
        next.inside_title_bar_drag_area = true;
        next
    }

    fn is_local(&self, value: &str) -> bool {
        self.locals.contains(value)
    }
}

fn scope_for_element(element: &RenderElement, scope: &RenderScope) -> RenderScope {
    let text_transform = text_transform_for_element(element, scope);
    let mut next = scope.with_text_transform(text_transform);
    if let Some(control_flow) = &element.control_flow
        && control_flow.kind == RenderControlFlowKind::For
        && let Some(binding) = &control_flow.binding
    {
        let local = sanitize_rust_identifier(&binding.name.to_snake_case(), "item");
        next = next.with_local(local);
    }
    next
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum TextTransform {
    #[default]
    None,
    Uppercase,
    Lowercase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ThemeBinding {
    local_name: String,
    field: &'static str,
    kind: ThemeBindingKind,
    span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ThemeBindingKey {
    token: CompactString,
    role: ThemeBindingRole,
}

impl ThemeBindingKey {
    fn new(token: impl Into<CompactString>, role: ThemeBindingRole) -> Self {
        Self {
            token: token.into(),
            role,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThemeBindingKind {
    Color,
    Length,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ThemeBindingRole {
    Text,
    Surface,
    Border,
    Length,
}

impl ThemeBindingRole {
    fn kind(self) -> ThemeBindingKind {
        match self {
            Self::Text | Self::Surface | Self::Border => ThemeBindingKind::Color,
            Self::Length => ThemeBindingKind::Length,
        }
    }
}

const SOURCE_MAP_START: &str = "/*htmlswap-source-map-start:";
const SOURCE_MAP_END: &str = "/*htmlswap-source-map-end:";
const SOURCE_MAP_CLOSE: &str = "*/";
const GPUI_TEXT_INPUT_HELPER: &str = r#"#[derive(Clone)]
struct HtmlswapGpuiTextInputEvent {
    value: String,
}

struct HtmlswapGpuiTextInput {
    focus_handle: gpui::FocusHandle,
    value: String,
    placeholder: String,
    selected_range: std::ops::Range<usize>,
    marked_range: Option<std::ops::Range<usize>>,
    last_bounds: Option<gpui::Bounds<gpui::Pixels>>,
}

impl gpui::EventEmitter<HtmlswapGpuiTextInputEvent> for HtmlswapGpuiTextInput {}

impl HtmlswapGpuiTextInput {
    fn new(
        value: impl Into<String>,
        placeholder: impl Into<String>,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let value = value.into();
        let cursor = value.len();
        Self {
            focus_handle: cx.focus_handle().tab_stop(true),
            value,
            placeholder: placeholder.into(),
            selected_range: cursor..cursor,
            marked_range: None,
            last_bounds: None,
        }
    }

    fn value(&self) -> &str {
        &self.value
    }

    fn set_value(&mut self, value: impl Into<String>, cx: &mut gpui::Context<Self>) {
        self.value = value.into();
        let cursor = self.value.len();
        self.selected_range = cursor..cursor;
        self.marked_range = None;
        cx.notify();
    }

    fn placeholder(&self) -> &str {
        &self.placeholder
    }

    fn set_placeholder(&mut self, placeholder: impl Into<String>, cx: &mut gpui::Context<Self>) {
        let placeholder = placeholder.into();
        if self.placeholder == placeholder {
            return;
        }
        self.placeholder = placeholder;
        cx.notify();
    }

    fn on_mouse_down(
        &mut self,
        _event: &gpui::MouseDownEvent,
        window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) {
        window.focus(&self.focus_handle);
    }

    fn on_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "backspace" => {
                if self.selected_range.is_empty() && self.selected_range.start > 0 {
                    let previous = self.previous_boundary(self.selected_range.start);
                    self.selected_range = previous..self.selected_range.start;
                }
                self.replace_selected_text("", cx);
            }
            "delete" => {
                if self.selected_range.is_empty() && self.selected_range.end < self.value.len() {
                    let next = self.next_boundary(self.selected_range.end);
                    self.selected_range = self.selected_range.end..next;
                }
                self.replace_selected_text("", cx);
            }
            "left" => {
                let previous = self.previous_boundary(self.selected_range.start);
                self.selected_range = previous..previous;
                cx.notify();
            }
            "right" => {
                let next = self.next_boundary(self.selected_range.end);
                self.selected_range = next..next;
                cx.notify();
            }
            "home" => {
                self.selected_range = 0..0;
                cx.notify();
            }
            "end" => {
                let end = self.value.len();
                self.selected_range = end..end;
                cx.notify();
            }
            _ => {}
        }
    }

    fn replace_selected_text(&mut self, replacement: &str, cx: &mut gpui::Context<Self>) {
        let range = self.selected_range.clone();
        self.replace_byte_range(range, replacement, cx);
    }

    fn replace_byte_range(
        &mut self,
        range: std::ops::Range<usize>,
        replacement: &str,
        cx: &mut gpui::Context<Self>,
    ) {
        let start = range.start.min(self.value.len());
        let end = range.end.min(self.value.len()).max(start);
        self.value.replace_range(start..end, replacement);
        let cursor = start + replacement.len();
        self.selected_range = cursor..cursor;
        self.marked_range = None;
        cx.emit(HtmlswapGpuiTextInputEvent {
            value: self.value.clone(),
        });
        cx.notify();
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.value[..offset.min(self.value.len())]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.value[offset.min(self.value.len())..]
            .char_indices()
            .nth(1)
            .map_or(self.value.len(), |(index, _)| offset + index)
    }

    fn byte_to_utf16(&self, offset: usize) -> usize {
        self.value[..offset.min(self.value.len())]
            .encode_utf16()
            .count()
    }

    fn utf16_to_byte(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        for (byte_offset, ch) in self.value.char_indices() {
            if utf16_offset >= offset {
                return byte_offset;
            }
            utf16_offset += ch.len_utf16();
        }
        self.value.len()
    }

    fn range_to_utf16(&self, range: &std::ops::Range<usize>) -> std::ops::Range<usize> {
        self.byte_to_utf16(range.start)..self.byte_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &std::ops::Range<usize>) -> std::ops::Range<usize> {
        self.utf16_to_byte(range.start)..self.utf16_to_byte(range.end)
    }
}

impl gpui::EntityInputHandler for HtmlswapGpuiTextInput {
    fn text_for_range(
        &mut self,
        range_utf16: std::ops::Range<usize>,
        actual_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.value[range].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> Option<gpui::UTF16Selection> {
        Some(gpui::UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut gpui::Window, _cx: &mut gpui::Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<std::ops::Range<usize>>,
        new_text: &str,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        self.replace_byte_range(range, new_text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<std::ops::Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<std::ops::Range<usize>>,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        let start = range.start;
        self.value.replace_range(range.clone(), new_text);
        self.marked_range = (!new_text.is_empty()).then_some(start..start + new_text.len());
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .map(|range| start + range.start..start + range.end)
            .unwrap_or_else(|| start + new_text.len()..start + new_text.len());
        cx.emit(HtmlswapGpuiTextInputEvent {
            value: self.value.clone(),
        });
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _bounds: gpui::Bounds<gpui::Pixels>,
        _window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> Option<gpui::Bounds<gpui::Pixels>> {
        self.last_bounds
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<gpui::Pixels>,
        _window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> Option<usize> {
        Some(self.byte_to_utf16(self.value.len()))
    }
}

impl gpui::Focusable for HtmlswapGpuiTextInput {
    fn focus_handle(&self, _cx: &gpui::App) -> gpui::FocusHandle {
        self.focus_handle.clone()
    }
}

impl gpui::Render for HtmlswapGpuiTextInput {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        gpui::div()
            .track_focus(&self.focus_handle)
            .cursor(gpui::CursorStyle::IBeam)
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_key_down(cx.listener(Self::on_key_down))
            .child(HtmlswapGpuiTextInputElement { input: cx.entity() })
    }
}

struct HtmlswapGpuiTextInputElement {
    input: gpui::Entity<HtmlswapGpuiTextInput>,
}

impl gpui::IntoElement for HtmlswapGpuiTextInputElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for HtmlswapGpuiTextInputElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<gpui::ShapedLine>;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let mut style = gpui::Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: gpui::Bounds<gpui::Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let style = window.text_style();
        let (text, color) = if input.value.is_empty() {
            (input.placeholder.clone(), gpui::hsla(0., 0., 0., 0.45))
        } else {
            (input.value.clone(), style.color)
        };
        let run = gpui::TextRun {
            len: text.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        Some(window.text_system().shape_line(text.into(), font_size, &[run], None))
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            gpui::ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        self.input.update(cx, |input, _cx| {
            input.last_bounds = Some(bounds);
        });
        if let Some(line) = prepaint.take() {
            line.paint(bounds.origin, window.line_height(), window, cx)
                .expect("text input line paint should succeed");
        }
    }
}
"#;

fn active_layers<'a>(
    available_layers: &'a [&'a dyn GpuiTargetLayer],
    routes: &RouteConfig,
) -> Vec<&'a dyn GpuiTargetLayer> {
    let mut selected = BTreeSet::from([GPUI_LAYER_ID]);
    selected.extend(routes.layers.iter().map(|layer| layer.id.as_str()));
    selected.extend(routes.overrides.iter().map(|route| route.layer.as_str()));

    available_layers
        .iter()
        .copied()
        .filter(|layer| selected.contains(layer.id()))
        .collect()
}

struct GpuiCodegen<'a, 'layers, 'cx> {
    options: &'a GpuiAdapterOptions,
    routes: &'a RouteConfig,
    available_layers: &'a [&'a dyn GpuiTargetLayer],
    active_layers: &'layers [&'a dyn GpuiTargetLayer],
    cx: &'cx mut AdapterContext,
    next_id: usize,
    used_layer_ids: BTreeSet<&'static str>,
    state_bindings: BTreeMap<CompactString, RenderStateBinding>,
    state_fields: BTreeMap<CompactString, String>,
    state_field_backends: BTreeMap<CompactString, GpuiStateFieldBackend>,
    used_state_ids: BTreeSet<CompactString>,
    used_dynamic_gpui_text_inputs: bool,
    used_dynamic_component_text_inputs: bool,
    theme_plan: RenderThemePlan,
    theme_bindings: BTreeMap<ThemeBindingKey, ThemeBinding>,
    used_gpui_component_theme: bool,
    used_tooltip_view: bool,
    used_material_symbol_icons: bool,
    used_dynamic_color_helper: bool,
    used_dynamic_length_helper: bool,
    sources: Option<&'a SourceMap>,
    source_markers: Vec<GeneratedSourceMarker>,
    next_source_marker: usize,
}

impl<'a, 'layers, 'cx> GpuiCodegen<'a, 'layers, 'cx> {
    fn new(
        options: &'a GpuiAdapterOptions,
        routes: &'a RouteConfig,
        available_layers: &'a [&'a dyn GpuiTargetLayer],
        active_layers: &'layers [&'a dyn GpuiTargetLayer],
        cx: &'cx mut AdapterContext,
    ) -> Self {
        Self {
            options,
            routes,
            available_layers,
            active_layers,
            cx,
            next_id: 0,
            used_layer_ids: BTreeSet::new(),
            state_bindings: BTreeMap::new(),
            state_fields: BTreeMap::new(),
            state_field_backends: BTreeMap::new(),
            used_state_ids: BTreeSet::new(),
            used_dynamic_gpui_text_inputs: false,
            used_dynamic_component_text_inputs: false,
            theme_plan: RenderThemePlan::default(),
            theme_bindings: BTreeMap::new(),
            used_gpui_component_theme: false,
            used_tooltip_view: false,
            used_material_symbol_icons: false,
            used_dynamic_color_helper: false,
            used_dynamic_length_helper: false,
            sources: None,
            source_markers: Vec::new(),
            next_source_marker: 0,
        }
    }

    fn render(&mut self, fragment: &'a CompiledFragment) -> RenderedGpui {
        self.sources = Some(&fragment.sources);
        self.state_bindings = collect_state_bindings(&fragment.plan);
        self.state_fields = state_fields(&self.state_bindings);
        self.state_field_backends.clear();
        self.theme_plan = fragment.plan.theme.clone();
        self.theme_bindings = theme_bindings_for_plan(&fragment.plan, self.options.theme);
        self.used_gpui_component_theme = !self.theme_bindings.is_empty();
        let body = self.render_component_body(fragment);
        let dependencies = self.dependencies();
        let mut output = String::new();

        if self.options.include_dependency_header {
            self.write_dependency_header(&mut output, &dependencies);
            output.push('\n');
        }

        if self.options.include_imports {
            self.write_imports(&mut output);
            output.push('\n');
        }

        output.push_str(&body);

        RenderedGpui {
            code: output,
            asset_files: self.asset_files(),
            dependencies,
            source_markers: std::mem::take(&mut self.source_markers),
        }
    }

    fn asset_files(&self) -> Vec<GeneratedFile> {
        if !self.used_material_symbol_icons {
            return Vec::new();
        }

        material_symbol_svg_files()
            .into_iter()
            .map(|(name, contents)| {
                GeneratedFile::new(format!("icons/dc/{name}"), contents.to_owned())
            })
            .collect()
    }

    fn render_component_body(&mut self, fragment: &CompiledFragment) -> String {
        self.mark_layer_used(GPUI_LAYER_ID);
        let mut render_body = String::new();

        if self.options.emit_source_comments {
            for annotation in &fragment.plan.annotations {
                self.write_annotation(&mut render_body, annotation, 2);
            }
            self.write_source_metadata_comments(&mut render_body, &fragment.plan, 2);
        }

        self.write_theme_bindings(&mut render_body);
        render_body.push_str(&self.render_plan_expression(&fragment.plan, 2));
        render_body.push('\n');

        let mut output = String::new();
        self.write_component_struct(&mut output, &fragment.plan);
        output.push('\n');
        if self.uses_gpui_text_input() {
            self.write_gpui_text_input_struct(&mut output);
            output.push('\n');
        }
        if self.used_tooltip_view {
            self.write_tooltip_view_struct(&mut output);
            output.push('\n');
        }
        if self.used_material_symbol_icons {
            self.write_material_symbol_icon_struct(&mut output);
            output.push('\n');
        }
        if self.used_dynamic_color_helper {
            self.write_dynamic_color_helper(&mut output);
            output.push('\n');
        }
        if self.used_dynamic_length_helper {
            self.write_dynamic_length_helper(&mut output);
            output.push('\n');
        }
        self.write_layer_helpers(&mut output);

        writeln!(output, "impl Render for {} {{", self.options.component_name)
            .expect("writing to String cannot fail");
        writeln!(
            output,
            "    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {{"
        )
        .expect("writing to String cannot fail");
        output.push_str(&render_body);
        output.push_str("    }\n");
        output.push_str("}\n");
        output
    }

    fn write_component_struct(&mut self, output: &mut String, plan: &RenderPlan) {
        let used_state_ids = self.used_state_ids.iter().cloned().collect::<Vec<_>>();
        let subscription_initializers = self.state_subscription_initializers(plan, &used_state_ids);
        let has_dynamic_text_inputs =
            self.used_dynamic_gpui_text_inputs || self.used_dynamic_component_text_inputs;
        let has_subscriptions = !subscription_initializers.is_empty() || has_dynamic_text_inputs;

        if used_state_ids.is_empty() && !has_subscriptions && !has_dynamic_text_inputs {
            writeln!(output, "pub struct {};", self.options.component_name)
                .expect("writing to String cannot fail");
            output.push('\n');
            writeln!(output, "impl {} {{", self.options.component_name)
                .expect("writing to String cannot fail");
            output.push_str(
                "    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {\n",
            );
            output.push_str("        Self\n");
            output.push_str("    }\n");
            output.push_str("}\n");
            return;
        }

        writeln!(output, "pub struct {} {{", self.options.component_name)
            .expect("writing to String cannot fail");

        for state_id in &used_state_ids {
            let Some(field) = self.state_fields.get(state_id) else {
                continue;
            };
            let Some(binding) = self.state_bindings.get(state_id) else {
                continue;
            };
            let backend = self.state_field_backend(binding);
            writeln!(
                output,
                "    {field}: {},",
                state_field_type(binding, backend)
            )
            .expect("writing to String cannot fail");
        }
        if has_subscriptions {
            output.push_str("    _htmlswap_subscriptions: Vec<gpui::Subscription>,\n");
        }
        if self.used_dynamic_gpui_text_inputs {
            output.push_str(
                "    _htmlswap_gpui_text_inputs: std::collections::BTreeMap<String, gpui::Entity<HtmlswapGpuiTextInput>>,\n",
            );
        }
        if self.used_dynamic_component_text_inputs {
            output.push_str(
                "    _htmlswap_component_text_inputs: std::collections::BTreeMap<String, gpui::Entity<gpui_component::input::InputState>>,\n",
            );
            output.push_str(
                "    _htmlswap_component_text_input_placeholders: std::collections::BTreeMap<String, String>,\n",
            );
        }
        output.push_str("}\n\n");

        writeln!(output, "impl {} {{", self.options.component_name)
            .expect("writing to String cannot fail");
        output.push_str("    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {\n");
        let post_initializers = self.state_post_initializers(&used_state_ids);
        if post_initializers.is_empty() && subscription_initializers.is_empty() {
            output.push_str("        Self {\n");
            self.write_state_field_initializers(output, &used_state_ids, has_subscriptions);
            output.push_str("        }\n");
        } else {
            if has_subscriptions {
                output.push_str("        let mut this = Self {\n");
            } else {
                output.push_str("        let this = Self {\n");
            }
            self.write_state_field_initializers(output, &used_state_ids, has_subscriptions);
            output.push_str("        };\n");
            for initializer in post_initializers {
                output.push_str(&indent_expression(&initializer, 2));
                output.push('\n');
            }
            for initializer in subscription_initializers {
                output.push_str(&indent_expression(&initializer, 2));
                output.push('\n');
            }
            output.push_str("        this\n");
        }
        output.push_str("    }\n");
        output.push_str("}\n");
    }

    fn uses_gpui_text_input(&self) -> bool {
        if self.used_dynamic_gpui_text_inputs {
            return true;
        }

        self.used_state_ids.iter().any(|state_id| {
            self.state_field_backends
                .get(state_id)
                .is_some_and(|backend| *backend == GpuiStateFieldBackend::GpuiTextInput)
        })
    }

    fn write_gpui_text_input_struct(&self, output: &mut String) {
        output.push_str(GPUI_TEXT_INPUT_HELPER);
    }

    fn write_tooltip_view_struct(&self, output: &mut String) {
        output.push_str("struct HtmlswapTooltipView {\n");
        output.push_str("    text: String,\n");
        output.push_str("}\n\n");
        output.push_str("impl Render for HtmlswapTooltipView {\n");
        output.push_str(
            "    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {\n",
        );
        output.push_str("        gpui::div().child(self.text.clone())\n");
        output.push_str("    }\n");
        output.push_str("}\n\n");
        output.push_str("trait HtmlswapDivTooltipExt {\n");
        output.push_str("    fn tooltip<F>(self, _build_tooltip: F) -> Self\n");
        output.push_str("    where\n");
        output.push_str("        F: Fn(&mut Window, &mut gpui::App) -> gpui::AnyView + 'static;\n");
        output.push_str("}\n\n");
        output.push_str("impl HtmlswapDivTooltipExt for gpui::Div {\n");
        output.push_str("    fn tooltip<F>(self, _build_tooltip: F) -> Self\n");
        output.push_str("    where\n");
        output.push_str("        F: Fn(&mut Window, &mut gpui::App) -> gpui::AnyView + 'static,\n");
        output.push_str("    {\n");
        output.push_str("        self\n");
        output.push_str("    }\n");
        output.push_str("}\n");
    }

    fn write_material_symbol_icon_struct(&self, output: &mut String) {
        output.push_str(GPUI_MATERIAL_SYMBOL_ICON_HELPER);
    }

    fn write_dynamic_color_helper(&self, output: &mut String) {
        output.push_str(GPUI_DYNAMIC_COLOR_HELPER);
    }

    fn write_dynamic_length_helper(&self, output: &mut String) {
        output.push_str(GPUI_DYNAMIC_LENGTH_HELPER);
    }

    fn write_layer_helpers(&self, output: &mut String) {
        for layer in self.active_layers {
            if !self.used_layer_ids.contains(layer.id()) {
                continue;
            }

            layer.write_helpers(output);
        }
    }

    fn write_state_field_initializers(
        &mut self,
        output: &mut String,
        used_state_ids: &[CompactString],
        include_subscriptions: bool,
    ) {
        for state_id in used_state_ids {
            let Some(field) = self.state_fields.get(state_id).cloned() else {
                continue;
            };
            let Some(binding) = self.state_bindings.get(state_id).cloned() else {
                continue;
            };
            if state_uses_entity(&binding) {
                writeln!(output, "            {field}: cx.new(|cx| {{")
                    .expect("writing to String cannot fail");
                let backend = self.state_field_backend(&binding);
                let initializer = self.mark_source(
                    state_initializer(&binding, backend),
                    binding.span,
                    SourceMappingKind::State,
                );
                output.push_str(&indent_expression(&initializer, 4));
                output.push('\n');
                output.push_str("            }),\n");
            } else {
                let backend = self.state_field_backend(&binding);
                let initializer = self.mark_source(
                    state_initializer(&binding, backend),
                    binding.span,
                    SourceMappingKind::State,
                );
                writeln!(output, "            {field}: {},", initializer)
                    .expect("writing to String cannot fail");
            }
        }
        if include_subscriptions {
            output.push_str("            _htmlswap_subscriptions: Vec::new(),\n");
        }
        if self.used_dynamic_gpui_text_inputs {
            output.push_str(
                "            _htmlswap_gpui_text_inputs: std::collections::BTreeMap::new(),\n",
            );
        }
        if self.used_dynamic_component_text_inputs {
            output.push_str(
                "            _htmlswap_component_text_inputs: std::collections::BTreeMap::new(),\n",
            );
            output.push_str(
                "            _htmlswap_component_text_input_placeholders: std::collections::BTreeMap::new(),\n",
            );
        }
    }

    fn state_post_initializers(&mut self, used_state_ids: &[CompactString]) -> Vec<String> {
        let mut initializers = Vec::new();
        for state_id in used_state_ids {
            let Some(field) = self.state_fields.get(state_id).cloned() else {
                continue;
            };
            let Some(binding) = self.state_bindings.get(state_id).cloned() else {
                continue;
            };
            let RenderStateKind::TextInput(input) = &binding.kind else {
                continue;
            };
            if let Some(template) = &input.initial_template {
                let value =
                    template_text_expression_with_root(template, &RenderScope::default(), "this");
                let variable = format!("htmlswap_initial_{field}");
                let statement = match self.state_field_backend(&binding) {
                    GpuiStateFieldBackend::ComponentTextInput => format!(
                        "let {variable} = {value};\nthis.{field}.update(cx, |state, cx| {{\n    state.set_value({variable}.clone(), window, cx);\n}});"
                    ),
                    GpuiStateFieldBackend::GpuiTextInput => format!(
                        "let {variable} = {value};\nthis.{field}.update(cx, |state, cx| {{\n    state.set_value({variable}.clone(), cx);\n}});"
                    ),
                };
                initializers.push(self.mark_source(
                    statement,
                    binding.span,
                    SourceMappingKind::State,
                ));
            }

            if let Some(template) = &input.placeholder_template {
                let value =
                    template_text_expression_with_root(template, &RenderScope::default(), "this");
                let variable = format!("htmlswap_placeholder_{field}");
                let statement = match self.state_field_backend(&binding) {
                    GpuiStateFieldBackend::ComponentTextInput => format!(
                        "let {variable} = {value};\nthis.{field}.update(cx, |state, cx| {{\n    state.set_placeholder({variable}.clone().into(), window, cx);\n}});"
                    ),
                    GpuiStateFieldBackend::GpuiTextInput => format!(
                        "let {variable} = {value};\nthis.{field}.update(cx, |state, cx| {{\n    state.set_placeholder({variable}.clone(), cx);\n}});"
                    ),
                };
                initializers.push(self.mark_source(
                    statement,
                    binding.span,
                    SourceMappingKind::State,
                ));
            }
        }

        initializers
    }

    fn state_field_backend(&self, binding: &RenderStateBinding) -> GpuiStateFieldBackend {
        self.state_field_backends
            .get(&binding.id)
            .copied()
            .unwrap_or(GpuiStateFieldBackend::GpuiTextInput)
    }

    fn state_subscription_initializers(
        &mut self,
        plan: &RenderPlan,
        used_state_ids: &[CompactString],
    ) -> Vec<String> {
        if used_state_ids.is_empty() {
            return Vec::new();
        }

        let used_state_ids = used_state_ids.iter().collect::<BTreeSet<_>>();
        let mut initializers = Vec::new();
        let scope = RenderScope::default();
        for node in &plan.nodes {
            self.collect_state_subscription_initializers(
                node,
                &scope,
                &used_state_ids,
                &mut initializers,
            );
        }
        initializers
    }

    fn collect_state_subscription_initializers(
        &mut self,
        node: &RenderNode,
        scope: &RenderScope,
        used_state_ids: &BTreeSet<&CompactString>,
        initializers: &mut Vec<String>,
    ) {
        let RenderNode::Element(element) = node else {
            return;
        };

        if let Some(initializer) =
            self.text_input_subscription_initializer(element, scope, used_state_ids)
        {
            initializers.push(initializer);
        }

        let next_scope = scope_for_element(element, scope);
        for child in &element.children {
            self.collect_state_subscription_initializers(
                child,
                &next_scope,
                used_state_ids,
                initializers,
            );
        }
        for pseudo in &element.pseudo_elements {
            for child in &pseudo.children {
                self.collect_state_subscription_initializers(
                    child,
                    &next_scope,
                    used_state_ids,
                    initializers,
                );
            }
        }
    }

    fn text_input_subscription_initializer(
        &mut self,
        element: &RenderElement,
        scope: &RenderScope,
        used_state_ids: &BTreeSet<&CompactString>,
    ) -> Option<String> {
        let state = element.state.as_ref()?;
        if !used_state_ids.contains(&state.id) {
            return None;
        }
        if !matches!(&state.kind, RenderStateKind::TextInput(_)) {
            return None;
        }

        let field = self.state_fields.get(&state.id)?.clone();
        let mut statements = Vec::new();
        for action in &element.actions {
            if !self.action_uses_text_input_subscription(element, action, scope) {
                continue;
            }
            if let Some(call) = input_subscription_action_call(action, scope, "&_htmlswap_value") {
                statements.push(self.mark_source(call, action.span, SourceMappingKind::Action));
            }
        }

        if statements.is_empty() {
            return None;
        }

        let body = statements.join("\n");
        let initializer = match self.state_field_backend(state) {
            GpuiStateFieldBackend::ComponentTextInput => format!(
                "let htmlswap_subscription = cx.subscribe_in(&this.{field}, window, |this: &mut Self, input, event: &gpui_component::input::InputEvent, window, cx| {{\n    if matches!(event, gpui_component::input::InputEvent::Change) {{\n        let _htmlswap_value = input.read(cx).value().to_string();\n{}\n    }}\n}});\nthis._htmlswap_subscriptions.push(htmlswap_subscription);",
                indent_expression(&body, 2)
            ),
            GpuiStateFieldBackend::GpuiTextInput => format!(
                "let htmlswap_subscription = cx.subscribe_in(&this.{field}, window, |this: &mut Self, input, event: &HtmlswapGpuiTextInputEvent, window, cx| {{\n    let _ = event;\n    let _htmlswap_value = input.read(cx).value().to_owned();\n{}\n}});\nthis._htmlswap_subscriptions.push(htmlswap_subscription);",
                indent_expression(&body, 1)
            ),
        };
        Some(self.mark_source(initializer, state.span, SourceMappingKind::State))
    }

    fn dependencies(&mut self) -> Vec<TargetDependency> {
        let mut dependencies = DependencySet::new();
        for layer in self.active_layers {
            if !self.used_layer_ids.contains(layer.id()) {
                continue;
            }

            for dependency in layer.dependencies() {
                dependencies.insert(dependency, self.cx);
            }
        }
        if self.used_gpui_component_theme {
            dependencies.insert(
                TargetDependency::crates_io("gpui-component", GPUI_COMPONENT_THEME_CRATE_VERSION),
                self.cx,
            );
        }
        dependencies.into_vec()
    }

    fn write_dependency_header(&self, output: &mut String, dependencies: &[TargetDependency]) {
        output.push_str("// Generated by htmlswap.\n");
        output.push_str("// Required target dependencies:\n");
        for dependency in dependencies {
            writeln!(
                output,
                "// {} = \"{}\"",
                dependency.package, dependency.version_req
            )
            .expect("writing to String cannot fail");
        }
        if self.used_material_symbol_icons {
            output.push_str("// Asset requirement: Material Symbol SVGs at `icons/dc/*.svg`.\n");
        }
    }

    fn write_imports(&self, output: &mut String) {
        for layer in self.active_layers {
            if !self.used_layer_ids.contains(layer.id()) {
                continue;
            }

            layer.write_imports(output);
        }
        if self.used_gpui_component_theme {
            output.push_str("use gpui_component::ActiveTheme as _;\n");
        }
    }

    fn write_theme_bindings(&mut self, output: &mut String) {
        if self.theme_bindings.is_empty() {
            return;
        }

        for binding in self.theme_bindings.values().cloned().collect::<Vec<_>>() {
            let line = self.mark_source(
                format!(
                    "let {} = _cx.theme().{};",
                    binding.local_name, binding.field
                ),
                binding.span,
                SourceMappingKind::Style,
            );
            writeln!(output, "{}{line}", indent(2)).expect("writing to String cannot fail");
        }
        output.push('\n');
    }

    fn write_annotation(
        &mut self,
        output: &mut String,
        annotation: &RenderAnnotation,
        depth: usize,
    ) {
        let indent = indent(depth);
        let label = annotation.kind.name();
        let value = annotation.value.trim();

        if value.is_empty() {
            let line = self.mark_line_comment_source(
                &indent,
                label,
                annotation.span,
                SourceMappingKind::Comment,
            );
            writeln!(output, "{line}").expect("writing to String cannot fail");
            return;
        }

        for line in value.lines() {
            let line = self.mark_line_comment_source(
                &indent,
                format!("{label}: {}", line.trim()),
                annotation.span,
                SourceMappingKind::Comment,
            );
            writeln!(output, "{line}").expect("writing to String cannot fail");
        }
    }

    fn write_source_metadata_comments(
        &mut self,
        output: &mut String,
        plan: &RenderPlan,
        depth: usize,
    ) {
        let line_indent = indent(depth);
        for element in &plan.head {
            let line = self.mark_line_comment_source(
                &line_indent,
                format!("source head: {}", one_line(&element.html)),
                element.span,
                SourceMappingKind::Raw,
            );
            writeln!(output, "{line}").expect("writing to String cannot fail");
        }

        for logic in &plan.source_logic {
            let mut parts = vec![format!("dialect={}", logic.dialect)];
            if let Some(script_type) = &logic.script_type {
                parts.push(format!("type={script_type}"));
            }
            if let Some(data_props) = &logic.data_props {
                parts.push(format!("data-props={}", one_line(data_props)));
            }
            parts.push("body omitted".to_owned());
            let line = self.mark_line_comment_source(
                &line_indent,
                format!("source logic: {}", parts.join(", ")),
                logic.span,
                SourceMappingKind::Raw,
            );
            writeln!(output, "{line}").expect("writing to String cannot fail");
        }
    }

    fn render_plan_expression(&mut self, plan: &RenderPlan, depth: usize) -> String {
        let scope = RenderScope::default();
        if !plan.root.is_empty() {
            return self.render_root_expression(plan, depth, &scope);
        }

        match plan.nodes.as_slice() {
            [] => {
                self.mark_layer_used(GPUI_LAYER_ID);
                format!("{}gpui::div()", indent(depth))
            }
            [node] => self.render_node(node, depth, &scope),
            nodes => {
                self.mark_layer_used(GPUI_LAYER_ID);
                let mut output = format!("{}gpui::div()", indent(depth));
                for node in stacked_child_nodes(nodes) {
                    self.push_ordered_child(&mut output, node, depth, &scope);
                }
                output
            }
        }
    }

    fn render_root_expression(
        &mut self,
        plan: &RenderPlan,
        depth: usize,
        scope: &RenderScope,
    ) -> String {
        self.mark_layer_used(GPUI_LAYER_ID);
        let mut comments = Vec::new();
        let mut expression =
            self.mark_source("gpui::div()", plan.root.span, SourceMappingKind::Element);
        self.push_root_style_methods(&mut expression, plan, &mut comments);
        push_method(&mut expression, 0, ".size_full()");

        for node in stacked_child_nodes(&plan.nodes) {
            self.push_ordered_child(&mut expression, node, depth, scope);
        }

        self.with_local_comments(expression, depth, &comments)
    }

    fn render_node(&mut self, node: &RenderNode, depth: usize, scope: &RenderScope) -> String {
        match node {
            RenderNode::Element(element) => {
                if element.control_flow.is_some() {
                    self.render_control_flow_expression(element, depth, scope)
                } else {
                    self.render_element(element, depth, scope)
                }
            }
            RenderNode::Text(text) => {
                let expression = render_text_expression(text, scope);
                let expression = self.mark_source(expression, text.span, SourceMappingKind::Text);
                format!("{}{expression}", indent(depth))
            }
            RenderNode::Raw(raw) => self.render_raw(raw, depth),
        }
    }

    fn render_raw(&mut self, raw: &RenderRaw, depth: usize) -> String {
        let comments = vec![MappedComment::new(
            format!("htmlswap raw HTML omitted: {}", one_line(raw.html.trim())),
            raw.span,
            SourceMappingKind::Raw,
        )];
        self.mark_layer_used(GPUI_LAYER_ID);
        let expression = self.mark_source("gpui::div()", raw.span, SourceMappingKind::Raw);
        self.with_local_comments(expression, depth, &comments)
    }

    fn render_element(
        &mut self,
        element: &RenderElement,
        depth: usize,
        scope: &RenderScope,
    ) -> String {
        if let Some(backend) = self.dynamic_source_text_input_backend(element, scope) {
            return self.render_dynamic_source_text_input(element, depth, scope, backend);
        }

        let spec = self.element_spec(element);

        if spec.children == ChildEmission::TextInputPlaceholder {
            self.warn(
                "GPUI text inputs require state planning; emitted placeholder div",
                element.span,
            );
        }

        if element.role == UiRole::Image && attribute_value(element, "src").is_none() {
            self.warn(
                "GPUI images require a `src`; emitted placeholder div",
                element.span,
            );
        }

        let mut comments = self.element_comments(element, &spec);
        let mut expression = self.mark_source(
            spec.expression.clone(),
            element.span,
            SourceMappingKind::Element,
        );

        if element.source_tag.eq_ignore_ascii_case("x-dc") {
            push_method(&mut expression, 0, ".size_full()");
        }
        self.push_attribute_methods(&mut expression, element, &mut comments, &spec, scope);
        let has_runtime_id = element_id(element).is_some()
            || self.push_debug_layout_id_if_needed(&mut expression, element, &spec, scope);
        self.push_generated_stateful_id_if_needed(
            &mut expression,
            element,
            &spec,
            has_runtime_id,
            scope,
        );
        self.push_style_methods(&mut expression, element, &mut comments, &spec, scope);
        self.push_accessibility_methods(&mut expression, element, &mut comments, &spec);
        self.push_title_bar_interaction_methods(&mut expression, element, &spec, scope);
        self.push_action_methods(&mut expression, element, &mut comments, &spec, scope);
        self.push_pseudo_element_comments(element, &mut comments, &spec);
        let child_scope = scope.with_text_transform(text_transform_for_element(element, scope));

        match spec.children {
            ChildEmission::Children => {
                self.push_pseudo_children(&mut expression, element, "before", 0, &child_scope);
                for child in stacked_child_nodes(&element.children) {
                    self.push_ordered_child(&mut expression, child, 0, &child_scope);
                }
                self.push_pseudo_children(&mut expression, element, "after", 0, &child_scope);
            }
            ChildEmission::TextAsLabel => {
                if let Some(label) = dynamic_text_only_children(element, &child_scope) {
                    let method = self.mark_source(
                        format!(".label({label})"),
                        text_children_span(element).or(element.span),
                        SourceMappingKind::Text,
                    );
                    push_method(&mut expression, 0, &method);
                } else if let Some(label) = text_only_children(element) {
                    let label = transform_static_text(&label, child_scope.text_transform);
                    let method = self.mark_source(
                        format!(".label({})", rust_string(&label)),
                        text_children_span(element).or(element.span),
                        SourceMappingKind::Text,
                    );
                    push_method(&mut expression, 0, &method);
                } else {
                    for child in stacked_child_nodes(&element.children) {
                        self.push_ordered_child(&mut expression, child, 0, scope);
                    }
                }
            }
            ChildEmission::MaterialIcon => {
                self.used_material_symbol_icons = true;
                let icon = material_symbol_text_expression(element, &child_scope);
                let method = self.mark_source(
                    format!(".child(htmlswap_material_symbol_icon({icon}))"),
                    text_children_span(element).or(element.span),
                    SourceMappingKind::Text,
                );
                push_method(&mut expression, 0, &method);
            }
            ChildEmission::TextInputPlaceholder => {
                let placeholder = attribute_value(element, "placeholder").unwrap_or_default();
                let method = self.mark_source(
                    format!(".child({})", rust_string(placeholder)),
                    attribute_span(element, "placeholder").or(element.span),
                    SourceMappingKind::Attribute,
                );
                push_method(&mut expression, 0, &method);
            }
            ChildEmission::TitleBarChildren => {
                self.push_title_bar_children(&mut expression, element, &child_scope, &mut comments);
            }
            ChildEmission::FormFields => {
                self.push_form_fields(&mut expression, element, scope);
            }
            ChildEmission::FieldsetChildren => {
                self.push_pseudo_children(&mut expression, element, "before", 0, &child_scope);
                for child in stacked_child_nodes(&element.children) {
                    if is_legend_child(child.node) {
                        continue;
                    }

                    self.push_ordered_child(&mut expression, child, 0, &child_scope);
                }
                self.push_pseudo_children(&mut expression, element, "after", 0, &child_scope);
            }
            ChildEmission::Consumed => {}
        }

        expression = self.wrap_title_tooltip_if_needed(expression, element, &spec, scope);
        self.with_local_comments(expression, depth, &comments)
    }

    fn dynamic_source_text_input_backend(
        &self,
        element: &RenderElement,
        scope: &RenderScope,
    ) -> Option<GpuiStateFieldBackend> {
        if scope.loop_indices.is_empty() || element.role != UiRole::TextInput {
            return None;
        }

        let state = element.state.as_ref()?;
        if state.owner != RenderStateOwner::Source
            || !matches!(&state.kind, RenderStateKind::TextInput(_))
        {
            return None;
        }

        if self.layer_for(element).id() == GPUI_LAYER_ID {
            Some(GpuiStateFieldBackend::GpuiTextInput)
        } else {
            Some(GpuiStateFieldBackend::ComponentTextInput)
        }
    }

    fn render_dynamic_source_text_input(
        &mut self,
        element: &RenderElement,
        depth: usize,
        scope: &RenderScope,
        backend: GpuiStateFieldBackend,
    ) -> String {
        self.mark_layer_used(GPUI_LAYER_ID);
        if backend == GpuiStateFieldBackend::ComponentTextInput {
            let layer_id = self.layer_for(element).id();
            self.mark_layer_used(layer_id);
            self.used_dynamic_component_text_inputs = true;
        } else {
            self.used_dynamic_gpui_text_inputs = true;
        }

        let mut spec = match backend {
            GpuiStateFieldBackend::ComponentTextInput => GpuiElementSpec::new(
                self.dynamic_source_text_input_expression(element, scope, backend),
            )
            .with_id_attribute(IdAttributeEmission::Method)
            .with_title_attribute(TitleAttributeEmission::GpuiTooltipWrapper)
            .with_children(ChildEmission::Consumed)
            .with_style_variants(StyleEmission::Comment)
            .with_actions(ActionEmission::Events)
            .with_state_actions(StateActionEmission::Methods)
            .with_accessibility(AccessibilityEmission::Methods)
            .with_consumed_attributes([
                "type",
                "value",
                "placeholder",
                "disabled",
                "required",
                "name",
                "aria-label",
                "rows",
            ]),
            GpuiStateFieldBackend::GpuiTextInput => GpuiElementSpec::new(
                self.dynamic_source_text_input_expression(element, scope, backend),
            )
            .with_id_attribute(IdAttributeEmission::Method)
            .with_title_attribute(TitleAttributeEmission::GpuiTooltip)
            .with_children(ChildEmission::Consumed)
            .with_actions(ActionEmission::Events)
            .with_state_actions(StateActionEmission::Methods)
            .with_accessibility(AccessibilityEmission::Methods)
            .with_consumed_attributes([
                "type",
                "value",
                "placeholder",
                "disabled",
                "required",
                "name",
                "aria-label",
                "rows",
            ]),
        };
        spec = spec.with_unsupported_action_events(["input", "change"]);

        let mut comments = self.element_comments(element, &spec);
        let mut expression = self.mark_source(
            spec.expression.clone(),
            element.span,
            SourceMappingKind::Element,
        );
        self.push_attribute_methods(&mut expression, element, &mut comments, &spec, scope);
        let has_runtime_id = element_id(element).is_some()
            || self.push_debug_layout_id_if_needed(&mut expression, element, &spec, scope);
        self.push_generated_stateful_id_if_needed(
            &mut expression,
            element,
            &spec,
            has_runtime_id,
            scope,
        );
        self.push_style_methods(&mut expression, element, &mut comments, &spec, scope);
        self.push_accessibility_methods(&mut expression, element, &mut comments, &spec);
        self.push_action_methods(&mut expression, element, &mut comments, &spec, scope);
        self.push_pseudo_element_comments(element, &mut comments, &spec);
        expression = self.wrap_title_tooltip_if_needed(expression, element, &spec, scope);
        self.with_local_comments(expression, depth, &comments)
    }

    fn dynamic_source_text_input_expression(
        &mut self,
        element: &RenderElement,
        scope: &RenderScope,
        backend: GpuiStateFieldBackend,
    ) -> String {
        let Some(state) = element.state.as_ref() else {
            return "gpui::div()".to_owned();
        };
        let RenderStateKind::TextInput(input) = &state.kind else {
            return "gpui::div()".to_owned();
        };

        let key = dynamic_state_key_expression(&state.id, scope);
        let value = text_input_render_value_expression(input, scope);
        let placeholder = text_input_placeholder_expression(input, scope);
        let subscription = self.dynamic_text_input_subscription(element, scope, backend);

        match backend {
            GpuiStateFieldBackend::GpuiTextInput => format!(
                "{{\n    let htmlswap_input_key = {key};\n    let htmlswap_input_value = ({value}).to_string();\n    let htmlswap_input_placeholder = ({placeholder}).to_string();\n    let htmlswap_input = if let Some(htmlswap_input) = self._htmlswap_gpui_text_inputs.get(&htmlswap_input_key) {{\n        htmlswap_input.clone()\n    }} else {{\n        let htmlswap_input = _cx.new(|cx| HtmlswapGpuiTextInput::new(htmlswap_input_value.clone(), htmlswap_input_placeholder.clone(), cx));\n{subscription}\n        self._htmlswap_gpui_text_inputs.insert(htmlswap_input_key.clone(), htmlswap_input.clone());\n        htmlswap_input\n    }};\n    if htmlswap_input.read(_cx).value() != htmlswap_input_value {{\n        htmlswap_input.update(_cx, |state, cx| {{\n            state.set_value(htmlswap_input_value.clone(), cx);\n        }});\n    }}\n    if htmlswap_input.read(_cx).placeholder() != htmlswap_input_placeholder {{\n        htmlswap_input.update(_cx, |state, cx| {{\n            state.set_placeholder(htmlswap_input_placeholder.clone(), cx);\n        }});\n    }}\n    gpui::div().child(htmlswap_input.clone())\n}}"
            ),
            GpuiStateFieldBackend::ComponentTextInput => format!(
                "{{\n    let htmlswap_input_key = {key};\n    let htmlswap_input_value = ({value}).to_string();\n    let htmlswap_input_placeholder = ({placeholder}).to_string();\n    let htmlswap_input = if let Some(htmlswap_input) = self._htmlswap_component_text_inputs.get(&htmlswap_input_key) {{\n        htmlswap_input.clone()\n    }} else {{\n        let htmlswap_input = _cx.new(|cx| {{\n            gpui_component::input::InputState::new(_window, cx).default_value(htmlswap_input_value.clone()).placeholder(htmlswap_input_placeholder.clone())\n        }});\n{subscription}\n        self._htmlswap_component_text_inputs.insert(htmlswap_input_key.clone(), htmlswap_input.clone());\n        self._htmlswap_component_text_input_placeholders.insert(htmlswap_input_key.clone(), htmlswap_input_placeholder.clone());\n        htmlswap_input\n    }};\n    if htmlswap_input.read(_cx).value() != htmlswap_input_value {{\n        htmlswap_input.update(_cx, |state, cx| {{\n            state.set_value(htmlswap_input_value.clone(), _window, cx);\n        }});\n    }}\n    if self._htmlswap_component_text_input_placeholders.get(&htmlswap_input_key).is_none_or(|placeholder| placeholder != &htmlswap_input_placeholder) {{\n        htmlswap_input.update(_cx, |state, cx| {{\n            state.set_placeholder(htmlswap_input_placeholder.clone(), _window, cx);\n        }});\n        self._htmlswap_component_text_input_placeholders.insert(htmlswap_input_key.clone(), htmlswap_input_placeholder.clone());\n    }}\n    {}\n}}",
                component_text_input_host_expression(
                    "gpui_component::input::Input::new(&htmlswap_input)"
                )
            ),
        }
    }

    fn dynamic_text_input_subscription(
        &mut self,
        element: &RenderElement,
        scope: &RenderScope,
        backend: GpuiStateFieldBackend,
    ) -> String {
        let mut statements = Vec::new();
        for action in &element.actions {
            if !self.action_uses_text_input_subscription(element, action, scope) {
                continue;
            }
            if let Some(call) = input_subscription_action_call(action, scope, "&_htmlswap_value") {
                statements.push(self.mark_source(call, action.span, SourceMappingKind::Action));
            }
        }
        if statements.is_empty() {
            return String::new();
        }

        let local_captures = element_action_local_captures(element, scope);
        let captures = local_capture_binding_statements(&local_captures);
        let body = rewrite_local_capture_references(&statements.join("\n"), &local_captures);
        match backend {
            GpuiStateFieldBackend::GpuiTextInput => format!(
                "        let htmlswap_subscription = _cx.subscribe_in(&htmlswap_input, _window, {{\n{}\n            move |this: &mut Self, input, event: &HtmlswapGpuiTextInputEvent, window, cx| {{\n                let _ = event;\n                let _htmlswap_value = input.read(cx).value().to_owned();\n{}\n            }}\n        }});\n        self._htmlswap_subscriptions.push(htmlswap_subscription);",
                indent_expression(&captures, 3),
                indent_expression(&body, 4)
            ),
            GpuiStateFieldBackend::ComponentTextInput => format!(
                "        let htmlswap_subscription = _cx.subscribe_in(&htmlswap_input, _window, {{\n{}\n            move |this: &mut Self, input, event: &gpui_component::input::InputEvent, window, cx| {{\n                if matches!(event, gpui_component::input::InputEvent::Change) {{\n                    let _htmlswap_value = input.read(cx).value().to_string();\n{}\n                }}\n            }}\n        }});\n        self._htmlswap_subscriptions.push(htmlswap_subscription);",
                indent_expression(&captures, 3),
                indent_expression(&body, 5)
            ),
        }
    }

    fn element_spec(&mut self, element: &RenderElement) -> GpuiElementSpec {
        let layer = self.layer_for(element);
        let selected_layer = layer.id();

        let spec = {
            let mut context = GpuiElementContext {
                next_id: &mut self.next_id,
                emit_debug_layout_ids: self.options.emit_debug_layout_ids,
                sources: self.sources,
                state_fields: &self.state_fields,
                state_field_backends: &mut self.state_field_backends,
                used_state_ids: &mut self.used_state_ids,
                cx: &mut *self.cx,
            };
            layer.element_spec(element, &mut context)
        };

        if let Some(spec) = spec {
            self.mark_layer_used(layer.id());
            return spec;
        }

        if selected_layer != GPUI_LAYER_ID {
            self.warn_layer_fallback(element, selected_layer);
        }
        self.mark_layer_used(GPUI_LAYER_ID);
        let mut context = GpuiElementContext {
            next_id: &mut self.next_id,
            emit_debug_layout_ids: self.options.emit_debug_layout_ids,
            sources: self.sources,
            state_fields: &self.state_fields,
            state_field_backends: &mut self.state_field_backends,
            used_state_ids: &mut self.used_state_ids,
            cx: &mut *self.cx,
        };
        gpui_base_layer()
            .element_spec(element, &mut context)
            .expect("base GPUI layer must emit every element")
    }

    fn layer_for(&self, element: &RenderElement) -> &'a dyn GpuiTargetLayer {
        let targets = route_targets(element);
        let Some(layer_id) = self.routes.resolve_first_ref(&targets) else {
            return gpui_base_layer();
        };

        self.available_layers
            .iter()
            .copied()
            .find(|layer| layer.id() == layer_id.as_str())
            .unwrap_or(gpui_base_layer())
    }

    fn warn_layer_fallback(&mut self, element: &RenderElement, layer_id: &str) {
        if let Some(component) = element
            .source_intent
            .as_ref()
            .and_then(|hints| hints.component.as_ref())
        {
            self.warn(
                format!(
                    "route selected layer `{layer_id}` for component intent `{component}`, but the layer did not emit it; emitted base GPUI"
                ),
                attribute_span(element, "data-htmlswap-component").or(element.span),
            );
            return;
        }

        self.warn(
            format!("route selected layer `{layer_id}`, but the layer did not emit this element; emitted base GPUI"),
            element.span,
        );
    }

    fn push_attribute_methods(
        &mut self,
        expression: &mut String,
        element: &RenderElement,
        comments: &mut Vec<MappedComment>,
        spec: &GpuiElementSpec,
        scope: &RenderScope,
    ) {
        if let Some(id) = element_id(element) {
            match spec.id_attribute {
                IdAttributeEmission::Method => {
                    let method = self.mark_source(
                        format!(".id({})", rust_string(id)),
                        attribute_span(element, "id").or(element.span),
                        SourceMappingKind::Attribute,
                    );
                    push_method(expression, 0, &method);
                }
                IdAttributeEmission::Consumed => {}
                IdAttributeEmission::Warn => {
                    self.warn(
                        "GPUI element does not support HTML `id` emission",
                        element.span,
                    );
                    comments.push(MappedComment::new(
                        format!("unmapped id: {id}"),
                        attribute_span(element, "id").or(element.span),
                        SourceMappingKind::Attribute,
                    ));
                }
            }
        }

        if let Some(title) = attribute_value(element, "title") {
            match spec.title_attribute {
                TitleAttributeEmission::Warn => {
                    comments.push(MappedComment::new(
                        format!("unmapped tooltip: {title}"),
                        attribute_span(element, "title").or(element.span),
                        SourceMappingKind::Attribute,
                    ));
                }
                TitleAttributeEmission::ComponentTooltip => {
                    let value = element
                        .attributes
                        .iter()
                        .find(|attribute| attribute.name == "title")
                        .and_then(|attribute| attribute.template.as_ref())
                        .map_or_else(
                            || rust_string(title),
                            |template| template_text_expression(template, scope),
                        );
                    let method = self.mark_source(
                        format!(".tooltip({value})"),
                        attribute_span(element, "title").or(element.span),
                        SourceMappingKind::Attribute,
                    );
                    push_method(expression, 0, &method);
                }
                TitleAttributeEmission::GpuiTooltip => {
                    let value = element
                        .attributes
                        .iter()
                        .find(|attribute| attribute.name == "title")
                        .and_then(|attribute| attribute.template.as_ref())
                        .map_or_else(
                            || rust_string(title),
                            |template| template_text_expression(template, scope),
                        );
                    let method = self.mark_source(
                        gpui_tooltip_method(&value),
                        attribute_span(element, "title").or(element.span),
                        SourceMappingKind::Attribute,
                    );
                    self.used_tooltip_view = true;
                    push_method(expression, 0, &method);
                }
                TitleAttributeEmission::GpuiTooltipWrapper => {}
            }
        }
    }

    fn wrap_title_tooltip_if_needed(
        &mut self,
        expression: String,
        element: &RenderElement,
        spec: &GpuiElementSpec,
        scope: &RenderScope,
    ) -> String {
        if spec.title_attribute != TitleAttributeEmission::GpuiTooltipWrapper {
            return expression;
        }

        let Some(title) = attribute_value(element, "title") else {
            return expression;
        };
        let value = element
            .attributes
            .iter()
            .find(|attribute| attribute.name == "title")
            .and_then(|attribute| attribute.template.as_ref())
            .map_or_else(
                || rust_string(title),
                |template| template_text_expression(template, scope),
            );
        let method = self.mark_source(
            gpui_tooltip_method(&value),
            attribute_span(element, "title").or(element.span),
            SourceMappingKind::Attribute,
        );
        self.used_tooltip_view = true;

        let mut wrapper = "gpui::div()".to_owned();
        push_method(&mut wrapper, 0, &method);
        wrapper.push_str("\n    .child(\n");
        wrapper.push_str(&indent_expression(&expression, 2));
        wrapper.push_str("\n    )");
        wrapper
    }

    fn push_style_methods(
        &mut self,
        expression: &mut String,
        element: &RenderElement,
        comments: &mut Vec<MappedComment>,
        spec: &GpuiElementSpec,
        scope: &RenderScope,
    ) {
        if spec.styles == StyleEmission::Comment {
            for style in &element.styles {
                if style_is_consumed_by_gpui_planner(element, style) {
                    comments.push(preserved_style_comment("planner CSS", style, element.span));
                    continue;
                }
                comments.push(MappedComment::new(
                    format!("unmapped CSS: {}: {}", style.property, style.value),
                    style.span.or(element.span),
                    SourceMappingKind::Style,
                ));
            }
            self.push_dynamic_style_comments(element, comments);
            return;
        }

        for style in &element.styles {
            if spec.children == ChildEmission::MaterialIcon
                && style_is_consumed_by_material_icon(style)
            {
                continue;
            }
            if style_is_consumed_by_gpui_planner(element, style) {
                comments.push(preserved_style_comment("planner CSS", style, element.span));
                continue;
            }
            if style.property == StyleProperty::Transform
                && let Some(method) = parse_element_transform_offset(element, style)
            {
                comments.push(preserved_style_comment(
                    "preserved CSS",
                    style,
                    element.span,
                ));
                if method.is_empty() {
                    continue;
                }
                let method = self.mark_source(method, style.span, SourceMappingKind::Style);
                push_method(expression, 0, &method);
                continue;
            }
            if style.property == StyleProperty::BoxShadow
                && let Some(method) = parse_element_inset_shadow_border(element, style)
            {
                comments.push(preserved_style_comment(
                    "preserved CSS",
                    style,
                    element.span,
                ));
                if method.is_empty() {
                    continue;
                }
                let method = self.mark_source(method, style.span, SourceMappingKind::Style);
                push_method(expression, 0, &method);
                continue;
            }
            if let Some(method) = self.gpui_style_method(style) {
                if style_should_preserve_original_css(style) {
                    comments.push(preserved_style_comment(
                        "preserved CSS",
                        style,
                        element.span,
                    ));
                }
                if method.is_empty() {
                    continue;
                }
                let method = self.mark_source(method, style.span, SourceMappingKind::Style);
                push_method(expression, 0, &method);
            } else {
                comments.push(MappedComment::new(
                    format!("unmapped CSS: {}: {}", style.property, style.value),
                    style.span.or(element.span),
                    SourceMappingKind::Style,
                ));
            }
        }

        for variant in &element.style_variants {
            if spec.style_variants == StyleEmission::Comment {
                let reason = style_variant_preservation_reason(variant);
                comments.push(MappedComment::new(
                    format!("{} ({reason})", format_style_variant(variant)),
                    variant.span.or(element.span),
                    SourceMappingKind::Style,
                ));
            } else if let Some(method) = self.gpui_style_variant_method(variant) {
                let method = self.mark_source(method, variant.span, SourceMappingKind::Style);
                push_method(expression, 0, &method);
            } else {
                let reason = style_variant_preservation_reason(variant);
                comments.push(MappedComment::new(
                    format!("{} ({reason})", format_style_variant(variant)),
                    variant.span.or(element.span),
                    SourceMappingKind::Style,
                ));
            }
        }

        self.push_dynamic_style_methods(expression, element, comments, scope);
    }

    fn push_root_style_methods(
        &mut self,
        expression: &mut String,
        plan: &RenderPlan,
        comments: &mut Vec<MappedComment>,
    ) {
        for style in &plan.root.styles {
            if let Some(method) = self.gpui_style_method(style) {
                if style_should_preserve_original_css(style) {
                    comments.push(preserved_style_comment(
                        "preserved root CSS",
                        style,
                        plan.root.span,
                    ));
                }
                if method.is_empty() {
                    continue;
                }
                let method = self.mark_source(method, style.span, SourceMappingKind::Style);
                push_method(expression, 0, &method);
            } else {
                comments.push(MappedComment::new(
                    format!("unmapped root CSS: {}: {}", style.property, style.value),
                    style.span.or(plan.root.span),
                    SourceMappingKind::Style,
                ));
            }
        }

        for variant in &plan.root.style_variants {
            if let Some(method) = self.gpui_style_variant_method(variant) {
                let method = self.mark_source(method, variant.span, SourceMappingKind::Style);
                push_method(expression, 0, &method);
            } else {
                let reason = style_variant_preservation_reason(variant);
                comments.push(MappedComment::new(
                    format!("{} ({reason})", format_style_variant(variant)),
                    variant.span.or(plan.root.span),
                    SourceMappingKind::Style,
                ));
            }
        }
    }

    fn push_dynamic_style_comments(
        &self,
        element: &RenderElement,
        comments: &mut Vec<MappedComment>,
    ) {
        for dynamic_style in &element.dynamic_styles {
            comments.push(dynamic_style_comment(dynamic_style, element));
        }
    }

    fn push_dynamic_style_methods(
        &mut self,
        expression: &mut String,
        element: &RenderElement,
        comments: &mut Vec<MappedComment>,
        scope: &RenderScope,
    ) {
        for dynamic_style in &element.dynamic_styles {
            comments.push(dynamic_style_comment(dynamic_style, element));
            if let Some(method) = self.gpui_dynamic_style_method(dynamic_style, scope) {
                let method = self.mark_source(
                    method,
                    dynamic_style.span.or(dynamic_style.expression.span),
                    SourceMappingKind::Style,
                );
                push_method(expression, 0, &method);
            }
        }
    }

    fn gpui_dynamic_style_method(
        &mut self,
        dynamic_style: &RenderDynamicStyleBinding,
        scope: &RenderScope,
    ) -> Option<String> {
        let declarations = dynamic_style_declarations(&dynamic_style.expression)?;
        let mut methods = Vec::new();
        for (property, value) in declarations {
            let method = self.gpui_dynamic_style_declaration_method(&property, &value, scope)?;
            if !method.is_empty() {
                methods.push(method);
            }
        }
        if methods.is_empty() {
            return Some(String::new());
        }

        if let Some(state) = dynamic_style.state.as_deref() {
            let method_name =
                dynamic_style_method_name(&[RenderStyleCondition::PseudoClass(state.into())])?;
            return Some(format!(
                ".{method_name}(|this| {})",
                chain_style_methods("this", &methods)
            ));
        }

        join_style_methods(methods)
    }

    fn gpui_dynamic_style_declaration_method(
        &mut self,
        property: &StyleProperty,
        value: &TemplateString,
        scope: &RenderScope,
    ) -> Option<String> {
        if value.expressions().next().is_none() {
            return gpui_style_method_for_value(property, value.raw.as_str());
        }

        match property {
            StyleProperty::Width => self.gpui_dynamic_definite_length_method("w", value, scope),
            StyleProperty::Height => self.gpui_dynamic_definite_length_method("h", value, scope),
            StyleProperty::MinWidth => {
                self.gpui_dynamic_definite_length_method("min_w", value, scope)
            }
            StyleProperty::MinHeight => {
                self.gpui_dynamic_definite_length_method("min_h", value, scope)
            }
            StyleProperty::MaxWidth => {
                self.gpui_dynamic_definite_length_method("max_w", value, scope)
            }
            StyleProperty::MaxHeight => {
                self.gpui_dynamic_definite_length_method("max_h", value, scope)
            }
            StyleProperty::Top => self.gpui_dynamic_definite_length_method("top", value, scope),
            StyleProperty::Right => self.gpui_dynamic_definite_length_method("right", value, scope),
            StyleProperty::Bottom => {
                self.gpui_dynamic_definite_length_method("bottom", value, scope)
            }
            StyleProperty::Left => self.gpui_dynamic_definite_length_method("left", value, scope),
            StyleProperty::PaddingTop => {
                self.gpui_dynamic_definite_length_method("pt", value, scope)
            }
            StyleProperty::PaddingRight => {
                self.gpui_dynamic_definite_length_method("pr", value, scope)
            }
            StyleProperty::PaddingBottom => {
                self.gpui_dynamic_definite_length_method("pb", value, scope)
            }
            StyleProperty::PaddingLeft => {
                self.gpui_dynamic_definite_length_method("pl", value, scope)
            }
            StyleProperty::MarginTop => {
                self.gpui_dynamic_definite_length_method("mt", value, scope)
            }
            StyleProperty::MarginRight => {
                self.gpui_dynamic_definite_length_method("mr", value, scope)
            }
            StyleProperty::MarginBottom => {
                self.gpui_dynamic_definite_length_method("mb", value, scope)
            }
            StyleProperty::MarginLeft => {
                self.gpui_dynamic_definite_length_method("ml", value, scope)
            }
            StyleProperty::Gap => self.gpui_dynamic_definite_length_method("gap", value, scope),
            StyleProperty::FlexBasis => self.gpui_dynamic_length_method("flex_basis", value, scope),
            StyleProperty::Flex => self.gpui_dynamic_flex_method(value, scope),
            StyleProperty::Color => {
                self.used_dynamic_color_helper = true;
                Some(format!(
                    ".text_color(htmlswap_dynamic_color({}))",
                    template_text_expression(value, scope)
                ))
            }
            StyleProperty::Background | StyleProperty::BackgroundColor => {
                self.used_dynamic_color_helper = true;
                Some(format!(
                    ".bg(htmlswap_dynamic_color({}))",
                    template_text_expression(value, scope)
                ))
            }
            StyleProperty::BorderColor => {
                self.used_dynamic_color_helper = true;
                Some(format!(
                    ".border_color(htmlswap_dynamic_color({}))",
                    template_text_expression(value, scope)
                ))
            }
            _ => None,
        }
    }

    fn gpui_dynamic_definite_length_method(
        &mut self,
        method: &str,
        value: &TemplateString,
        scope: &RenderScope,
    ) -> Option<String> {
        self.used_dynamic_length_helper = true;
        Some(format!(
            ".{method}(htmlswap_dynamic_definite_length({}))",
            template_text_expression(value, scope)
        ))
    }

    fn gpui_dynamic_length_method(
        &mut self,
        method: &str,
        value: &TemplateString,
        scope: &RenderScope,
    ) -> Option<String> {
        self.used_dynamic_length_helper = true;
        Some(format!(
            ".{method}(htmlswap_dynamic_length({}))",
            template_text_expression(value, scope)
        ))
    }

    fn gpui_dynamic_flex_method(
        &mut self,
        value: &TemplateString,
        scope: &RenderScope,
    ) -> Option<String> {
        let raw = value.raw.trim();
        let (prefix, methods): (&str, &[&str]) = if raw.starts_with("0 0 ") {
            ("0 0 ", &[".flex_shrink_0()", ".flex_none()"])
        } else if raw.starts_with("0 1 ") {
            ("0 1 ", &[])
        } else if raw.starts_with("1 1 ") {
            ("1 1 ", &[".flex_grow_1()"])
        } else if raw.starts_with("1 0 ") {
            ("1 0 ", &[".flex_grow_1()", ".flex_shrink_0()"])
        } else {
            return None;
        };
        let basis_raw = raw.strip_prefix(prefix)?.trim();
        let basis = parse_dynamic_style_value_template(basis_raw, value.span)?;

        self.used_dynamic_length_helper = true;
        let mut output = methods
            .iter()
            .map(|method| (*method).to_owned())
            .collect::<Vec<_>>();
        output.push(format!(
            ".flex_basis(htmlswap_dynamic_length({}))",
            template_text_expression(&basis, scope)
        ));
        join_style_methods(output)
    }

    fn push_pseudo_element_comments(
        &mut self,
        element: &RenderElement,
        comments: &mut Vec<MappedComment>,
        spec: &GpuiElementSpec,
    ) {
        for pseudo in &element.pseudo_elements {
            if matches!(
                spec.children,
                ChildEmission::Children
                    | ChildEmission::TitleBarChildren
                    | ChildEmission::FieldsetChildren
            ) && (pseudo_element_emits_as_child(pseudo, "before")
                || pseudo_element_emits_as_child(pseudo, "after"))
            {
                continue;
            }

            let reason = pseudo_element_preservation_reason(pseudo, spec);
            comments.push(MappedComment::new(
                format!("{} ({reason})", format_pseudo_element(pseudo)),
                pseudo.span.or(element.span),
                SourceMappingKind::Style,
            ));
        }
    }

    fn push_accessibility_methods(
        &mut self,
        expression: &mut String,
        element: &RenderElement,
        comments: &mut Vec<MappedComment>,
        spec: &GpuiElementSpec,
    ) {
        let Some(accessibility) = &element.accessibility else {
            return;
        };

        if let Some(tab_index) = accessibility.tab_index {
            match spec.accessibility {
                AccessibilityEmission::Methods => {
                    let method = self.mark_source(
                        format!(".tab_index({tab_index})"),
                        attribute_span(element, "tabindex").or(element.span),
                        SourceMappingKind::Attribute,
                    );
                    push_method(expression, 0, &method);
                }
                AccessibilityEmission::Comment => {
                    self.warn(
                        "tabindex is preserved but not emitted for this GPUI element",
                        element.span,
                    );
                    comments.push(MappedComment::new(
                        format!("accessibility tabindex: {tab_index}"),
                        attribute_span(element, "tabindex").or(element.span),
                        SourceMappingKind::Attribute,
                    ));
                }
            }
        }

        if accessibility.hidden {
            match spec.accessibility {
                AccessibilityEmission::Methods => {
                    let method = self.mark_source(
                        ".hidden()",
                        attribute_span(element, "hidden")
                            .or_else(|| attribute_span(element, "aria-hidden"))
                            .or(element.span),
                        SourceMappingKind::Attribute,
                    );
                    push_method(expression, 0, &method);
                }
                AccessibilityEmission::Comment => {
                    self.warn(
                        "hidden state is preserved but not emitted for this GPUI element",
                        element.span,
                    );
                    comments.push(MappedComment::new(
                        "accessibility hidden",
                        attribute_span(element, "hidden")
                            .or_else(|| attribute_span(element, "aria-hidden"))
                            .or(element.span),
                        SourceMappingKind::Attribute,
                    ));
                }
            }
        }

        if accessibility.autofocus {
            self.warn(
                "autofocus requires generated focus side effects and is preserved as metadata",
                element.span,
            );
            comments.push(MappedComment::new(
                "accessibility autofocus",
                attribute_span(element, "autofocus").or(element.span),
                SourceMappingKind::Attribute,
            ));
        }

        if let Some(role) = &accessibility.role {
            comments.push(MappedComment::new(
                format!("accessibility role: {role}"),
                attribute_span(element, "role").or(element.span),
                SourceMappingKind::Attribute,
            ));
        }
        if let Some(label) = &accessibility.label {
            comments.push(MappedComment::new(
                format!("accessibility label: {label:?}"),
                attribute_span(element, "aria-label").or(element.span),
                SourceMappingKind::Attribute,
            ));
        }
        if let Some(labelled_by) = &accessibility.labelled_by {
            comments.push(MappedComment::new(
                format!("accessibility labelled-by: {labelled_by}"),
                attribute_span(element, "aria-labelledby").or(element.span),
                SourceMappingKind::Attribute,
            ));
        }
        if let Some(described_by) = &accessibility.described_by {
            comments.push(MappedComment::new(
                format!("accessibility described-by: {described_by}"),
                attribute_span(element, "aria-describedby").or(element.span),
                SourceMappingKind::Attribute,
            ));
        }
        for attribute in &accessibility.aria {
            comments.push(MappedComment::new(
                format!("accessibility {}={:?}", attribute.name, attribute.value),
                attribute.span.or(element.span),
                SourceMappingKind::Attribute,
            ));
        }
    }

    fn push_debug_layout_id_if_needed(
        &mut self,
        expression: &mut String,
        element: &RenderElement,
        spec: &GpuiElementSpec,
        scope: &RenderScope,
    ) -> bool {
        if !self.options.emit_debug_layout_ids
            || spec.id_attribute != IdAttributeEmission::Method
            || element_id(element).is_some()
        {
            return false;
        }

        let id = layout_debug_id_for_element_in_sources(element, self.sources)
            .unwrap_or_else(|| self.generated_id("debug"));
        let method = self.mark_source(
            format!(".id({})", gpui_generated_element_id_expression(&id, scope)),
            element.span,
            SourceMappingKind::Attribute,
        );
        push_method(expression, 0, &method);
        true
    }

    fn push_generated_stateful_id_if_needed(
        &mut self,
        expression: &mut String,
        element: &RenderElement,
        spec: &GpuiElementSpec,
        has_runtime_id: bool,
        scope: &RenderScope,
    ) {
        if spec.id_attribute != IdAttributeEmission::Method
            || has_runtime_id
            || element
                .source_intent
                .as_ref()
                .and_then(|hints| hints.key.as_ref())
                .is_some()
            || !element_needs_generated_stateful_id(element)
        {
            return;
        }

        let id = self.generated_id("interactive");
        push_method(
            expression,
            0,
            &format!(".id({})", gpui_generated_element_id_expression(&id, scope)),
        );
    }

    fn push_action_methods(
        &mut self,
        expression: &mut String,
        element: &RenderElement,
        comments: &mut Vec<MappedComment>,
        spec: &GpuiElementSpec,
        scope: &RenderScope,
    ) {
        if spec.actions == ActionEmission::Comment {
            for action in &element.actions {
                comments.push(MappedComment::new(
                    format_action_comment(action),
                    action.span.or(element.span),
                    SourceMappingKind::Action,
                ));
                if let Some(payload) = action_payload_comment(&action.payload) {
                    comments.push(MappedComment::new(
                        payload,
                        action.span.or(element.span),
                        SourceMappingKind::Action,
                    ));
                }
            }
            return;
        }
        if spec.actions == ActionEmission::Delegate {
            return;
        }

        for action in &element.actions {
            if self.action_uses_text_input_subscription(element, action, scope) {
                if action.event != "input"
                    || self
                        .dynamic_source_text_input_backend(element, scope)
                        .is_some()
                {
                    self.push_preserved_action_comment(comments, action, element, true);
                }
                continue;
            }
            if !spec.supports_action_event(&action.event) {
                self.push_preserved_action_comment(comments, action, element, true);
                continue;
            }

            let Some(method_name) = gpui_event_method(&action.event, spec.actions) else {
                self.push_preserved_action_comment(comments, action, element, true);
                continue;
            };

            let Some(method) =
                self.action_method(method_name, action, element, comments, spec, scope)
            else {
                continue;
            };
            let method = self.mark_source(
                method,
                action.span.or(element.span),
                SourceMappingKind::Action,
            );
            push_method(expression, 0, &method);
        }
    }

    fn push_title_bar_interaction_methods(
        &mut self,
        expression: &mut String,
        element: &RenderElement,
        spec: &GpuiElementSpec,
        scope: &RenderScope,
    ) {
        if !title_bar_descendant_needs_mouse_down_stop(element, spec, scope) {
            return;
        }

        push_method(
            expression,
            0,
            ".on_mouse_down(gpui::MouseButton::Left, |_, _window, _cx| {\n        _cx.stop_propagation();\n    })",
        );
    }

    fn action_uses_text_input_subscription(
        &self,
        element: &RenderElement,
        action: &ActionBinding,
        scope: &RenderScope,
    ) -> bool {
        if !matches!(action.event.as_str(), "input" | "change") {
            return false;
        }
        let Some(state) = &element.state else {
            return false;
        };
        if !matches!(&state.kind, RenderStateKind::TextInput(_)) {
            return false;
        }
        if !matches!(&action.payload, ActionPayload::ElementState { state_id } if state_id == state.id)
        {
            return false;
        }
        if !action.handler.effects.is_empty() {
            return false;
        }
        let is_dynamic_source_input = self
            .dynamic_source_text_input_backend(element, scope)
            .is_some();
        let local_captures = action_local_captures(action, scope);
        if !is_dynamic_source_input && !self.used_state_ids.contains(&state.id) {
            return false;
        }
        if !is_dynamic_source_input && !local_captures.is_empty() {
            return false;
        }

        input_subscription_action_call(action, scope, "&_htmlswap_value").is_some()
    }

    fn action_method(
        &mut self,
        method_name: &str,
        action: &ActionBinding,
        element: &RenderElement,
        comments: &mut Vec<MappedComment>,
        spec: &GpuiElementSpec,
        scope: &RenderScope,
    ) -> Option<String> {
        let mut statements = Vec::new();
        let event_argument = listener_event_argument(method_name);
        self.push_handler_effect_statements(&mut statements, action, event_argument);
        self.push_payload_action_statements(&mut statements, action, element, comments, spec);

        if let Some(call) = template_action_call(action, scope, event_argument) {
            let call = self.mark_source(call, action.span, SourceMappingKind::Action);
            statements.push(call);
        } else if let Some(call) = action_call(action, event_argument) {
            let call = self.mark_source(call, action.action_span, SourceMappingKind::Action);
            statements.push(call);
        } else if !action.resolved {
            self.preserve_action_comment(
                comments,
                action,
                element,
                format!("event handler `{}` is unresolved", action.expression),
                format!("unresolved action: {}", action.expression),
                statements.is_empty(),
            );
        } else if action.action.is_some() {
            self.preserve_action_comment(
                comments,
                action,
                element,
                format!(
                    "event handler `{}` cannot be emitted as a Rust function call",
                    action.expression
                ),
                format!("unmapped action call: {}", action.expression),
                statements.is_empty(),
            );
        }

        if statements.is_empty() {
            return None;
        }

        let body = statements.join("\n");
        let local_captures = action_local_captures(action, scope);
        if method_name == "on_click" && is_double_click_event(&action.event) {
            let listener = wrap_listener_with_local_captures(
                &local_captures,
                format!(
                    "_cx.listener(move |this, _event: &gpui::ClickEvent, _window, _cx| {{\n        if _event.click_count() == 2 {{\n{}\n        }}\n    }})",
                    indent_expression(&body, 3)
                ),
            );
            return Some(format!(".on_click({listener})"));
        }

        let simple_click = method_name == "on_click"
            && action.event == "click"
            && action.template.is_none()
            && matches!(&action.payload, ActionPayload::None)
            && action.handler.effects.is_empty()
            && local_captures.is_empty()
            && action
                .handler
                .primary_invocation()
                .is_none_or(|invocation| invocation.arguments.is_empty());
        if simple_click {
            return Some(format!(".on_click(|_, _, _| {{\n        {body}\n    }})"));
        }

        if method_name == "on_hover" {
            let hovered_condition = if action.event == "mouseleave" {
                "!*is_hovered"
            } else {
                "*is_hovered"
            };
            let listener = wrap_listener_with_local_captures(
                &local_captures,
                format!(
                    "_cx.listener(move |this, is_hovered, _window, _cx| {{\n        if {hovered_condition} {{\n{}\n        }}\n    }})",
                    indent_expression(&body, 3)
                ),
            );
            return Some(format!(".on_hover({listener})"));
        }

        let listener = wrap_listener_with_local_captures(
            &local_captures,
            format!(
                "_cx.listener(move |this, _event, _window, _cx| {{\n{}\n    }})",
                indent_expression(&body, 2)
            ),
        );
        Some(format!(".{method_name}({listener})"))
    }

    fn push_handler_effect_statements(
        &mut self,
        statements: &mut Vec<String>,
        action: &ActionBinding,
        event_argument: &str,
    ) {
        for effect in &action.handler.effects {
            if event_argument != "_event" {
                statements.push(format!(
                    "// handler effect from `{}` cannot be emitted for GPUI `{}` listeners",
                    action.expression, action.event
                ));
                continue;
            }

            let statement = match effect {
                RenderActionHandlerEffect::PreventDefault { span } => self.mark_source(
                    "_event.prevent_default();",
                    *span,
                    SourceMappingKind::Action,
                ),
                RenderActionHandlerEffect::StopPropagation { span } => self.mark_source(
                    "_event.stop_propagation();",
                    *span,
                    SourceMappingKind::Action,
                ),
            };
            statements.push(statement);
        }
    }

    fn push_payload_action_statements(
        &mut self,
        statements: &mut Vec<String>,
        action: &ActionBinding,
        element: &RenderElement,
        comments: &mut Vec<MappedComment>,
        spec: &GpuiElementSpec,
    ) {
        match &action.payload {
            ActionPayload::None => {}
            ActionPayload::ElementState { state_id } => {
                if spec.state_actions != StateActionEmission::Methods {
                    self.preserve_action_comment(
                        comments,
                        action,
                        element,
                        format!(
                            "state update for event `{}` cannot be emitted for this GPUI element",
                            action.event
                        ),
                        action_payload_comment(&action.payload)
                            .unwrap_or_else(|| "unmapped state update payload".to_owned()),
                        false,
                    );
                    return;
                }

                let source = state_value_source_for_element(element);
                if let Some(statement) =
                    self.state_update_statement(state_id, source, action.span.or(element.span))
                {
                    statements.push(statement);
                }
            }
            ActionPayload::FormData { controls } => {
                if spec.state_actions != StateActionEmission::Methods {
                    self.preserve_action_comment(
                        comments,
                        action,
                        element,
                        format!(
                            "form data for event `{}` cannot be emitted for this GPUI element",
                            action.event
                        ),
                        action_payload_comment(&action.payload)
                            .unwrap_or_else(|| "unmapped form data payload".to_owned()),
                        false,
                    );
                    return;
                }

                statements.push(self.form_data_statement(controls, action.span.or(element.span)));
            }
        }
    }

    fn state_update_statement(
        &mut self,
        state_id: &str,
        source: RenderStateValueSource,
        span: Option<Span>,
    ) -> Option<String> {
        let Some(binding) = self.state_bindings.get(state_id).cloned() else {
            self.warn(format!("state binding `{state_id}` is unavailable"), span);
            return None;
        };
        if binding.owner != RenderStateOwner::Target {
            self.warn(
                format!(
                    "state binding `{}` is {:?}-owned and cannot be mutated by the GPUI adapter",
                    binding.id, binding.owner
                ),
                binding.span.or(span),
            );
            return None;
        }
        let Some(field) = self.ensure_state_field(&binding) else {
            self.warn(
                format!(
                    "state binding `{}` cannot be represented as a GPUI field",
                    binding.id
                ),
                binding.span.or(span),
            );
            return None;
        };

        let statement = match (&binding.kind, source.clone()) {
            (RenderStateKind::TextInput(_), RenderStateValueSource::EventValue) => {
                match self.state_field_backend(&binding) {
                    GpuiStateFieldBackend::ComponentTextInput => format!(
                        "this.{field}.update(_cx, |state, _cx| {{\n    state.set_value(_event.into(), _window, _cx);\n}});"
                    ),
                    GpuiStateFieldBackend::GpuiTextInput => format!(
                        "this.{field}.update(_cx, |state, _cx| {{\n    state.set_value(_event.into(), _cx);\n}});"
                    ),
                }
            }
            (RenderStateKind::TextInput(_), RenderStateValueSource::Static(value)) => {
                match self.state_field_backend(&binding) {
                    GpuiStateFieldBackend::ComponentTextInput => format!(
                        "this.{field}.update(_cx, |state, _cx| {{\n    state.set_value({}.into(), _window, _cx);\n}});",
                        rust_string(&value)
                    ),
                    GpuiStateFieldBackend::GpuiTextInput => format!(
                        "this.{field}.update(_cx, |state, _cx| {{\n    state.set_value({}.into(), _cx);\n}});",
                        rust_string(&value)
                    ),
                }
            }
            (RenderStateKind::Choice(_), RenderStateValueSource::EventValue) => format!(
                "this.{field}.update(_cx, |state, _cx| {{\n    state.set_selected(_event, _window, _cx);\n}});"
            ),
            (RenderStateKind::Choice(_), RenderStateValueSource::Static(value)) => format!(
                "this.{field}.update(_cx, |state, _cx| {{\n    state.set_selected({}.into(), _window, _cx);\n}});",
                rust_string(&value)
            ),
            (RenderStateKind::Toggle(_), RenderStateValueSource::EventChecked)
            | (RenderStateKind::Toggle(_), RenderStateValueSource::EventValue) => {
                format!("this.{field} = _event;\n_cx.notify();")
            }
            (_, RenderStateValueSource::FormField { name }) => format!(
                "// htmlswap state update for form field {} requires target-specific extraction",
                rust_string(&name)
            ),
            _ => {
                self.warn(
                    format!(
                        "state binding `{}` cannot be updated from {:?}",
                        binding.id, source
                    ),
                    binding.span.or(span),
                );
                return None;
            }
        };

        Some(self.mark_source(statement, binding.span.or(span), SourceMappingKind::State))
    }

    fn form_data_statement(
        &mut self,
        controls: &[RenderFormDataField],
        span: Option<Span>,
    ) -> String {
        let mut output =
            "let _htmlswap_form_data = {\n    let mut data = std::collections::BTreeMap::new();"
                .to_owned();

        for control in controls {
            let name = rust_string(&control.name);
            let value = self.form_data_value_expression(control);
            write!(output, "\n    data.insert({name}, {value});")
                .expect("writing to String cannot fail");
        }

        output.push_str("\n    data\n};");
        self.mark_source(output, span, SourceMappingKind::Action)
    }

    fn form_data_value_expression(&mut self, control: &RenderFormDataField) -> String {
        if let Some(state_id) = &control.state_id
            && let Some(binding) = self.state_bindings.get(state_id).cloned()
            && binding.owner == RenderStateOwner::Target
            && let Some(field) = self.ensure_state_field(&binding)
        {
            return match binding.kind {
                RenderStateKind::TextInput(_) => match self.state_field_backend(&binding) {
                    GpuiStateFieldBackend::ComponentTextInput => {
                        format!("this.{field}.read(_cx).value().to_string()")
                    }
                    GpuiStateFieldBackend::GpuiTextInput => {
                        format!("this.{field}.read(_cx).value().to_owned()")
                    }
                },
                RenderStateKind::Choice(_) => {
                    format!("this.{field}.read(_cx).value().to_string()")
                }
                RenderStateKind::Toggle(_) => format!("this.{field}.to_string()"),
            };
        }

        control.value.as_ref().map_or_else(
            || "String::new()".to_owned(),
            |value| format!("String::from({})", rust_string(value)),
        )
    }

    fn ensure_state_field(&mut self, binding: &RenderStateBinding) -> Option<String> {
        let field = self.state_fields.get(&binding.id)?.clone();
        self.used_state_ids.insert(binding.id.clone());
        Some(field)
    }

    fn preserve_action_comment(
        &mut self,
        comments: &mut Vec<MappedComment>,
        action: &ActionBinding,
        element: &RenderElement,
        diagnostic: impl Into<String>,
        comment: impl Into<String>,
        include_payload: bool,
    ) {
        self.warn(diagnostic, action.span.or(element.span));
        comments.push(MappedComment::new(
            comment,
            action.span.or(element.span),
            SourceMappingKind::Action,
        ));
        if include_payload && let Some(payload) = action_payload_comment(&action.payload) {
            comments.push(MappedComment::new(
                payload,
                action.span.or(element.span),
                SourceMappingKind::Action,
            ));
        }
    }

    fn push_preserved_action_comment(
        &self,
        comments: &mut Vec<MappedComment>,
        action: &ActionBinding,
        element: &RenderElement,
        include_payload: bool,
    ) {
        comments.push(MappedComment::new(
            format_action_comment(action),
            action.span.or(element.span),
            SourceMappingKind::Action,
        ));
        if include_payload && let Some(payload) = action_payload_comment(&action.payload) {
            comments.push(MappedComment::new(
                payload,
                action.span.or(element.span),
                SourceMappingKind::Action,
            ));
        }
    }

    fn push_child(
        &mut self,
        output: &mut String,
        node: &RenderNode,
        depth: usize,
        scope: &RenderScope,
    ) {
        if let RenderNode::Element(element) = node
            && let Some(control_flow) = &element.control_flow
        {
            match control_flow.kind {
                RenderControlFlowKind::For => {
                    self.push_for_children(output, element, depth, scope);
                    return;
                }
                RenderControlFlowKind::If | RenderControlFlowKind::ElseIf => {
                    self.push_if_children(output, element, depth, scope);
                    return;
                }
                RenderControlFlowKind::Else => {
                    self.push_control_flow_children(output, element, depth, scope);
                    return;
                }
                RenderControlFlowKind::Switch => {
                    self.push_switch_children(output, element, depth, scope);
                    return;
                }
                RenderControlFlowKind::Case | RenderControlFlowKind::Default => {}
            }
        }

        self.push_plain_child(output, node, depth, scope, true);
    }

    fn push_plain_child(
        &mut self,
        output: &mut String,
        node: &RenderNode,
        depth: usize,
        scope: &RenderScope,
        leading_newline: bool,
    ) {
        if leading_newline {
            output.push('\n');
        }
        output.push_str(&indent(depth + 1));
        output.push_str(".child(\n");
        output.push_str(&self.render_node(node, depth + 2, scope));
        output.push('\n');
        output.push_str(&indent(depth + 1));
        output.push(')');
    }

    fn push_ordered_child(
        &mut self,
        output: &mut String,
        child: StackedChild<'_>,
        depth: usize,
        scope: &RenderScope,
    ) {
        if child.emit_order_hint {
            let prefix = if control_flow_node(child.node).is_some() {
                "htmlswap control-flow source-order"
            } else {
                "htmlswap source-order"
            };
            output.push('\n');
            let source_order = self.mark_line_comment_source(
                &indent(depth + 1),
                format!("{prefix}: {}", child.source_index),
                node_span(child.node),
                SourceMappingKind::Element,
            );
            writeln!(output, "{source_order}").expect("writing to String cannot fail");

            let z_index = child
                .z_index
                .map_or_else(|| "auto".to_owned(), |z_index| z_index.to_string());
            let paint_order = self.mark_line_comment_source(
                &indent(depth + 1),
                format!(
                    "htmlswap paint-order: {} z-index: {z_index}",
                    child.paint_index
                ),
                node_span(child.node),
                SourceMappingKind::Element,
            );
            writeln!(output, "{paint_order}").expect("writing to String cannot fail");
        }

        if child.emit_order_hint && control_flow_node(child.node).is_none() {
            self.push_plain_child(output, child.node, depth, scope, false);
        } else {
            self.push_child(output, child.node, depth, scope);
        }
    }

    fn push_title_bar_children(
        &mut self,
        output: &mut String,
        element: &RenderElement,
        scope: &RenderScope,
        comments: &mut Vec<MappedComment>,
    ) {
        let mut skipped_controls = false;
        let title_bar_scope = scope.inside_title_bar_drag_area();

        self.push_pseudo_children(output, element, "before", 0, &title_bar_scope);
        for child in stacked_child_nodes(&element.children) {
            if is_title_bar_window_control_child(child.node) {
                if !skipped_controls {
                    comments.push(MappedComment::new(
                        "source window controls replaced by gpui_component::TitleBar native controls",
                        node_span(child.node).or(element.span),
                        SourceMappingKind::Element,
                    ));
                    skipped_controls = true;
                }
                comments.push(MappedComment::new(
                    format!(
                        "htmlswap raw child HTML omitted: {}",
                        one_line(render_html_fragment(std::slice::from_ref(child.node)).trim())
                    ),
                    node_span(child.node).or(element.span),
                    SourceMappingKind::Raw,
                ));
                continue;
            }

            self.push_ordered_child(output, child, 0, &title_bar_scope);
        }
        self.push_pseudo_children(output, element, "after", 0, &title_bar_scope);
    }

    fn push_for_children(
        &mut self,
        output: &mut String,
        element: &RenderElement,
        depth: usize,
        scope: &RenderScope,
    ) {
        let Some(control_flow) = &element.control_flow else {
            return;
        };
        let Some(list) = control_flow.expression.as_ref() else {
            self.warn(
                "source loop is missing a collection expression",
                control_flow.span.or(element.span),
            );
            return;
        };
        let item = control_flow
            .binding
            .as_ref()
            .map_or("item", |binding| binding.name.as_str());
        let item_ident = sanitize_rust_identifier(&item.to_snake_case(), "item");
        let index_ident = self.generated_id("index");
        let next_scope = scope.with_loop_local(item_ident.as_str(), index_ident.as_str());
        let collection = rust_binding_expression(list, scope);
        let body = self.render_control_flow_body_as_expression(element, depth + 3, &next_scope);

        if !self.push_control_flow_source_comment(output, element, depth) {
            output.push('\n');
        }
        output.push_str(&indent(depth + 1));
        output.push_str(".children(\n");
        output.push_str(&indent(depth + 2));
        output.push_str(&format!(
            "{collection}.iter().enumerate().map(|({index_ident}, {item_ident})| {{\n"
        ));
        output.push_str(&indent(depth + 3));
        output.push_str(&format!("let {item_ident} = {item_ident}.clone();\n"));
        output.push_str(&body);
        output.push('\n');
        output.push_str(&indent(depth + 2));
        output.push_str("})\n");
        output.push_str(&indent(depth + 1));
        output.push(')');
    }

    fn push_if_children(
        &mut self,
        output: &mut String,
        element: &RenderElement,
        depth: usize,
        scope: &RenderScope,
    ) {
        let Some(control_flow) = &element.control_flow else {
            return;
        };
        let Some(condition) = control_flow.expression.as_ref() else {
            self.warn(
                "source conditional is missing a condition expression",
                control_flow.span.or(element.span),
            );
            return;
        };
        let condition = rust_binding_expression(condition, scope);

        if !self.push_control_flow_source_comment(output, element, depth) {
            output.push('\n');
        }
        output.push_str(&indent(depth + 1));
        output.push_str(&format!(".when({condition}, |this| {{"));
        let mut branch = format!("{}this", indent(depth + 2));
        self.push_control_flow_body_children(&mut branch, element, depth + 1, scope);
        output.push('\n');
        output.push_str(&branch);
        output.push('\n');
        output.push_str(&indent(depth + 1));
        output.push_str("})");
    }

    fn push_control_flow_children(
        &mut self,
        output: &mut String,
        element: &RenderElement,
        depth: usize,
        scope: &RenderScope,
    ) {
        self.push_control_flow_body_children(output, element, depth, scope);
    }

    fn push_control_flow_body_children(
        &mut self,
        output: &mut String,
        element: &RenderElement,
        depth: usize,
        scope: &RenderScope,
    ) {
        if element
            .control_flow
            .as_ref()
            .is_some_and(|control_flow| control_flow.host == RenderControlFlowHost::Element)
        {
            output.push('\n');
            output.push_str(&indent(depth + 1));
            output.push_str(".child(\n");
            output.push_str(&self.render_element(element, depth + 2, scope));
            output.push('\n');
            output.push_str(&indent(depth + 1));
            output.push(')');
        } else {
            for child in stacked_child_nodes(&element.children) {
                self.push_ordered_child(output, child, depth, scope);
            }
        }
    }

    fn push_switch_children(
        &mut self,
        output: &mut String,
        element: &RenderElement,
        depth: usize,
        scope: &RenderScope,
    ) {
        let Some(control_flow) = &element.control_flow else {
            return;
        };
        let Some(discriminant) = control_flow.expression.as_ref() else {
            self.warn(
                "source switch is missing a discriminant expression",
                control_flow.span.or(element.span),
            );
            return;
        };
        let _ = self.push_control_flow_source_comment(output, element, depth);
        let discriminant = rust_binding_expression(discriminant, scope);
        let mut case_conditions = Vec::new();
        let mut default_children = None;

        for child in &element.children {
            let RenderNode::Element(case_element) = child else {
                self.push_child(output, child, depth, scope);
                continue;
            };
            let Some(case_flow) = &case_element.control_flow else {
                self.push_child(output, child, depth, scope);
                continue;
            };

            match case_flow.kind {
                RenderControlFlowKind::Case => {
                    let Some(case_expression) = case_flow.expression.as_ref() else {
                        self.warn(
                            "source switch case is missing a value expression",
                            case_flow.span.or(case_element.span),
                        );
                        continue;
                    };
                    let case_value = rust_binding_expression(case_expression, scope);
                    let condition = format!("{discriminant} == {case_value}");
                    case_conditions.push(condition.clone());
                    self.push_case_when(output, case_element, depth, scope, &condition);
                }
                RenderControlFlowKind::Default => {
                    default_children = Some(case_element.as_ref());
                }
                _ => self.push_child(output, child, depth, scope),
            }
        }

        if let Some(default_element) = default_children {
            let condition = if case_conditions.is_empty() {
                "true".to_owned()
            } else {
                format!("!({})", case_conditions.join(" || "))
            };
            self.push_case_when(output, default_element, depth, scope, &condition);
        }
    }

    fn push_case_when(
        &mut self,
        output: &mut String,
        element: &RenderElement,
        depth: usize,
        scope: &RenderScope,
        condition: &str,
    ) {
        self.push_case_when_with_comment_state(output, element, depth, scope, condition, true);
    }

    fn push_case_when_with_comment_state(
        &mut self,
        output: &mut String,
        element: &RenderElement,
        depth: usize,
        scope: &RenderScope,
        condition: &str,
        emit_source_comment: bool,
    ) {
        if !(emit_source_comment && self.push_control_flow_source_comment(output, element, depth)) {
            output.push('\n');
        }
        output.push_str(&indent(depth + 1));
        output.push_str(&format!(".when({condition}, |this| {{"));
        let mut branch = format!("{}this", indent(depth + 2));
        self.push_control_flow_body_children(&mut branch, element, depth + 1, scope);
        output.push('\n');
        output.push_str(&branch);
        output.push('\n');
        output.push_str(&indent(depth + 1));
        output.push_str("})");
    }

    fn render_control_flow_expression(
        &mut self,
        element: &RenderElement,
        depth: usize,
        scope: &RenderScope,
    ) -> String {
        let mut expression = format!("{}gpui::div()", indent(depth));
        if let Some(control_flow) = &element.control_flow {
            match control_flow.kind {
                RenderControlFlowKind::For => {
                    self.push_for_children(&mut expression, element, depth, scope)
                }
                RenderControlFlowKind::If | RenderControlFlowKind::ElseIf => {
                    self.push_if_children(&mut expression, element, depth, scope)
                }
                RenderControlFlowKind::Else => {
                    self.push_control_flow_body_children(&mut expression, element, depth, scope);
                }
                RenderControlFlowKind::Switch => {
                    self.push_switch_children(&mut expression, element, depth, scope);
                }
                RenderControlFlowKind::Case | RenderControlFlowKind::Default => {
                    self.push_control_flow_body_children(&mut expression, element, depth, scope);
                }
            }
        }
        expression
    }

    fn render_control_flow_body_as_expression(
        &mut self,
        element: &RenderElement,
        depth: usize,
        scope: &RenderScope,
    ) -> String {
        if element
            .control_flow
            .as_ref()
            .is_some_and(|control_flow| control_flow.host == RenderControlFlowHost::Element)
        {
            self.render_element(element, depth, scope)
        } else {
            self.render_children_as_expression(element, depth, scope)
        }
    }

    fn render_children_as_expression(
        &mut self,
        element: &RenderElement,
        depth: usize,
        scope: &RenderScope,
    ) -> String {
        match element.children.as_slice() {
            [] => format!("{}gpui::div()", indent(depth)),
            [child] => self.render_node(child, depth, scope),
            children => {
                let mut output = format!("{}gpui::div()", indent(depth));
                for child in stacked_child_nodes(children) {
                    self.push_ordered_child(&mut output, child, depth, scope);
                }
                output
            }
        }
    }

    fn push_pseudo_children(
        &mut self,
        output: &mut String,
        element: &RenderElement,
        kind: &str,
        depth: usize,
        scope: &RenderScope,
    ) {
        for pseudo in &element.pseudo_elements {
            if !pseudo_element_emits_as_child(pseudo, kind) {
                continue;
            }

            output.push('\n');
            output.push_str(&indent(depth + 1));
            output.push_str(".child(\n");
            output.push_str(&self.render_pseudo_element(pseudo, depth + 2, scope));
            output.push('\n');
            output.push_str(&indent(depth + 1));
            output.push(')');
        }
    }

    fn render_pseudo_element(
        &mut self,
        pseudo: &RenderPseudoElement,
        depth: usize,
        scope: &RenderScope,
    ) -> String {
        let mut comments = Vec::new();
        let mut expression = self.mark_source("gpui::div()", pseudo.span, SourceMappingKind::Style);

        for style in &pseudo.styles {
            if let Some(method) = self.gpui_style_method(style) {
                if method.is_empty() {
                    continue;
                }
                let method = self.mark_source(method, style.span, SourceMappingKind::Style);
                push_method(&mut expression, 0, &method);
            } else {
                self.warn(
                    format!(
                        "CSS declaration `{}` on pseudo-element `::{}` is not mapped to GPUI yet",
                        style.property, pseudo.kind
                    ),
                    style.span.or(pseudo.span),
                );
                comments.push(MappedComment::new(
                    format!(
                        "unmapped pseudo-element CSS: {}: {}",
                        style.property, style.value
                    ),
                    style.span.or(pseudo.span),
                    SourceMappingKind::Style,
                ));
            }
        }

        for child in &pseudo.children {
            self.push_child(&mut expression, child, 0, scope);
        }

        self.with_local_comments(expression, depth, &comments)
    }

    fn push_form_fields(&mut self, output: &mut String, form: &RenderElement, scope: &RenderScope) {
        for child in stacked_child_nodes(&form.children) {
            if is_consumed_form_label(form, child.node) {
                continue;
            }

            output.push('\n');
            output.push_str("    .child(\n");
            output.push_str(&self.render_form_field(child.node, 2, scope));
            output.push('\n');
            output.push_str("    )");
        }
    }

    fn render_form_field(
        &mut self,
        node: &RenderNode,
        depth: usize,
        scope: &RenderScope,
    ) -> String {
        let RenderNode::Element(element) = node else {
            let mut field = "gpui_component::form::field()".to_owned();
            self.push_field_child(&mut field, node, scope);
            return indent_expression(&field, depth);
        };

        let Some(control) = &element.form_control else {
            let mut field = "gpui_component::form::field()".to_owned();
            self.push_field_child(&mut field, node, scope);
            return indent_expression(&field, depth);
        };

        let mut field = "gpui_component::form::field()".to_owned();
        if let Some(label) = &control.label {
            push_method(&mut field, 0, &format!(".label({})", rust_string(label)));
        }
        if control.required {
            push_method(&mut field, 0, ".required(true)");
        }
        self.push_field_child(&mut field, node, scope);
        indent_expression(&field, depth)
    }

    fn push_field_child(&mut self, field: &mut String, node: &RenderNode, scope: &RenderScope) {
        field.push('\n');
        field.push_str("    .child(\n");
        field.push_str(&self.render_node(node, 2, scope));
        field.push('\n');
        field.push_str("    )");
    }

    fn with_local_comments(
        &mut self,
        expression: String,
        depth: usize,
        comments: &[MappedComment],
    ) -> String {
        if comments.is_empty() {
            return indent_expression(&expression, depth);
        }

        let mut output = String::new();
        let outer_indent = indent(depth);
        let inner_indent = indent(depth + 1);
        writeln!(output, "{outer_indent}{{").expect("writing to String cannot fail");
        for comment in comments {
            for line in comment.text.lines() {
                let line = self.mark_line_comment_source(
                    &inner_indent,
                    line.trim(),
                    comment.span,
                    comment.kind,
                );
                writeln!(output, "{line}").expect("writing to String cannot fail");
            }
        }
        output.push_str(&indent_expression(&expression, depth + 1));
        output.push('\n');
        output.push_str(&outer_indent);
        output.push('}');
        output
    }

    fn push_control_flow_source_comment(
        &mut self,
        output: &mut String,
        element: &RenderElement,
        depth: usize,
    ) -> bool {
        let Some(comment) = source_control_flow_comment(element) else {
            return false;
        };

        if !output.ends_with('\n') {
            output.push('\n');
        }
        let line = self.mark_line_comment_source(
            &indent(depth + 1),
            comment.text,
            comment.span,
            comment.kind,
        );
        writeln!(output, "{line}").expect("writing to String cannot fail");
        true
    }

    fn element_comments(
        &mut self,
        element: &RenderElement,
        spec: &GpuiElementSpec,
    ) -> Vec<MappedComment> {
        let mut comments = Vec::new();

        if self.options.emit_debug_source_html_comments
            && let Some(source_html) = self.source_html_comment(element.span)
        {
            comments.push(MappedComment::new(
                format!("source html: {source_html}"),
                element.span,
                SourceMappingKind::Element,
            ));
        }

        if self.options.emit_debug_source_html_comments
            && let Some(span) = element.span
        {
            comments.push(MappedComment::new(
                format!(
                    "layout source key: {}",
                    layout_source_key_for_span_in_sources(span, self.sources)
                ),
                element.span,
                SourceMappingKind::Element,
            ));
            comments.push(MappedComment::new(
                format!("layout source span: {}:{}", span.start, span.end),
                element.span,
                SourceMappingKind::Element,
            ));
        }

        if !element.source_tag.is_empty() {
            comments.push(MappedComment::new(
                format!("source tag: {}", element.source_tag),
                element.span,
                SourceMappingKind::Element,
            ));
        }

        if !element.classes.is_empty() {
            let classes = element
                .classes
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<&str>>()
                .join(" ");
            comments.push(MappedComment::new(
                format!("html classes: {classes}"),
                attribute_span(element, "class").or(element.span),
                SourceMappingKind::Attribute,
            ));
        }

        if element
            .control_flow
            .as_ref()
            .is_some_and(|control_flow| control_flow.host == RenderControlFlowHost::Wrapper)
            && let Some(comment) = source_control_flow_comment(element)
        {
            comments.push(comment);
        }

        if let Some(region) = attribute_value(element, "data-htmlswap-region") {
            comments.push(MappedComment::new(
                format!("source region: {region}"),
                attribute_span(element, "data-htmlswap-region").or(element.span),
                SourceMappingKind::Attribute,
            ));
        }

        if let Some(semantics) = &element.semantics {
            if let Some(variant) = &semantics.variant {
                comments.push(MappedComment::new(
                    format!(
                        "source variant: {}",
                        attribute_value(element, "data-htmlswap-variant")
                            .unwrap_or_else(|| variant.as_str())
                    ),
                    attribute_span(element, "data-htmlswap-variant").or(element.span),
                    SourceMappingKind::Attribute,
                ));
            }
            if let Some(tone) = &semantics.tone {
                comments.push(MappedComment::new(
                    format!(
                        "source tone: {}",
                        attribute_value(element, "data-htmlswap-tone")
                            .unwrap_or_else(|| tone.as_str())
                    ),
                    attribute_span(element, "data-htmlswap-tone").or(element.span),
                    SourceMappingKind::Attribute,
                ));
            }
            if let Some(size) = &semantics.size {
                comments.push(MappedComment::new(
                    format!(
                        "source size: {}",
                        attribute_value(element, "data-htmlswap-size")
                            .unwrap_or_else(|| size.as_str())
                    ),
                    attribute_span(element, "data-htmlswap-size").or(element.span),
                    SourceMappingKind::Attribute,
                ));
            }
            if let Some(density) = &semantics.density {
                comments.push(MappedComment::new(
                    format!(
                        "source density: {}",
                        attribute_value(element, "data-htmlswap-density")
                            .unwrap_or_else(|| density.as_str())
                    ),
                    attribute_span(element, "data-htmlswap-density").or(element.span),
                    SourceMappingKind::Attribute,
                ));
            }
            for extra in &semantics.extras {
                comments.push(MappedComment::new(
                    format!("source semantic {}: {}", extra.axis, extra.value),
                    extra.span.or(element.span),
                    SourceMappingKind::Attribute,
                ));
            }
        }

        if let Some(hints) = &element.source_intent {
            if let Some(key) = &hints.key {
                comments.push(MappedComment::new(
                    format!("source key: {key}"),
                    attribute_span(element, "data-htmlswap-key").or(element.span),
                    SourceMappingKind::Attribute,
                ));
            }
            if let Some(state_id) = &hints.state_id {
                comments.push(MappedComment::new(
                    format!("source state id: {state_id}"),
                    attribute_span(element, "data-htmlswap-state").or(element.span),
                    SourceMappingKind::Attribute,
                ));
            }
            if let Some(component) = &hints.component {
                comments.push(MappedComment::new(
                    format!(
                        "source component: {}",
                        attribute_value(element, "data-htmlswap-component")
                            .unwrap_or_else(|| component.as_str())
                    ),
                    attribute_span(element, "data-htmlswap-component").or(element.span),
                    SourceMappingKind::Attribute,
                ));
            }
            if let Some(source) = &hints.component_source {
                comments.push(MappedComment::new(
                    format!("source component source: {source}"),
                    attribute_span(element, "data-htmlswap-component-source").or(element.span),
                    SourceMappingKind::Attribute,
                ));
            }
            if let Some(slot) = &hints.slot {
                comments.push(MappedComment::new(
                    format!(
                        "source slot: {}",
                        attribute_value(element, "data-htmlswap-slot")
                            .unwrap_or_else(|| slot.as_str())
                    ),
                    attribute_span(element, "data-htmlswap-slot").or(element.span),
                    SourceMappingKind::Attribute,
                ));
            }
            if let Some(child_strategy) = &hints.child_strategy {
                comments.push(MappedComment::new(
                    format!("source child strategy: {child_strategy}"),
                    attribute_span(element, "data-htmlswap-children").or(element.span),
                    SourceMappingKind::Attribute,
                ));
            }
            for prop in &hints.props {
                comments.push(MappedComment::new(
                    format!("source prop {}={:?}", prop.name, prop.value),
                    prop.span.or(element.span),
                    SourceMappingKind::Attribute,
                ));
            }
        }

        if let Some(state) = &element.state
            && state.owner != RenderStateOwner::Target
            && let Some(span) = attribute_span(element, "data-htmlswap-state-owner")
        {
            comments.push(MappedComment::new(
                format!("source state owner: {:?}", state.owner).to_ascii_lowercase(),
                Some(span),
                SourceMappingKind::Attribute,
            ));
        }

        for attribute in &element.attributes {
            if self.attribute_is_handled(attribute.name.as_str(), spec) {
                continue;
            }
            if attribute.name == "href" {
                comments.push(MappedComment::new(
                    format!("source prop href={:?}", attribute.value),
                    attribute.span.or(element.span),
                    SourceMappingKind::Attribute,
                ));
                continue;
            }

            if attribute.value.is_empty() {
                comments.push(MappedComment::new(
                    format!("unmapped attribute: {}", attribute.name),
                    attribute.span.or(element.span),
                    SourceMappingKind::Attribute,
                ));
            } else {
                comments.push(MappedComment::new(
                    format!(
                        "unmapped attribute: {}={:?}",
                        attribute.name, attribute.value
                    ),
                    attribute.span.or(element.span),
                    SourceMappingKind::Attribute,
                ));
            }
        }

        comments
    }

    fn source_html_comment(&self, span: Option<Span>) -> Option<String> {
        let source = self.sources?.source_text(span?)?;
        source_html_start_tag(source).map(one_line)
    }

    fn attribute_is_handled(&self, attribute: &str, spec: &GpuiElementSpec) -> bool {
        matches!(attribute, "id" | "title")
            || matches!(attribute, "role" | "tabindex" | "autofocus" | "hidden")
            || attribute.starts_with("aria-")
            || attribute.starts_with("data-htmlswap-")
            || (attribute == "placeholder" && spec.children == ChildEmission::TextInputPlaceholder)
            || spec.consumes_attribute(attribute)
    }

    fn generated_id(&mut self, prefix: &str) -> String {
        let id = format!("htmlswap_{prefix}_{}", self.next_id);
        self.next_id += 1;
        id
    }

    fn warn(&mut self, message: impl Into<String>, span: Option<Span>) {
        self.cx.push(Diagnostic::warning(message, span));
    }

    fn mark_layer_used(&mut self, layer: &'static str) {
        self.used_layer_ids.insert(layer);
    }

    fn mark_source(
        &mut self,
        value: impl Into<String>,
        span: Option<Span>,
        kind: SourceMappingKind,
    ) -> String {
        let value = value.into();
        let Some(original) = span else {
            return value;
        };

        let id = self.next_source_marker;
        self.next_source_marker += 1;
        self.source_markers
            .push(GeneratedSourceMarker { id, original, kind });
        format!(
            "{SOURCE_MAP_START}{id}{SOURCE_MAP_CLOSE}{value}{SOURCE_MAP_END}{id}{SOURCE_MAP_CLOSE}"
        )
    }

    fn mark_line_comment_source(
        &mut self,
        indent: &str,
        value: impl Into<String>,
        span: Option<Span>,
        kind: SourceMappingKind,
    ) -> String {
        let value = value.into();
        let Some(original) = span else {
            return format!("{indent}// {value}");
        };

        let id = self.next_source_marker;
        self.next_source_marker += 1;
        self.source_markers
            .push(GeneratedSourceMarker { id, original, kind });
        format!(
            "{indent}// {SOURCE_MAP_START}{id}{SOURCE_MAP_CLOSE}{value}{SOURCE_MAP_END}{id}{SOURCE_MAP_CLOSE}"
        )
    }

    fn gpui_style_method(&self, style: &StyleDeclaration) -> Option<String> {
        if matches!(style.property, StyleProperty::Custom(_)) {
            return None;
        }

        match &style.value {
            StyleValue::Token(token) => self
                .gpui_token_style_method(&style.property, token)
                .or_else(|| {
                    self.literal_value_for_token(token).and_then(|value| {
                        gpui_style_method_for_value(&style.property, value.as_str())
                    })
                }),
            StyleValue::Raw(_) if border_width_method(&style.property).is_some() => self
                .gpui_border_method_with_tokens(style)
                .or_else(|| gpui_style_method_for_value(&style.property, style.value.as_str())),
            StyleValue::Raw(_)
                if matches!(
                    style.property,
                    StyleProperty::Background | StyleProperty::BackgroundImage
                ) =>
            {
                self.gpui_background_method_with_tokens(style)
                    .or_else(|| gpui_style_method_for_value(&style.property, style.value.as_str()))
            }
            StyleValue::Raw(_) => self
                .literal_text_with_tokens(style)
                .and_then(|value| gpui_style_method_for_value(&style.property, &value))
                .or_else(|| gpui_style_method_for_value(&style.property, style.value.as_str())),
            _ => gpui_style_method_for_value(&style.property, style.value.as_str()),
        }
    }

    fn gpui_style_variant_method(&self, variant: &RenderStyleVariant) -> Option<String> {
        let method_name = dynamic_style_method_name(&variant.conditions)?;
        let methods = variant
            .declarations
            .iter()
            .map(|declaration| self.gpui_style_method(declaration))
            .collect::<Option<Vec<_>>>()?
            .into_iter()
            .filter(|method| !method.is_empty())
            .collect::<Vec<_>>();

        (!methods.is_empty()).then(|| {
            format!(
                ".{method_name}(|this| {})",
                chain_style_methods("this", &methods)
            )
        })
    }

    fn gpui_token_style_method(
        &self,
        property: &StyleProperty,
        token: &StyleToken,
    ) -> Option<String> {
        let binding = self.theme_binding_for_property(property, token)?;
        gpui_style_method_for_theme_value(property, &binding.local_name, binding.kind)
    }

    fn gpui_border_method_with_tokens(&self, style: &StyleDeclaration) -> Option<String> {
        let border_method = border_width_method(&style.property)?;
        let value = style.value.as_str();
        let mut width = None;
        let mut color = None;
        let dashed = value
            .split_whitespace()
            .any(|part| part.eq_ignore_ascii_case("dashed"));

        for part in value
            .split_whitespace()
            .filter(|part| !part.contains("var("))
        {
            if width.is_none() {
                width = parse_length(part, LengthOptions::absolute());
            }
            if color.is_none() {
                color = parse_color(part);
            }
        }

        if color.is_none() {
            color = style
                .value
                .tokens_with_span(style.span)
                .iter()
                .find_map(|token| {
                    self.theme_color_expression_for_role(token, ThemeBindingRole::Border)
                });
        }

        let mut methods = Vec::new();
        if let Some(width) = width {
            methods.push(format!(".{border_method}({width})"));
        }
        if dashed {
            methods.push(".border_dashed()".to_owned());
        }
        if let Some(color) = color {
            methods.push(format!(".border_color({color})"));
        }
        join_style_methods(methods)
    }

    fn gpui_background_method_with_tokens(&self, style: &StyleDeclaration) -> Option<String> {
        parse_linear_gradient_with_color(style.value.as_str(), |value| {
            self.theme_or_literal_color_expression(style, value)
        })
        .map(|gradient| format!(".bg({gradient})"))
    }

    fn theme_or_literal_color_expression(
        &self,
        style: &StyleDeclaration,
        value: &str,
    ) -> Option<String> {
        parse_color(value).or_else(|| {
            let value = value.trim();
            style
                .value
                .tokens_with_span(style.span)
                .into_iter()
                .find(|token| token.raw.as_str() == value)
                .and_then(|token| {
                    self.theme_color_expression_for_role(&token, ThemeBindingRole::Surface)
                })
        })
    }

    fn theme_binding_for_property(
        &self,
        property: &StyleProperty,
        token: &StyleToken,
    ) -> Option<&ThemeBinding> {
        let role = theme_binding_role_for_property(property)?;
        self.theme_bindings
            .get(&ThemeBindingKey::new(token.name.clone(), role))
    }

    fn theme_color_expression_for_role(
        &self,
        token: &StyleToken,
        role: ThemeBindingRole,
    ) -> Option<String> {
        if let Some(binding) = self
            .theme_bindings
            .get(&ThemeBindingKey::new(token.name.clone(), role))
            && binding.kind == ThemeBindingKind::Color
        {
            return Some(binding.local_name.clone());
        }

        let value = self.literal_value_for_token(token)?;
        parse_color(value.as_str())
    }

    fn literal_value_for_token(&self, token: &StyleToken) -> Option<StyleValue> {
        let mut seen = BTreeSet::new();
        self.literal_value_for_token_inner(token, &mut seen)
    }

    fn literal_text_with_tokens(&self, style: &StyleDeclaration) -> Option<String> {
        let mut value = style.value.as_str().to_owned();
        let mut changed = false;
        for token in style.value.tokens_with_span(style.span) {
            let Some(literal) = self.literal_value_for_token(&token) else {
                continue;
            };
            let replaced = value.replace(token.raw.as_str(), literal.as_str());
            changed |= replaced != value;
            value = replaced;
        }
        changed.then_some(value)
    }

    fn literal_value_for_token_inner(
        &self,
        token: &StyleToken,
        seen: &mut BTreeSet<CompactString>,
    ) -> Option<StyleValue> {
        if !seen.insert(token.name.clone()) {
            return None;
        }

        if let Some(plan_token) = self.theme_plan.token(&token.name)
            && let Some(value) = plan_token
                .latest_root_value()
                .or_else(|| plan_token.latest_unconditional_value())
            && let Some(value) = self.resolve_literal_style_value(&value.value, seen)
        {
            return Some(value);
        }

        token
            .fallback
            .as_deref()
            .and_then(|fallback| self.resolve_literal_style_value(fallback, seen))
    }

    fn resolve_literal_style_value(
        &self,
        value: &StyleValue,
        seen: &mut BTreeSet<CompactString>,
    ) -> Option<StyleValue> {
        match value {
            StyleValue::Token(token) => self.literal_value_for_token_inner(token, seen),
            _ => Some(value.clone()),
        }
    }
}

fn stacked_child_nodes(nodes: &[RenderNode]) -> Vec<StackedChild<'_>> {
    let mut indexed = nodes
        .iter()
        .enumerate()
        .map(|(source_index, node)| (source_index, node, planned_z_index_for_node(node)))
        .collect::<Vec<_>>();
    let has_z_index = indexed.iter().any(|(_, _, z_index)| z_index.is_some());
    let all_element_like = indexed
        .iter()
        .all(|(_, node, _)| matches!(node, RenderNode::Element(_)));

    if !has_z_index || !all_element_like {
        return indexed
            .into_iter()
            .enumerate()
            .map(
                |(paint_index, (source_index, node, z_index))| StackedChild {
                    node,
                    source_index,
                    paint_index,
                    z_index,
                    emit_order_hint: false,
                },
            )
            .collect();
    }

    let can_reorder_for_paint = indexed
        .iter()
        .all(|(_, node, z_index)| z_index.is_none() || node_z_index_is_out_of_flow(node));
    if !can_reorder_for_paint {
        return indexed
            .into_iter()
            .enumerate()
            .map(
                |(paint_index, (source_index, node, z_index))| StackedChild {
                    node,
                    source_index,
                    paint_index,
                    z_index,
                    emit_order_hint: true,
                },
            )
            .collect();
    }

    indexed.sort_by_key(|(source_index, _, z_index)| (z_index.unwrap_or(0), *source_index));
    indexed
        .into_iter()
        .enumerate()
        .map(
            |(paint_index, (source_index, node, z_index))| StackedChild {
                node,
                source_index,
                paint_index,
                z_index,
                emit_order_hint: true,
            },
        )
        .collect()
}

fn node_span(node: &RenderNode) -> Option<Span> {
    match node {
        RenderNode::Element(element) => element.span,
        RenderNode::Text(text) => text.span,
        RenderNode::Raw(raw) => raw.span,
    }
}

fn control_flow_node(node: &RenderNode) -> Option<&RenderControlFlowKind> {
    let RenderNode::Element(element) = node else {
        return None;
    };
    element
        .control_flow
        .as_ref()
        .map(|control_flow| &control_flow.kind)
}

fn planned_z_index_for_node(node: &RenderNode) -> Option<i32> {
    let RenderNode::Element(element) = node else {
        return None;
    };

    planned_z_index_for_element(element)
        .or_else(|| planned_z_index_for_transparent_wrapper(element))
}

fn node_z_index_is_out_of_flow(node: &RenderNode) -> bool {
    let RenderNode::Element(element) = node else {
        return false;
    };

    element_has_out_of_flow_z_index_context(element)
        || transparent_wrapper_z_index_is_out_of_flow(element)
}

fn transparent_wrapper_z_index_is_out_of_flow(element: &RenderElement) -> bool {
    let Some(control_flow) = &element.control_flow else {
        return false;
    };
    if control_flow.host != RenderControlFlowHost::Wrapper {
        return false;
    }
    let [child] = element.children.as_slice() else {
        return false;
    };
    node_z_index_is_out_of_flow(child)
}

fn planned_z_index_for_transparent_wrapper(element: &RenderElement) -> Option<i32> {
    let Some(control_flow) = &element.control_flow else {
        return None;
    };
    if control_flow.host != RenderControlFlowHost::Wrapper {
        return None;
    }
    let [child] = element.children.as_slice() else {
        return None;
    };
    planned_z_index_for_node(child)
}

fn planned_z_index_for_element(element: &RenderElement) -> Option<i32> {
    if !element_has_effective_z_index_context(element) {
        return None;
    }

    element.styles.iter().find_map(style_z_index)
}

fn element_has_effective_z_index_context(element: &RenderElement) -> bool {
    element.styles.iter().any(|style| {
        style.property == StyleProperty::Position
            && !matches!(
                normalize_css_keyword(style.value.as_str()).as_str(),
                "" | "static"
            )
    })
}

fn element_has_out_of_flow_z_index_context(element: &RenderElement) -> bool {
    element.styles.iter().any(|style| {
        style.property == StyleProperty::Position
            && matches!(
                normalize_css_keyword(style.value.as_str()).as_str(),
                "absolute" | "fixed"
            )
    })
}

fn style_z_index(style: &StyleDeclaration) -> Option<i32> {
    if style.property != StyleProperty::ZIndex {
        return None;
    }
    parse_z_index(style.value.as_str())
}

fn parse_z_index(value: &str) -> Option<i32> {
    let trimmed = value.trim();
    if trimmed.contains('.') {
        return None;
    }
    trimmed.parse::<i32>().ok()
}

fn style_is_consumed_by_gpui_planner(element: &RenderElement, style: &StyleDeclaration) -> bool {
    if style.property != StyleProperty::ZIndex {
        return false;
    }

    element_has_effective_z_index_context(element)
        && (normalize_css_keyword(style.value.as_str()) == "auto" || style_z_index(style).is_some())
}

fn style_is_consumed_by_material_icon(style: &StyleDeclaration) -> bool {
    matches!(
        style.property,
        StyleProperty::FontFamily
            | StyleProperty::FontWeight
            | StyleProperty::FontStyle
            | StyleProperty::FontVariationSettings
            | StyleProperty::LineHeight
            | StyleProperty::LetterSpacing
            | StyleProperty::TextTransform
            | StyleProperty::WhiteSpace
            | StyleProperty::VerticalAlign
            | StyleProperty::Direction
            | StyleProperty::UserSelect
            | StyleProperty::WebkitFontSmoothing
    )
}

pub(crate) fn is_material_symbol_element(element: &RenderElement) -> bool {
    element.source_tag == "span"
        && element.classes.iter().any(|class| {
            matches!(
                class.as_str(),
                "material-icons" | "material-symbols-outlined" | "ms"
            )
        })
}

fn source_control_flow_comment(element: &RenderElement) -> Option<MappedComment> {
    let control_flow = element.control_flow.as_ref()?;
    let kind = match &control_flow.kind {
        RenderControlFlowKind::For => "for".to_owned(),
        RenderControlFlowKind::If => "if".to_owned(),
        RenderControlFlowKind::ElseIf => "else-if".to_owned(),
        RenderControlFlowKind::Else => "else".to_owned(),
        RenderControlFlowKind::Switch => "switch".to_owned(),
        RenderControlFlowKind::Case => "case".to_owned(),
        RenderControlFlowKind::Default => "default".to_owned(),
    };
    let mut parts = vec![format!("source control-flow: {kind}")];
    if let Some(expression) = &control_flow.expression {
        parts.push(format!("expr={expression}"));
    }
    if let Some(binding) = &control_flow.binding {
        parts.push(format!("item={}", binding.name));
    }
    if let Some(placeholder) = &control_flow.placeholder {
        parts.push(format!("placeholder={placeholder}"));
    }

    Some(MappedComment::new(
        parts.join(", "),
        control_flow.span.or(element.span),
        SourceMappingKind::Attribute,
    ))
}

fn style_should_preserve_original_css(style: &StyleDeclaration) -> bool {
    let value = normalize_css_keyword(style.value.as_str());
    match style.property {
        StyleProperty::Display => matches!(
            value.as_str(),
            "inline" | "inline-block" | "inline-flex" | "inline-grid" | "contents"
        ),
        StyleProperty::GridTemplateColumns => {
            value.starts_with("repeat(auto-fill,")
                || value.starts_with("repeat(auto-fit,")
                || value.contains("minmax(")
        }
        StyleProperty::Background | StyleProperty::BackgroundImage => {
            background_needs_original_css_preservation(style.value.as_str())
        }
        StyleProperty::BoxShadow => value
            .split_whitespace()
            .any(|part| part.eq_ignore_ascii_case("inset")),
        StyleProperty::Width
        | StyleProperty::Height
        | StyleProperty::MinWidth
        | StyleProperty::MinHeight
        | StyleProperty::MaxWidth
        | StyleProperty::MaxHeight
        | StyleProperty::FlexBasis => is_intrinsic_size_keyword(&value),
        StyleProperty::Top | StyleProperty::Right | StyleProperty::Bottom | StyleProperty::Left => {
            value.contains("calc(")
        }
        StyleProperty::Position => matches!(value.as_str(), "fixed" | "sticky"),
        StyleProperty::TransformOrigin => true,
        StyleProperty::WhiteSpace => !matches!(value.as_str(), "normal" | "nowrap"),
        _ => false,
    }
}

fn background_needs_original_css_preservation(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("radial-gradient(")
        || lower.contains("repeating-linear-gradient(")
        || lower.contains("repeating-radial-gradient(")
        || lower.contains("conic-gradient(")
        || lower.contains("url(")
        || split_top_level_commas(value).len() > 1
}

fn preserved_style_comment(
    prefix: &'static str,
    style: &StyleDeclaration,
    fallback_span: Option<Span>,
) -> MappedComment {
    MappedComment::new(
        format!("{prefix}: {}: {}", style.property, style.value),
        style.span.or(fallback_span),
        SourceMappingKind::Style,
    )
}

fn gpui_style_method_for_value(property: &StyleProperty, value: &str) -> Option<String> {
    match property {
        StyleProperty::Display => match normalize_css_keyword(value).as_str() {
            "block" => Some(".block()".to_owned()),
            "flex" | "inline-flex" => Some(".flex()".to_owned()),
            "grid" | "inline-grid" => Some(".grid()".to_owned()),
            "none" => Some(".hidden()".to_owned()),
            "inline" | "inline-block" | "contents" => Some(".block()".to_owned()),
            _ => None,
        },
        StyleProperty::Color => parse_noop_keyword(value, &["inherit"])
            .or_else(|| parse_color(value).map(|color| format!(".text_color({color})"))),
        StyleProperty::Background | StyleProperty::BackgroundColor => parse_background(value),
        StyleProperty::BackgroundClip => {
            parse_noop_keyword(value, &["border-box", "padding-box", "content-box", "text"])
        }
        StyleProperty::BackgroundImage => parse_background_image(value),
        StyleProperty::BackgroundSize => parse_noop_keyword(value, &["auto"]),
        StyleProperty::BoxSizing => parse_noop_keyword(value, &["border-box", "content-box"]),
        StyleProperty::BorderColor => {
            parse_color(value).map(|color| format!(".border_color({color})"))
        }
        StyleProperty::Gap => {
            parse_length(value, LengthOptions::relative()).map(|value| format!(".gap({value})"))
        }
        StyleProperty::Padding => parse_box_edge_methods(
            value,
            LengthOptions::relative(),
            "p",
            ["pt", "pr", "pb", "pl"],
        ),
        StyleProperty::PaddingTop => {
            parse_length(value, LengthOptions::relative()).map(|value| format!(".pt({value})"))
        }
        StyleProperty::PaddingRight => {
            parse_length(value, LengthOptions::relative()).map(|value| format!(".pr({value})"))
        }
        StyleProperty::PaddingBottom => {
            parse_length(value, LengthOptions::relative()).map(|value| format!(".pb({value})"))
        }
        StyleProperty::PaddingLeft => {
            parse_length(value, LengthOptions::relative()).map(|value| format!(".pl({value})"))
        }
        StyleProperty::Margin => parse_box_edge_methods(
            value,
            LengthOptions::auto_relative(),
            "m",
            ["mt", "mr", "mb", "ml"],
        ),
        StyleProperty::MarginTop => {
            parse_length(value, LengthOptions::auto_relative()).map(|value| format!(".mt({value})"))
        }
        StyleProperty::MarginRight => {
            parse_length(value, LengthOptions::auto_relative()).map(|value| format!(".mr({value})"))
        }
        StyleProperty::MarginBottom => {
            parse_length(value, LengthOptions::auto_relative()).map(|value| format!(".mb({value})"))
        }
        StyleProperty::MarginLeft => {
            parse_length(value, LengthOptions::auto_relative()).map(|value| format!(".ml({value})"))
        }
        StyleProperty::Width => parse_dimension(value, "w", Some("w_full")),
        StyleProperty::Height => parse_dimension(value, "h", Some("h_full")),
        StyleProperty::MinWidth => parse_length(value, LengthOptions::auto_relative())
            .map(|value| format!(".min_w({value})")),
        StyleProperty::MinHeight => parse_length(value, LengthOptions::auto_relative())
            .map(|value| format!(".min_h({value})")),
        StyleProperty::MaxWidth => parse_length(value, LengthOptions::auto_relative())
            .map(|value| format!(".max_w({value})")),
        StyleProperty::MaxHeight => parse_length(value, LengthOptions::auto_relative())
            .map(|value| format!(".max_h({value})")),
        StyleProperty::AspectRatio => parse_aspect_ratio(value).map(|ratio| {
            format!(".map(|mut this| {{ this.style().aspect_ratio = Some({ratio}); this }})")
        }),
        StyleProperty::Overflow => parse_overflow(value, OverflowAxis::Both),
        StyleProperty::OverflowX => parse_overflow(value, OverflowAxis::X),
        StyleProperty::OverflowY => parse_overflow(value, OverflowAxis::Y),
        StyleProperty::Position => parse_position(value),
        StyleProperty::Top => parse_position_offset(value, "top", "mt"),
        StyleProperty::Right => parse_position_offset(value, "right", "mr"),
        StyleProperty::Bottom => parse_position_offset(value, "bottom", "mb"),
        StyleProperty::Left => parse_position_offset(value, "left", "ml"),
        StyleProperty::Inset => parse_box_edge_methods(
            value,
            LengthOptions::auto_relative(),
            "inset",
            ["top", "right", "bottom", "left"],
        ),
        StyleProperty::Flex => parse_flex(value),
        StyleProperty::FlexDirection => parse_flex_direction(value),
        StyleProperty::FlexWrap => parse_flex_wrap(value),
        StyleProperty::FlexBasis => parse_length(value, LengthOptions::auto_relative())
            .map(|value| format!(".flex_basis({value})")),
        StyleProperty::FlexGrow => parse_flex_grow(value),
        StyleProperty::FlexShrink => parse_flex_shrink(value),
        StyleProperty::GridTemplateColumns => parse_grid_template_columns(value),
        StyleProperty::AlignItems => parse_align_items(value),
        StyleProperty::AlignSelf => parse_align_self(value),
        StyleProperty::AlignContent => parse_align_content(value),
        StyleProperty::JustifyContent => parse_justify_content(value),
        StyleProperty::Border
        | StyleProperty::BorderTop
        | StyleProperty::BorderRight
        | StyleProperty::BorderBottom
        | StyleProperty::BorderLeft => {
            border_width_method(property).and_then(|method| parse_border_with_method(value, method))
        }
        StyleProperty::BorderWidth => parse_box_edge_methods(
            value,
            LengthOptions::absolute(),
            "border",
            ["border_t", "border_r", "border_b", "border_l"],
        ),
        StyleProperty::BorderStyle
        | StyleProperty::BorderTopStyle
        | StyleProperty::BorderRightStyle
        | StyleProperty::BorderBottomStyle
        | StyleProperty::BorderLeftStyle => parse_border_style(value),
        StyleProperty::BorderRadius => parse_border_radius(value),
        StyleProperty::BoxShadow => parse_box_shadow(value),
        StyleProperty::FontSize => parse_length(value, LengthOptions::absolute())
            .map(|value| format!(".text_size({value})")),
        StyleProperty::FontWeight => {
            parse_font_weight(value).map(|weight| format!(".font_weight({weight})"))
        }
        StyleProperty::FontFamily => {
            parse_font_family(value).map(|family| format!(".font_family({})", rust_string(&family)))
        }
        StyleProperty::FontStyle => parse_font_style(value),
        StyleProperty::LineHeight => {
            parse_line_height(value).map(|value| format!(".line_height({value})"))
        }
        StyleProperty::TextAlign => parse_text_align(value),
        StyleProperty::TextDecoration => parse_text_decoration(value),
        StyleProperty::TextOverflow => parse_text_overflow(value),
        StyleProperty::TextTransform => text_transform_from_css(value).map(|_| String::new()),
        StyleProperty::WordBreak => parse_noop_keyword(value, &["normal"]),
        StyleProperty::WhiteSpace => parse_white_space(value),
        StyleProperty::Cursor => parse_cursor(value),
        StyleProperty::Opacity => parse_opacity(value),
        StyleProperty::Outline => parse_noop_keyword(value, &["none", "0"]),
        StyleProperty::Direction => parse_noop_keyword(value, &["ltr"]),
        StyleProperty::UserSelect => parse_noop_keyword(value, &["none"]),
        StyleProperty::VerticalAlign => parse_noop_keyword(value, &["baseline", "middle"]),
        StyleProperty::LetterSpacing => parse_zero_or_normal_noop(value),
        StyleProperty::FontVariationSettings => parse_noop_keyword(value, &["normal"]),
        StyleProperty::PointerEvents => parse_noop_keyword(value, &["auto"]),
        StyleProperty::TransformOrigin => Some(String::new()),
        StyleProperty::WebkitFontSmoothing => {
            parse_noop_keyword(value, &["auto", "antialiased", "subpixel-antialiased"])
        }
        _ => None,
    }
}

fn gpui_style_method_for_theme_value(
    property: &StyleProperty,
    expression: &str,
    kind: ThemeBindingKind,
) -> Option<String> {
    match (property, kind) {
        (StyleProperty::Color, ThemeBindingKind::Color) => {
            Some(format!(".text_color({expression})"))
        }
        (StyleProperty::Background | StyleProperty::BackgroundColor, ThemeBindingKind::Color) => {
            Some(format!(".bg({expression})"))
        }
        (
            StyleProperty::BorderColor
            | StyleProperty::Border
            | StyleProperty::BorderTop
            | StyleProperty::BorderRight
            | StyleProperty::BorderBottom
            | StyleProperty::BorderLeft,
            ThemeBindingKind::Color,
        ) => Some(format!(".border_color({expression})")),
        (
            StyleProperty::Border
            | StyleProperty::BorderTop
            | StyleProperty::BorderRight
            | StyleProperty::BorderBottom
            | StyleProperty::BorderLeft,
            ThemeBindingKind::Length,
        ) => border_width_method(property).map(|method| format!(".{method}({expression})")),
        (StyleProperty::Gap, ThemeBindingKind::Length) => Some(format!(".gap({expression})")),
        (StyleProperty::Padding, ThemeBindingKind::Length) => Some(format!(".p({expression})")),
        (StyleProperty::PaddingTop, ThemeBindingKind::Length) => Some(format!(".pt({expression})")),
        (StyleProperty::PaddingRight, ThemeBindingKind::Length) => {
            Some(format!(".pr({expression})"))
        }
        (StyleProperty::PaddingBottom, ThemeBindingKind::Length) => {
            Some(format!(".pb({expression})"))
        }
        (StyleProperty::PaddingLeft, ThemeBindingKind::Length) => {
            Some(format!(".pl({expression})"))
        }
        (StyleProperty::Margin, ThemeBindingKind::Length) => Some(format!(".m({expression})")),
        (StyleProperty::MarginTop, ThemeBindingKind::Length) => Some(format!(".mt({expression})")),
        (StyleProperty::MarginRight, ThemeBindingKind::Length) => {
            Some(format!(".mr({expression})"))
        }
        (StyleProperty::MarginBottom, ThemeBindingKind::Length) => {
            Some(format!(".mb({expression})"))
        }
        (StyleProperty::MarginLeft, ThemeBindingKind::Length) => Some(format!(".ml({expression})")),
        (StyleProperty::Width, ThemeBindingKind::Length) => Some(format!(".w({expression})")),
        (StyleProperty::Height, ThemeBindingKind::Length) => Some(format!(".h({expression})")),
        (StyleProperty::MinWidth, ThemeBindingKind::Length) => {
            Some(format!(".min_w({expression})"))
        }
        (StyleProperty::MinHeight, ThemeBindingKind::Length) => {
            Some(format!(".min_h({expression})"))
        }
        (StyleProperty::MaxWidth, ThemeBindingKind::Length) => {
            Some(format!(".max_w({expression})"))
        }
        (StyleProperty::MaxHeight, ThemeBindingKind::Length) => {
            Some(format!(".max_h({expression})"))
        }
        (StyleProperty::BorderRadius, ThemeBindingKind::Length) => {
            Some(format!(".rounded({expression})"))
        }
        (StyleProperty::FontSize, ThemeBindingKind::Length) => {
            Some(format!(".text_size({expression})"))
        }
        (StyleProperty::LineHeight, ThemeBindingKind::Length) => {
            Some(format!(".line_height({expression})"))
        }
        _ => None,
    }
}

fn theme_binding_role_for_property(property: &StyleProperty) -> Option<ThemeBindingRole> {
    match property {
        StyleProperty::Color => Some(ThemeBindingRole::Text),
        StyleProperty::Background
        | StyleProperty::BackgroundColor
        | StyleProperty::BackgroundImage => Some(ThemeBindingRole::Surface),
        StyleProperty::BorderColor
        | StyleProperty::Border
        | StyleProperty::BorderTop
        | StyleProperty::BorderRight
        | StyleProperty::BorderBottom
        | StyleProperty::BorderLeft => Some(ThemeBindingRole::Border),
        StyleProperty::Gap
        | StyleProperty::Padding
        | StyleProperty::PaddingTop
        | StyleProperty::PaddingRight
        | StyleProperty::PaddingBottom
        | StyleProperty::PaddingLeft
        | StyleProperty::Margin
        | StyleProperty::MarginTop
        | StyleProperty::MarginRight
        | StyleProperty::MarginBottom
        | StyleProperty::MarginLeft
        | StyleProperty::Width
        | StyleProperty::Height
        | StyleProperty::MinWidth
        | StyleProperty::MinHeight
        | StyleProperty::MaxWidth
        | StyleProperty::MaxHeight
        | StyleProperty::Top
        | StyleProperty::Right
        | StyleProperty::Bottom
        | StyleProperty::Left
        | StyleProperty::Inset
        | StyleProperty::FlexBasis
        | StyleProperty::BorderWidth
        | StyleProperty::BorderRadius
        | StyleProperty::FontSize
        | StyleProperty::LineHeight => Some(ThemeBindingRole::Length),
        _ => None,
    }
}

fn theme_bindings_for_plan(
    plan: &RenderPlan,
    emission: ThemeEmission,
) -> BTreeMap<ThemeBindingKey, ThemeBinding> {
    if emission == ThemeEmission::Literal {
        return BTreeMap::new();
    }

    let mut bindings = BTreeMap::new();
    let mut used_names = BTreeSet::new();
    collect_theme_bindings_from_root(plan, &mut bindings, &mut used_names);
    for node in &plan.nodes {
        collect_theme_bindings_from_node(plan, node, &mut bindings, &mut used_names);
    }
    bindings
}

fn collect_theme_bindings_from_root(
    plan: &RenderPlan,
    bindings: &mut BTreeMap<ThemeBindingKey, ThemeBinding>,
    used_names: &mut BTreeSet<String>,
) {
    for style in &plan.root.styles {
        collect_theme_binding_from_style(plan, style, bindings, used_names);
    }
    for variant in &plan.root.style_variants {
        for declaration in &variant.declarations {
            collect_theme_binding_from_style(plan, declaration, bindings, used_names);
        }
    }
}

fn collect_theme_bindings_from_node(
    plan: &RenderPlan,
    node: &RenderNode,
    bindings: &mut BTreeMap<ThemeBindingKey, ThemeBinding>,
    used_names: &mut BTreeSet<String>,
) {
    let RenderNode::Element(element) = node else {
        return;
    };

    for style in &element.styles {
        collect_theme_binding_from_style(plan, style, bindings, used_names);
    }
    for variant in &element.style_variants {
        for declaration in &variant.declarations {
            collect_theme_binding_from_style(plan, declaration, bindings, used_names);
        }
    }
    for pseudo in &element.pseudo_elements {
        for declaration in &pseudo.styles {
            collect_theme_binding_from_style(plan, declaration, bindings, used_names);
        }
    }
    for child in &element.children {
        collect_theme_bindings_from_node(plan, child, bindings, used_names);
    }
}

fn collect_theme_binding_from_style(
    plan: &RenderPlan,
    style: &StyleDeclaration,
    bindings: &mut BTreeMap<ThemeBindingKey, ThemeBinding>,
    used_names: &mut BTreeSet<String>,
) {
    if let StyleValue::Token(token) = &style.value {
        if border_width_method(&style.property).is_some() {
            add_theme_binding_for_token(
                plan,
                token,
                ThemeBindingRole::Border,
                bindings,
                used_names,
            );
            add_theme_binding_for_token(
                plan,
                token,
                ThemeBindingRole::Length,
                bindings,
                used_names,
            );
            return;
        }

        if let Some(role) = theme_binding_role_for_property(&style.property) {
            add_theme_binding_for_token(plan, token, role, bindings, used_names);
        }
        return;
    }

    if border_width_method(&style.property).is_some() {
        for token in style.value.tokens_with_span(style.span) {
            add_theme_binding_for_token(
                plan,
                &token,
                ThemeBindingRole::Border,
                bindings,
                used_names,
            );
        }
    }

    if matches!(
        style.property,
        StyleProperty::Background | StyleProperty::BackgroundColor | StyleProperty::BackgroundImage
    ) {
        for token in style.value.tokens_with_span(style.span) {
            add_theme_binding_for_token(
                plan,
                &token,
                ThemeBindingRole::Surface,
                bindings,
                used_names,
            );
        }
    }
}

fn add_theme_binding_for_token(
    plan: &RenderPlan,
    token: &StyleToken,
    role: ThemeBindingRole,
    bindings: &mut BTreeMap<ThemeBindingKey, ThemeBinding>,
    used_names: &mut BTreeSet<String>,
) {
    let key = ThemeBindingKey::new(token.name.clone(), role);
    if bindings.contains_key(&key) {
        return;
    }

    let literal_value = plan
        .theme
        .token(&token.name)
        .and_then(|theme_token| {
            theme_token
                .latest_root_value()
                .or_else(|| theme_token.latest_unconditional_value())
        })
        .map(|value| value.value.as_str());

    let Some(field) = gpui_component_theme_field_for_role(&token.name, role, literal_value) else {
        return;
    };
    if field.kind != role.kind() {
        return;
    }

    let local_name = unique_theme_binding_name(&token.name, field.name, used_names);
    let span = plan
        .theme
        .token(&token.name)
        .and_then(|theme_token| {
            theme_token
                .latest_root_value()
                .or_else(|| theme_token.latest_unconditional_value())
        })
        .and_then(|value| value.span)
        .or(token.span);

    bindings.insert(
        key,
        ThemeBinding {
            local_name,
            field: field.name,
            kind: field.kind,
            span,
        },
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ThemeField {
    name: &'static str,
    kind: ThemeBindingKind,
}

fn gpui_component_theme_field_for_role(
    token_name: &str,
    role: ThemeBindingRole,
    literal_value: Option<&str>,
) -> Option<ThemeField> {
    let field = gpui_component_theme_field(token_name)?;
    if role != ThemeBindingRole::Surface {
        return Some(field);
    }

    if !theme_field_is_hued(field.name) {
        return Some(field);
    }

    let Some(value) = literal_value else {
        return Some(field);
    };

    if color_is_low_saturation_dark(value) {
        return Some(ThemeField {
            name: "background",
            kind: ThemeBindingKind::Color,
        });
    }

    Some(field)
}

fn gpui_component_theme_field(token_name: &str) -> Option<ThemeField> {
    let normalized = normalize_theme_token_name(token_name);
    let normalized = normalized
        .strip_prefix("color_")
        .or_else(|| normalized.strip_prefix("theme_"))
        .unwrap_or(&normalized);
    let field = match normalized {
        "bg" => "background",
        "fg" => "foreground",
        "border_color" => "border",
        "focus_ring" => "ring",
        "destructive" => "danger",
        "destructive_active" => "danger_active",
        "destructive_foreground" => "danger_foreground",
        "destructive_hover" => "danger_hover",
        "error" => "danger",
        "error_active" => "danger_active",
        "error_foreground" => "danger_foreground",
        "error_hover" => "danger_hover",
        other => other,
    };

    if matches!(
        field,
        "radius" | "radius_lg" | "font_size" | "mono_font_size" | "tile_grid_size" | "tile_radius"
    ) {
        return Some(ThemeField {
            name: length_theme_field(field)?,
            kind: ThemeBindingKind::Length,
        });
    }

    Some(ThemeField {
        name: color_theme_field(field)?,
        kind: ThemeBindingKind::Color,
    })
}

fn length_theme_field(field: &str) -> Option<&'static str> {
    match field {
        "radius" => Some("radius"),
        "radius_lg" => Some("radius_lg"),
        "font_size" => Some("font_size"),
        "mono_font_size" => Some("mono_font_size"),
        "tile_grid_size" => Some("tile_grid_size"),
        "tile_radius" => Some("tile_radius"),
        _ => None,
    }
}

fn color_theme_field(field: &str) -> Option<&'static str> {
    GPUI_COMPONENT_THEME_COLOR_FIELDS
        .iter()
        .copied()
        .find(|candidate| *candidate == field)
}

fn theme_field_is_hued(field: &str) -> bool {
    field == "accent"
        || field == "ring"
        || field == "primary"
        || field == "primary_active"
        || field == "primary_hover"
        || field == "link"
        || field == "link_active"
        || field == "link_hover"
        || field == "danger"
        || field == "danger_active"
        || field == "danger_hover"
        || field == "info"
        || field == "info_active"
        || field == "info_hover"
        || field == "success"
        || field == "success_active"
        || field == "success_hover"
        || field == "warning"
        || field == "warning_active"
        || field == "warning_hover"
        || field == "bullish"
        || field == "bearish"
        || field == "sidebar_primary"
        || field.starts_with("chart_")
        || field.starts_with("blue")
        || field.starts_with("cyan")
        || field.starts_with("magenta")
        || field.starts_with("red")
        || field.starts_with("green")
        || field.starts_with("yellow")
}

fn unique_theme_binding_name(
    token_name: &str,
    field: &str,
    used_names: &mut BTreeSet<String>,
) -> String {
    let seed = normalize_theme_token_name(token_name);
    let base = if seed.is_empty() {
        format!("htmlswap_theme_{field}")
    } else if seed == field || theme_seed_aliases_field(&seed, field) {
        format!("htmlswap_theme_{seed}")
    } else {
        format!("htmlswap_theme_{seed}_{field}")
    };
    let mut candidate = sanitize_rust_identifier(&base, "htmlswap_theme");
    if used_names.insert(candidate.clone()) {
        return candidate;
    }

    let mut suffix = 2;
    loop {
        candidate = format!("{base}_{suffix}");
        candidate = sanitize_rust_identifier(&candidate, "htmlswap_theme");
        if used_names.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
}

fn theme_seed_aliases_field(seed: &str, field: &str) -> bool {
    matches!(
        (seed, field),
        ("bg", "background") | ("fg", "foreground") | ("border_color", "border")
    )
}

fn normalize_theme_token_name(name: &str) -> String {
    let name = name.trim().trim_start_matches("--");
    let mut output = String::with_capacity(name.len());
    let mut previous_separator = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
            previous_separator = false;
        } else if !previous_separator {
            output.push('_');
            previous_separator = true;
        }
    }
    output.trim_matches('_').to_owned()
}

fn sanitize_rust_identifier(value: &str, fallback: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
        } else if !output.ends_with('_') {
            output.push('_');
        }
    }
    let output = output.trim_matches('_');
    let output = if output.is_empty() {
        fallback.to_owned()
    } else if output.as_bytes()[0].is_ascii_digit() {
        format!("{fallback}_{output}")
    } else {
        output.to_owned()
    };

    if is_rust_identifier(&output) {
        output
    } else {
        fallback.to_owned()
    }
}

const GPUI_COMPONENT_THEME_COLOR_FIELDS: &[&str] = &[
    "accent",
    "accent_foreground",
    "accordion",
    "accordion_hover",
    "background",
    "border",
    "group_box",
    "group_box_foreground",
    "caret",
    "chart_1",
    "chart_2",
    "chart_3",
    "chart_4",
    "chart_5",
    "danger",
    "danger_active",
    "danger_foreground",
    "danger_hover",
    "description_list_label",
    "description_list_label_foreground",
    "drag_border",
    "drop_target",
    "foreground",
    "info",
    "info_active",
    "info_foreground",
    "info_hover",
    "input",
    "link",
    "link_active",
    "link_hover",
    "list",
    "list_active",
    "list_active_border",
    "list_even",
    "list_head",
    "list_hover",
    "muted",
    "muted_foreground",
    "popover",
    "popover_foreground",
    "primary",
    "primary_active",
    "primary_foreground",
    "primary_hover",
    "progress_bar",
    "ring",
    "scrollbar",
    "scrollbar_thumb",
    "scrollbar_thumb_hover",
    "secondary",
    "secondary_active",
    "secondary_foreground",
    "secondary_hover",
    "selection",
    "sidebar",
    "sidebar_accent",
    "sidebar_accent_foreground",
    "sidebar_border",
    "sidebar_foreground",
    "sidebar_primary",
    "sidebar_primary_foreground",
    "skeleton",
    "slider_bar",
    "slider_thumb",
    "success",
    "success_active",
    "success_foreground",
    "success_hover",
    "bullish",
    "bearish",
    "switch",
    "switch_thumb",
    "tab",
    "tab_active",
    "tab_active_foreground",
    "tab_bar",
    "tab_bar_segmented",
    "tab_foreground",
    "table",
    "table_active",
    "table_active_border",
    "table_even",
    "table_head",
    "table_head_foreground",
    "table_hover",
    "table_row_border",
    "title_bar",
    "title_bar_border",
    "tiles",
    "warning",
    "warning_active",
    "warning_foreground",
    "warning_hover",
    "overlay",
    "window_border",
    "red",
    "red_light",
    "green",
    "green_light",
    "blue",
    "blue_light",
    "yellow",
    "yellow_light",
    "magenta",
    "magenta_light",
    "cyan",
    "cyan_light",
];

fn dynamic_style_method_name(conditions: &[RenderStyleCondition]) -> Option<&'static str> {
    let mut pseudo_classes = conditions.iter().filter_map(|condition| match condition {
        RenderStyleCondition::PseudoClass(value) => Some(value.as_str()),
        _ => None,
    });
    let value = pseudo_classes.next()?;
    if pseudo_classes.next().is_some()
        || conditions.iter().any(|condition| match condition {
            RenderStyleCondition::PseudoClass(_) => false,
            RenderStyleCondition::Media(query) => !interaction_media_allows(query, value),
            RenderStyleCondition::PseudoElement(_)
            | RenderStyleCondition::Supports(_)
            | RenderStyleCondition::Container(_)
            | RenderStyleCondition::StartingStyle
            | RenderStyleCondition::ActiveViewTransitionType(_)
            | RenderStyleCondition::ElementState { .. } => true,
        })
    {
        return None;
    }

    match value {
        "hover" => Some("hover"),
        "active" => Some("active"),
        // GPUI currently exposes one focus style hook. Treat `:focus-visible` as
        // that hook so keyboard-focus affordances remain functional instead of
        // being discarded by the adapter.
        "focus" | "focus-visible" => Some("focus"),
        _ => None,
    }
}

fn interaction_media_allows(query: &str, pseudo_class: &str) -> bool {
    let query = normalize_css_keyword(query);
    match pseudo_class {
        "hover" => matches!(
            query.as_str(),
            "(hover:hover)" | "(hover: hover)" | "(any-hover:hover)" | "(any-hover: hover)"
        ),
        "focus" | "focus-visible" => {
            matches!(query.as_str(), "(focus:focus)" | "(focus: focus)")
        }
        "active" => false,
        _ => false,
    }
}

fn style_variant_preservation_reason(variant: &RenderStyleVariant) -> String {
    if variant
        .conditions
        .iter()
        .any(|condition| matches!(condition, RenderStyleCondition::Media(_)))
    {
        return "media queries need a target runtime/layout policy".to_owned();
    }

    if variant
        .conditions
        .iter()
        .any(|condition| matches!(condition, RenderStyleCondition::Supports(_)))
    {
        return "supports queries need a target feature-gating policy".to_owned();
    }

    if variant
        .conditions
        .iter()
        .any(|condition| matches!(condition, RenderStyleCondition::Container(_)))
    {
        return "container queries need a target layout-container policy".to_owned();
    }

    if variant
        .conditions
        .iter()
        .any(|condition| matches!(condition, RenderStyleCondition::PseudoElement(_)))
    {
        return "pseudo-elements need target support for synthetic children".to_owned();
    }

    if variant.conditions.iter().any(|condition| {
        matches!(
            condition,
            RenderStyleCondition::PseudoClass(value)
                if value == "focus-within"
        )
    }) {
        return "this focus pseudo-class has no exact GPUI style hook".to_owned();
    }

    if dynamic_style_method_name(&variant.conditions).is_none() {
        return "combined or unsupported conditional states need adapter-specific planning"
            .to_owned();
    }

    "one or more declarations are not mapped to GPUI style methods".to_owned()
}

fn element_needs_generated_stateful_id(element: &RenderElement) -> bool {
    element.actions.iter().any(|action| {
        matches!(
            action.event.as_str(),
            "click"
                | "dblclick"
                | "doubleclick"
                | "mouseenter"
                | "mouseover"
                | "mouseleave"
                | "mousedown"
                | "mousemove"
                | "wheel"
                | "keydown"
                | "keyup"
                | "input"
                | "change"
                | "submit"
        )
    }) || element.styles.iter().any(style_needs_stateful_id)
        || element
            .style_variants
            .iter()
            .any(style_variant_needs_stateful_id)
}

fn title_bar_descendant_needs_mouse_down_stop(
    element: &RenderElement,
    spec: &GpuiElementSpec,
    scope: &RenderScope,
) -> bool {
    if !scope.inside_title_bar_drag_area || spec.children == ChildEmission::TitleBarChildren {
        return false;
    }

    matches!(element.role, UiRole::Button | UiRole::Link)
        || element.actions.iter().any(|action| {
            matches!(
                action.event.as_str(),
                "click" | "dblclick" | "doubleclick" | "mousedown" | "mouseup"
            )
        })
}

fn style_needs_stateful_id(style: &StyleDeclaration) -> bool {
    if !matches!(
        style.property,
        StyleProperty::Overflow | StyleProperty::OverflowX | StyleProperty::OverflowY
    ) {
        return false;
    }

    matches!(
        normalize_css_keyword(style.value.as_str()).as_str(),
        "auto" | "scroll"
    )
}

fn style_variant_needs_stateful_id(variant: &RenderStyleVariant) -> bool {
    matches!(
        dynamic_style_method_name(&variant.conditions),
        Some("active" | "focus")
    )
}

fn pseudo_element_emits_as_child(pseudo: &RenderPseudoElement, kind: &str) -> bool {
    pseudo.kind == kind && pseudo.conditions.is_empty()
}

fn pseudo_element_preservation_reason(
    pseudo: &RenderPseudoElement,
    spec: &GpuiElementSpec,
) -> String {
    if !matches!(
        spec.children,
        ChildEmission::Children | ChildEmission::TitleBarChildren | ChildEmission::FieldsetChildren
    ) {
        return "the selected target component consumes or reshapes children".to_owned();
    }

    if !matches!(pseudo.kind.as_str(), "before" | "after") {
        return "this pseudo-element is not representable as a GPUI child".to_owned();
    }

    if !pseudo.conditions.is_empty() {
        return "conditional pseudo-elements need a target runtime/layout policy".to_owned();
    }

    "this pseudo-element could not be emitted by the selected adapter".to_owned()
}

fn chain_style_methods(base: &str, methods: &[String]) -> String {
    let mut chained = base.to_owned();
    for method in methods {
        for line in method.lines() {
            chained.push_str(line.trim());
        }
    }
    chained
}

#[derive(Debug, Clone, Copy)]
struct LengthOptions {
    allow_auto: bool,
    allow_relative: bool,
}

impl LengthOptions {
    const fn absolute() -> Self {
        Self {
            allow_auto: false,
            allow_relative: false,
        }
    }

    const fn relative() -> Self {
        Self {
            allow_auto: false,
            allow_relative: true,
        }
    }

    const fn auto_relative() -> Self {
        Self {
            allow_auto: true,
            allow_relative: true,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum OverflowAxis {
    Both,
    X,
    Y,
}

fn normalize_css_keyword(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn parse_noop_keyword(value: &str, supported: &[&str]) -> Option<String> {
    let value = normalize_css_keyword(value);
    supported.contains(&value.as_str()).then(String::new)
}

fn parse_zero_or_normal_noop(value: &str) -> Option<String> {
    let value = normalize_css_keyword(value);
    if value == "normal" || is_zero_length(&value) {
        return Some(String::new());
    }

    None
}

fn parse_background(value: &str) -> Option<String> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none") {
        return Some(String::new());
    }

    if let Some(gradient) = parse_linear_gradient(value) {
        return Some(format!(".bg({gradient})"));
    }

    if let Some(color) = parse_color(value) {
        return Some(format!(".bg({color})"));
    }

    value
        .split_whitespace()
        .find_map(parse_color)
        .map(|color| format!(".bg({color})"))
}

fn parse_background_image(value: &str) -> Option<String> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none") {
        return Some(String::new());
    }

    parse_linear_gradient(value).map(|gradient| format!(".bg({gradient})"))
}

fn parse_linear_gradient(value: &str) -> Option<String> {
    parse_linear_gradient_with_color(value, parse_color)
}

fn parse_linear_gradient_with_color(
    value: &str,
    color: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    if !lower.starts_with("linear-gradient(") || !lower.ends_with(')') {
        return None;
    }

    let args = &value["linear-gradient(".len()..value.len() - 1];
    let parts = split_top_level_commas(args);
    let (angle, stops) = match parts.as_slice() {
        [from, to] => (180.0, [*from, *to]),
        [direction, from, to] => (parse_linear_gradient_angle(direction)?, [*from, *to]),
        _ => return None,
    };

    let from = parse_linear_color_stop(stops[0], 0.0, &color)?;
    let to = parse_linear_color_stop(stops[1], 1.0, &color)?;
    Some(format!(
        "gpui::linear_gradient({}, {from}, {to})",
        format_float(angle)
    ))
}

fn parse_linear_gradient_angle(value: &str) -> Option<f32> {
    let value = normalize_css_keyword(value);
    if let Some(degrees) = parse_numeric_suffix(&value, "deg") {
        return Some(degrees.rem_euclid(360.0));
    }

    let angle = match value.as_str() {
        "to top" => 0.0,
        "to right" => 90.0,
        "to bottom" => 180.0,
        "to left" => 270.0,
        "to top right" | "to right top" => 45.0,
        "to bottom right" | "to right bottom" => 135.0,
        "to bottom left" | "to left bottom" => 225.0,
        "to top left" | "to left top" => 315.0,
        _ => return None,
    };
    Some(angle)
}

fn parse_linear_color_stop(
    value: &str,
    default_percentage: f32,
    color: &impl Fn(&str) -> Option<String>,
) -> Option<String> {
    let parts = split_top_level_whitespace(value);
    let color = color(parts.first().copied()?)?;
    let percentage = parts.get(1).map_or(Some(default_percentage), |stop| {
        parse_numeric_suffix(stop, "%").map(|percentage| percentage / 100.0)
    })?;
    Some(format!(
        "gpui::linear_color_stop({color}, {})",
        format_float(percentage)
    ))
}

fn split_top_level_whitespace(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;

    for (index, ch) in value.char_indices() {
        if let Some(quote_ch) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == quote_ch {
                quote = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' => {
                quote = Some(ch);
                start.get_or_insert(index);
            }
            '(' | '[' => {
                depth += 1;
                start.get_or_insert(index);
            }
            ')' | ']' => {
                depth = depth.saturating_sub(1);
            }
            ch if ch.is_whitespace() && depth == 0 => {
                if let Some(part_start) = start.take() {
                    parts.push(value[part_start..index].trim());
                }
            }
            _ => {
                start.get_or_insert(index);
            }
        }
    }

    if let Some(part_start) = start {
        parts.push(value[part_start..].trim());
    }
    parts.retain(|part| !part.is_empty());
    parts
}

fn parse_box_edge_methods(
    value: &str,
    options: LengthOptions,
    all_method: &str,
    side_methods: [&str; 4],
) -> Option<String> {
    let parts = split_top_level_whitespace(value);
    let expanded = match parts.as_slice() {
        [all] => {
            let value = parse_length(all, options)?;
            return Some(format!(".{all_method}({value})"));
        }
        [vertical, horizontal] => [*vertical, *horizontal, *vertical, *horizontal],
        [top, horizontal, bottom] => [*top, *horizontal, *bottom, *horizontal],
        [top, right, bottom, left] => [*top, *right, *bottom, *left],
        _ => return None,
    };

    let methods = expanded
        .iter()
        .zip(side_methods)
        .map(|(part, method)| {
            parse_length(part, options).map(|value| format!(".{method}({value})"))
        })
        .collect::<Option<Vec<_>>>()?;

    join_style_methods(methods)
}

fn join_style_methods(methods: Vec<String>) -> Option<String> {
    (!methods.is_empty()).then(|| methods.join("\n    "))
}

fn parse_overflow(value: &str, axis: OverflowAxis) -> Option<String> {
    let value = normalize_css_keyword(value);
    let method = match (value.as_str(), axis) {
        ("visible", _) => return Some(String::new()),
        ("hidden" | "clip", OverflowAxis::Both) => ".overflow_hidden()",
        ("hidden" | "clip", OverflowAxis::X) => ".overflow_x_hidden()",
        ("hidden" | "clip", OverflowAxis::Y) => ".overflow_y_hidden()",
        ("auto" | "scroll", OverflowAxis::Both) => ".overflow_scroll()",
        ("auto" | "scroll", OverflowAxis::X) => ".overflow_x_scroll()",
        ("auto" | "scroll", OverflowAxis::Y) => ".overflow_y_scroll()",
        _ => return None,
    };
    Some(method.to_owned())
}

fn parse_dimension(value: &str, method: &str, full_method: Option<&str>) -> Option<String> {
    if let Some(full_method) = full_method
        && is_full_viewport_or_parent_length(value)
    {
        return Some(format!(".{full_method}()"));
    }

    parse_length(value, LengthOptions::auto_relative()).map(|value| format!(".{method}({value})"))
}

fn is_full_viewport_or_parent_length(value: &str) -> bool {
    matches!(
        normalize_css_keyword(value).as_str(),
        "100%"
            | "100vw"
            | "100vh"
            | "100dvw"
            | "100dvh"
            | "100svw"
            | "100svh"
            | "100lvw"
            | "100lvh"
    )
}

fn parse_grid_template_columns(value: &str) -> Option<String> {
    let value = normalize_css_keyword(value);
    if value == "none" {
        return Some(String::new());
    }

    if value.starts_with("repeat(auto-fill,")
        || value.starts_with("repeat(auto-fit,")
        || value.contains("minmax(")
    {
        return Some(".flex()\n    .flex_wrap()".to_owned());
    }

    if let Some(columns) = parse_grid_column_count(&value) {
        return Some(format!(".grid_cols({columns})"));
    }

    None
}

fn parse_grid_column_count(value: &str) -> Option<u16> {
    if let Some(inner) = value
        .strip_prefix("repeat(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let parts = split_top_level_commas(inner);
        let [count, _track] = parts.as_slice() else {
            return None;
        };
        return count.trim().parse::<u16>().ok().filter(|count| *count > 0);
    }

    let count = split_top_level_whitespace(value)
        .into_iter()
        .filter(|part| *part != "/")
        .count();
    u16::try_from(count).ok().filter(|count| *count > 0)
}

fn parse_position(value: &str) -> Option<String> {
    match normalize_css_keyword(value).as_str() {
        "static" => Some(String::new()),
        "relative" => Some(".relative()".to_owned()),
        "absolute" | "fixed" => Some(".absolute()".to_owned()),
        "sticky" => Some(".relative()".to_owned()),
        _ => None,
    }
}

fn parse_position_offset(value: &str, method: &str, margin_method: &str) -> Option<String> {
    if let Some(value) = parse_length(value, LengthOptions::auto_relative()) {
        return Some(format!(".{method}({value})"));
    }

    let offset = parse_calc_position_offset(value)?;
    let mut methods = vec![format!(".{method}({})", offset.base)];
    if let Some(margin) = offset.margin {
        methods.push(format!(".{margin_method}({margin})"));
    }
    join_style_methods(methods)
}

#[derive(Debug)]
struct CalcPositionOffset {
    base: String,
    margin: Option<String>,
}

fn parse_calc_position_offset(value: &str) -> Option<CalcPositionOffset> {
    let inner = strip_css_function(value, "calc")?;
    let inner = strip_wrapping_parentheses(inner.trim());

    if let Some(value) = parse_length(inner, LengthOptions::auto_relative()) {
        return Some(CalcPositionOffset {
            base: value,
            margin: None,
        });
    }

    let (left, operator, right) = split_top_level_binary(inner, &['+', '-'])?;
    let left = strip_wrapping_parentheses(left.trim());
    let right = strip_wrapping_parentheses(right.trim());

    if calc_term_is_zero(right) {
        return parse_length(left, LengthOptions::auto_relative())
            .map(|base| CalcPositionOffset { base, margin: None });
    }

    if operator == '+' && calc_term_is_zero(left) {
        return parse_length(right, LengthOptions::auto_relative())
            .map(|base| CalcPositionOffset { base, margin: None });
    }

    if let (Some(percent), Some(px)) = (parse_percent_number(left), parse_px_number(right)) {
        let margin = if operator == '-' { -px } else { px };
        return Some(CalcPositionOffset {
            base: format!("gpui::relative({})", format_float(percent / 100.0)),
            margin: Some(format!("gpui::px({})", format_float(margin))),
        });
    }

    if let (Some(px), Some(percent)) = (parse_px_number(left), parse_percent_number(right)) {
        let relative = if operator == '-' { -percent } else { percent };
        return Some(CalcPositionOffset {
            base: format!("gpui::relative({})", format_float(relative / 100.0)),
            margin: Some(format!("gpui::px({})", format_float(px))),
        });
    }

    None
}

fn parse_element_transform_offset(
    element: &RenderElement,
    style: &StyleDeclaration,
) -> Option<String> {
    if !element_has_absolute_position(element) {
        return None;
    }

    let transform = normalize_css_function_spacing(style.value.as_str());
    let width = element_static_pixel_dimension(element, &StyleProperty::Width);
    let height = element_static_pixel_dimension(element, &StyleProperty::Height);

    let (x, y) = if let Some(inner) = strip_css_function(&transform, "translatex") {
        (parse_translate_component(inner, width)?, 0.0)
    } else if let Some(inner) = strip_css_function(&transform, "translatey") {
        (0.0, parse_translate_component(inner, height)?)
    } else {
        let inner = strip_css_function(&transform, "translate")?;
        let parts = split_translate_arguments(inner);
        let x = parse_translate_component(parts.first().copied()?, width)?;
        let y = parts
            .get(1)
            .map_or(Some(0.0), |part| parse_translate_component(part, height))?;
        (x, y)
    };

    let horizontal_margin = if element_has_right_without_left(element) {
        "mr"
    } else {
        "ml"
    };
    let vertical_margin = if element_has_bottom_without_top(element) {
        "mb"
    } else {
        "mt"
    };
    let mut methods = Vec::new();
    if !float_is_zero(x) {
        methods.push(format!(
            ".{horizontal_margin}(gpui::px({}))",
            format_float(x)
        ));
    }
    if !float_is_zero(y) {
        methods.push(format!(".{vertical_margin}(gpui::px({}))", format_float(y)));
    }

    Some(methods.join("\n    "))
}

fn normalize_css_function_spacing(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn split_translate_arguments(value: &str) -> Vec<&str> {
    let comma_parts = split_top_level_commas(value);
    if comma_parts.len() > 1 {
        return comma_parts;
    }

    split_top_level_whitespace(value)
}

fn parse_translate_component(value: &str, basis_px: Option<f32>) -> Option<f32> {
    let value = strip_wrapping_parentheses(value.trim()).to_ascii_lowercase();
    if value == "0" {
        return Some(0.0);
    }
    if let Some(px) = parse_numeric_suffix(&value, "px") {
        return Some(px);
    }
    if let Some(percent) = parse_numeric_suffix(&value, "%") {
        return basis_px.map(|basis| basis * percent / 100.0);
    }

    None
}

fn element_static_pixel_dimension(
    element: &RenderElement,
    property: &StyleProperty,
) -> Option<f32> {
    element
        .styles
        .iter()
        .rev()
        .find(|style| &style.property == property)
        .and_then(|style| parse_px_number(style.value.as_str()))
}

fn element_has_absolute_position(element: &RenderElement) -> bool {
    element.styles.iter().rev().any(|style| {
        style.property == StyleProperty::Position
            && matches!(
                normalize_css_keyword(style.value.as_str()).as_str(),
                "absolute" | "fixed"
            )
    })
}

fn element_has_right_without_left(element: &RenderElement) -> bool {
    let has_right = element
        .styles
        .iter()
        .any(|style| style.property == StyleProperty::Right);
    let has_left = element
        .styles
        .iter()
        .any(|style| style.property == StyleProperty::Left);
    has_right && !has_left
}

fn element_has_bottom_without_top(element: &RenderElement) -> bool {
    let has_bottom = element
        .styles
        .iter()
        .any(|style| style.property == StyleProperty::Bottom);
    let has_top = element
        .styles
        .iter()
        .any(|style| style.property == StyleProperty::Top);
    has_bottom && !has_top
}

fn float_is_zero(value: f32) -> bool {
    value.abs() < 0.000_1
}

fn strip_css_function<'a>(value: &'a str, function: &str) -> Option<&'a str> {
    let value = value.trim();
    let name = value.get(..function.len())?;
    if !name.eq_ignore_ascii_case(function) {
        return None;
    }
    value
        .get(function.len()..)?
        .trim_start()
        .strip_prefix('(')?
        .strip_suffix(')')
}

fn calc_term_is_zero(value: &str) -> bool {
    let value = strip_wrapping_parentheses(value.trim());
    if value == "0" || is_zero_length(value) {
        return true;
    }

    split_top_level_binary(value, &['*'])
        .is_some_and(|(left, _, right)| calc_term_is_zero(left) || calc_term_is_zero(right))
}

fn parse_percent_number(value: &str) -> Option<f32> {
    let value = strip_wrapping_parentheses(value.trim()).to_ascii_lowercase();
    parse_numeric_suffix(&value, "%")
}

fn parse_px_number(value: &str) -> Option<f32> {
    let value = strip_wrapping_parentheses(value.trim()).to_ascii_lowercase();
    if value == "0" {
        return Some(0.0);
    }
    parse_numeric_suffix(&value, "px")
}

fn split_top_level_binary<'a>(
    value: &'a str,
    operators: &[char],
) -> Option<(&'a str, char, &'a str)> {
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;

    for (index, ch) in value.char_indices() {
        if let Some(quote_ch) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == quote_ch {
                quote = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' => quote = Some(ch),
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            ch if depth == 0 && operators.contains(&ch) && !is_unary_operator(value, index) => {
                let left = value[..index].trim();
                let right = value[index + ch.len_utf8()..].trim();
                if !left.is_empty() && !right.is_empty() {
                    return Some((left, ch, right));
                }
            }
            _ => {}
        }
    }

    None
}

fn is_unary_operator(value: &str, index: usize) -> bool {
    value[..index]
        .chars()
        .rev()
        .find(|ch| !ch.is_whitespace())
        .is_none_or(|previous| matches!(previous, '(' | '[' | '+' | '-' | '*' | '/'))
}

fn strip_wrapping_parentheses(mut value: &str) -> &str {
    loop {
        let trimmed = value.trim();
        if !trimmed.starts_with('(') || !trimmed.ends_with(')') {
            return trimmed;
        }

        let mut depth = 0usize;
        let mut wraps_all = false;
        for (index, ch) in trimmed.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        wraps_all = index == trimmed.len() - ch.len_utf8();
                        break;
                    }
                }
                _ => {}
            }
        }

        if !wraps_all {
            return trimmed;
        }

        value = &trimmed[1..trimmed.len() - 1];
    }
}

fn parse_flex(value: &str) -> Option<String> {
    let value = normalize_css_keyword(value);
    match value.as_str() {
        "none" => return Some(".flex_none()".to_owned()),
        "auto" => return Some(".flex_auto()".to_owned()),
        "initial" => return Some(".flex_initial()".to_owned()),
        "1" => return Some(".flex_1()".to_owned()),
        "0" => {
            return Some(".flex_initial()\n    .flex_basis(gpui::px(0.0))".to_owned());
        }
        _ => {}
    }

    let parts = value.split_whitespace().collect::<Vec<_>>();
    if parts.len() == 1 {
        return parse_length(parts[0], LengthOptions::auto_relative())
            .map(|value| format!(".flex_grow_1()\n    .flex_basis({value})"));
    }

    let [grow, shrink, basis] = parts.as_slice() else {
        return None;
    };

    if *grow == "1" && *shrink == "1" && is_zero_length(basis) {
        return Some(".flex_1()".to_owned());
    }
    if *grow == "1" && *shrink == "1" && *basis == "auto" {
        return Some(".flex_auto()".to_owned());
    }
    if *grow == "0" && *shrink == "1" && *basis == "auto" {
        return Some(".flex_initial()".to_owned());
    }

    let mut methods = Vec::new();
    match *grow {
        "0" => {}
        "1" => methods.push(".flex_grow_1()".to_owned()),
        _ => return None,
    }
    match *shrink {
        "0" => methods.push(".flex_shrink_0()".to_owned()),
        "1" => {}
        _ => return None,
    }
    if *grow == "0" && *shrink == "0" {
        methods.push(".flex_none()".to_owned());
    }
    if let Some(value) = parse_length(basis, LengthOptions::auto_relative()) {
        methods.push(format!(".flex_basis({value})"));
    }

    join_style_methods(methods)
}

fn parse_flex_direction(value: &str) -> Option<String> {
    let method = match normalize_css_keyword(value).as_str() {
        "row" => ".flex_row()",
        "row-reverse" => ".flex_row_reverse()",
        "column" => ".flex_col()",
        "column-reverse" => ".flex_col_reverse()",
        _ => return None,
    };
    Some(format!(".flex()\n    {method}"))
}

fn parse_flex_wrap(value: &str) -> Option<String> {
    let method = match normalize_css_keyword(value).as_str() {
        "nowrap" => ".flex_nowrap()",
        "wrap" => ".flex_wrap()",
        "wrap-reverse" => ".flex_wrap_reverse()",
        _ => return None,
    };
    Some(method.to_owned())
}

fn parse_flex_grow(value: &str) -> Option<String> {
    match normalize_css_keyword(value).as_str() {
        "0" => Some(String::new()),
        "1" => Some(".flex_grow_1()".to_owned()),
        _ => None,
    }
}

fn parse_flex_shrink(value: &str) -> Option<String> {
    match normalize_css_keyword(value).as_str() {
        "0" => Some(".flex_shrink_0()".to_owned()),
        "1" => Some(".flex_shrink_1()".to_owned()),
        _ => None,
    }
}

fn parse_align_items(value: &str) -> Option<String> {
    let method = match normalize_css_keyword(value).as_str() {
        "normal" | "stretch" => return Some(String::new()),
        "start" | "flex-start" => ".items_start()",
        "end" | "flex-end" => ".items_end()",
        "center" => ".items_center()",
        "baseline" => ".items_baseline()",
        _ => return None,
    };
    Some(method.to_owned())
}

fn parse_align_self(value: &str) -> Option<String> {
    match normalize_css_keyword(value).as_str() {
        "auto" | "normal" | "stretch" => Some(String::new()),
        _ => None,
    }
}

fn parse_align_content(value: &str) -> Option<String> {
    let method = match normalize_css_keyword(value).as_str() {
        "normal" => ".content_normal()",
        "start" | "flex-start" => ".content_start()",
        "end" | "flex-end" => ".content_end()",
        "center" => ".content_center()",
        "space-between" => ".content_between()",
        "space-around" => ".content_around()",
        "space-evenly" => ".content_evenly()",
        "stretch" => ".content_stretch()",
        _ => return None,
    };
    Some(method.to_owned())
}

fn parse_justify_content(value: &str) -> Option<String> {
    let method = match normalize_css_keyword(value).as_str() {
        "normal" | "stretch" => return Some(String::new()),
        "start" | "flex-start" | "left" => ".justify_start()",
        "end" | "flex-end" | "right" => ".justify_end()",
        "center" => ".justify_center()",
        "space-between" => ".justify_between()",
        "space-around" => ".justify_around()",
        _ => return None,
    };
    Some(method.to_owned())
}

fn parse_length(value: &str, options: LengthOptions) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    if let Some(inner) = strip_css_function(&value, "calc")
        && let Some(value) = simplify_calc_length(inner)
    {
        return parse_length(&value, options);
    }

    if value == "0" {
        return Some("gpui::px(0.0)".to_owned());
    }

    if value == "auto" && options.allow_auto {
        return Some("gpui::auto()".to_owned());
    }

    if options.allow_auto && is_intrinsic_size_keyword(&value) {
        return Some("gpui::auto()".to_owned());
    }

    if let Some(numeric) = parse_numeric_suffix(&value, "px") {
        return Some(format!("gpui::px({})", format_float(numeric)));
    }

    if let Some(numeric) = parse_numeric_suffix(&value, "rem") {
        return Some(format!("gpui::rems({})", format_float(numeric)));
    }

    if let Some(numeric) = parse_numeric_suffix(&value, "em") {
        return Some(format!("gpui::rems({})", format_float(numeric)));
    }

    if options.allow_relative
        && let Some(numeric) = parse_numeric_suffix(&value, "%")
    {
        return Some(format!("gpui::relative({})", format_float(numeric / 100.0)));
    }

    if options.allow_relative
        && let Some(numeric) =
            parse_numeric_suffix(&value, "vw").or_else(|| parse_numeric_suffix(&value, "vh"))
    {
        return Some(format!("gpui::relative({})", format_float(numeric / 100.0)));
    }

    None
}

fn simplify_calc_length(value: &str) -> Option<String> {
    let value = strip_wrapping_parentheses(value.trim());
    let (left, operator, right) = split_top_level_binary(value, &['*', '/'])?;
    let left = strip_wrapping_parentheses(left.trim());
    let right = strip_wrapping_parentheses(right.trim());

    let (number, unit) = match operator {
        '*' => {
            if let Some((length, unit)) = parse_numeric_dimension(left) {
                (length * parse_css_number(right)?, unit)
            } else {
                let scalar = parse_css_number(left)?;
                let (length, unit) = parse_numeric_dimension(right)?;
                (scalar * length, unit)
            }
        }
        '/' => {
            let (length, unit) = parse_numeric_dimension(left)?;
            let divisor = parse_css_number(right)?;
            if float_is_zero(divisor) {
                return None;
            }
            (length / divisor, unit)
        }
        _ => return None,
    };

    Some(format!("{}{unit}", format_float(number)))
}

fn parse_numeric_dimension(value: &str) -> Option<(f32, &'static str)> {
    ["rem", "px", "em", "vw", "vh", "%"]
        .into_iter()
        .find_map(|unit| parse_numeric_suffix(value, unit).map(|number| (number, unit)))
}

fn is_intrinsic_size_keyword(value: &str) -> bool {
    matches!(
        normalize_css_keyword(value).as_str(),
        "fit-content" | "max-content" | "min-content"
    )
}

fn parse_numeric_suffix(value: &str, suffix: &str) -> Option<f32> {
    value.strip_suffix(suffix)?.trim().parse::<f32>().ok()
}

fn is_zero_length(value: &str) -> bool {
    let value = normalize_css_keyword(value);
    value == "0"
        || value == "0%"
        || value == "0px"
        || value == "0rem"
        || value == "0em"
        || value == "0vh"
        || value == "0vw"
}

fn parse_css_number(value: &str) -> Option<f32> {
    value.trim().parse::<f32>().ok()
}

fn parse_aspect_ratio(value: &str) -> Option<String> {
    let value = value.trim();
    let value = value.strip_prefix("auto").map(str::trim).unwrap_or(value);
    let ratio = if let Some((width, height)) = value.split_once('/') {
        let width = parse_css_number(width)?;
        let height = parse_css_number(height)?;
        if height == 0.0 {
            return None;
        }
        width / height
    } else {
        parse_css_number(value)?
    };

    (ratio > 0.0).then(|| format_float(ratio))
}

fn parse_line_height(value: &str) -> Option<String> {
    let value = value.trim();
    if let Some(number) = parse_css_number(value) {
        return Some(format!("gpui::relative({})", format_float(number)));
    }

    parse_length(value, LengthOptions::relative())
}

fn parse_color(value: &str) -> Option<String> {
    let color = parse_color_literal(value)?;

    Some(match color {
        ColorLiteral::Rgb(hex) => format!("gpui::rgb(0x{hex:06X})"),
        ColorLiteral::Rgba(hex) => format!("gpui::rgba(0x{hex:08X})"),
    })
}

fn parse_color_literal(value: &str) -> Option<ColorLiteral> {
    let value = value.trim().to_ascii_lowercase();
    Some(match value.as_str() {
        "black" => ColorLiteral::Rgb(0x000000),
        "white" => ColorLiteral::Rgb(0xFFFFFF),
        "red" => ColorLiteral::Rgb(0xFF0000),
        "green" => ColorLiteral::Rgb(0x008000),
        "blue" => ColorLiteral::Rgb(0x0000FF),
        "transparent" => ColorLiteral::Rgba(0x00000000),
        _ => parse_hex_color(&value)
            .or_else(|| parse_function_color(&value))
            .or_else(|| parse_modern_color(&value))?,
    })
}

#[derive(Debug, Clone, Copy)]
enum ColorLiteral {
    Rgb(u32),
    Rgba(u32),
}

fn parse_modern_color(value: &str) -> Option<ColorLiteral> {
    let CssColor::RGBA(color) = CssColor::parse_string(value).ok()?.to_rgb().ok()? else {
        return None;
    };
    let rgb = (u32::from(color.red) << 16) | (u32::from(color.green) << 8) | u32::from(color.blue);
    if color.alpha == u8::MAX {
        Some(ColorLiteral::Rgb(rgb))
    } else {
        Some(ColorLiteral::Rgba((rgb << 8) | u32::from(color.alpha)))
    }
}

fn color_is_low_saturation_dark(value: &str) -> bool {
    let Some((r, g, b)) = parse_color_literal(value).map(color_literal_rgb) else {
        return false;
    };
    let r = f32::from(r) / 255.0;
    let g = f32::from(g) / 255.0;
    let b = f32::from(b) / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let lightness = (max + min) / 2.0;
    let saturation = if (max - min).abs() < f32::EPSILON {
        0.0
    } else {
        let delta = max - min;
        delta / (1.0 - (2.0 * lightness - 1.0).abs())
    };

    lightness < 0.35 && saturation < 0.20
}

fn color_literal_rgb(color: ColorLiteral) -> (u8, u8, u8) {
    let value = match color {
        ColorLiteral::Rgb(value) => value,
        ColorLiteral::Rgba(value) => value >> 8,
    };

    (
        ((value >> 16) & 0xFF) as u8,
        ((value >> 8) & 0xFF) as u8,
        (value & 0xFF) as u8,
    )
}

fn parse_hex_color(value: &str) -> Option<ColorLiteral> {
    let value = value.strip_prefix('#')?;
    match value.len() {
        3 => {
            let expanded = expand_hex(value);
            u32::from_str_radix(&expanded, 16)
                .ok()
                .map(ColorLiteral::Rgb)
        }
        4 => {
            let expanded = expand_hex(value);
            u32::from_str_radix(&expanded, 16)
                .ok()
                .map(ColorLiteral::Rgba)
        }
        6 => u32::from_str_radix(value, 16).ok().map(ColorLiteral::Rgb),
        8 => u32::from_str_radix(value, 16).ok().map(ColorLiteral::Rgba),
        _ => None,
    }
}

fn expand_hex(value: &str) -> String {
    let mut expanded = String::with_capacity(value.len() * 2);
    for ch in value.chars() {
        expanded.push(ch);
        expanded.push(ch);
    }
    expanded
}

fn parse_function_color(value: &str) -> Option<ColorLiteral> {
    let (name, args) = value.split_once('(')?;
    let args = args.strip_suffix(')')?;
    if name != "rgb" && name != "rgba" {
        return None;
    }

    let parts = args
        .replace(['/', ','], " ")
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if parts.len() < 3 {
        return None;
    }

    let r = parse_color_channel(&parts[0])?;
    let g = parse_color_channel(&parts[1])?;
    let b = parse_color_channel(&parts[2])?;
    let alpha = parts
        .get(3)
        .map_or(Some(255), |value| parse_alpha_channel(value))?;
    let hex = ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | alpha as u32;

    if alpha == 255 {
        Some(ColorLiteral::Rgb(hex >> 8))
    } else {
        Some(ColorLiteral::Rgba(hex))
    }
}

fn parse_color_channel(value: &str) -> Option<u8> {
    if let Some(percent) = value.strip_suffix('%') {
        let percent = percent.parse::<f32>().ok()?;
        return Some(((percent.clamp(0.0, 100.0) / 100.0) * 255.0).round() as u8);
    }

    let value = value.parse::<f32>().ok()?;
    Some(value.clamp(0.0, 255.0).round() as u8)
}

fn parse_alpha_channel(value: &str) -> Option<u8> {
    if let Some(percent) = value.strip_suffix('%') {
        let percent = percent.parse::<f32>().ok()?;
        return Some(((percent.clamp(0.0, 100.0) / 100.0) * 255.0).round() as u8);
    }

    let value = value.parse::<f32>().ok()?;
    let alpha = if value <= 1.0 { value * 255.0 } else { value };
    Some(alpha.clamp(0.0, 255.0).round() as u8)
}

fn parse_opacity(value: &str) -> Option<String> {
    let value = value.trim();
    let opacity = if let Some(percent) = value.strip_suffix('%') {
        percent.parse::<f32>().ok()? / 100.0
    } else {
        parse_css_number(value)?
    };
    Some(format!(
        ".opacity({})",
        format_float(opacity.clamp(0.0, 1.0))
    ))
}

fn parse_font_weight(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    let weight = match value.as_str() {
        "thin" | "100" => "gpui::FontWeight::THIN".to_owned(),
        "extra-light" | "extralight" | "200" => "gpui::FontWeight::EXTRA_LIGHT".to_owned(),
        "light" | "300" => "gpui::FontWeight::LIGHT".to_owned(),
        "normal" | "regular" | "400" => "gpui::FontWeight::NORMAL".to_owned(),
        "medium" | "500" => "gpui::FontWeight::MEDIUM".to_owned(),
        "semibold" | "semi-bold" | "600" => "gpui::FontWeight::SEMIBOLD".to_owned(),
        "bold" | "700" => "gpui::FontWeight::BOLD".to_owned(),
        "extra-bold" | "extrabold" | "800" => "gpui::FontWeight::EXTRA_BOLD".to_owned(),
        "black" | "900" => "gpui::FontWeight::BLACK".to_owned(),
        _ => {
            let value = value.parse::<f32>().ok()?;
            format!("gpui::FontWeight({})", format_float(value))
        }
    };

    Some(weight)
}

fn parse_font_style(value: &str) -> Option<String> {
    match normalize_css_keyword(value).as_str() {
        "italic" | "oblique" => Some(".italic()".to_owned()),
        "normal" => Some(".not_italic()".to_owned()),
        _ => None,
    }
}

fn parse_font_family(value: &str) -> Option<String> {
    let family = value
        .split(',')
        .next()?
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim();
    (!family.is_empty()).then(|| family.to_owned())
}

fn parse_text_align(value: &str) -> Option<String> {
    let method = match normalize_css_keyword(value).as_str() {
        "left" | "start" => ".text_left()",
        "center" => ".text_center()",
        "right" | "end" => ".text_right()",
        _ => return None,
    };
    Some(method.to_owned())
}

fn parse_text_decoration(value: &str) -> Option<String> {
    let value = normalize_css_keyword(value);
    if value == "none" {
        return Some(".text_decoration_none()".to_owned());
    }
    if value.split_whitespace().any(|part| part == "underline") {
        return Some(".underline()".to_owned());
    }
    if value.split_whitespace().any(|part| part == "line-through") {
        return Some(".line_through()".to_owned());
    }
    None
}

fn parse_text_overflow(value: &str) -> Option<String> {
    match normalize_css_keyword(value).as_str() {
        "ellipsis" => Some(".text_ellipsis()".to_owned()),
        "clip" => Some(String::new()),
        _ => None,
    }
}

fn parse_white_space(value: &str) -> Option<String> {
    match normalize_css_keyword(value).as_str() {
        "normal" => Some(".whitespace_normal()".to_owned()),
        "nowrap" => Some(".whitespace_nowrap()".to_owned()),
        "pre" => Some(".whitespace_nowrap()".to_owned()),
        _ => None,
    }
}

fn parse_cursor(value: &str) -> Option<String> {
    let method = match normalize_css_keyword(value).as_str() {
        "auto" | "default" => ".cursor_default()",
        "pointer" => ".cursor_pointer()",
        "text" => ".cursor_text()",
        "move" => ".cursor_move()",
        "not-allowed" => ".cursor_not_allowed()",
        "context-menu" => ".cursor_context_menu()",
        "crosshair" => ".cursor_crosshair()",
        "vertical-text" => ".cursor_vertical_text()",
        "alias" => ".cursor_alias()",
        "copy" => ".cursor_copy()",
        "no-drop" => ".cursor_no_drop()",
        "grab" => ".cursor_grab()",
        "grabbing" => ".cursor_grabbing()",
        "ew-resize" => ".cursor_ew_resize()",
        "ns-resize" => ".cursor_ns_resize()",
        "nesw-resize" => ".cursor_nesw_resize()",
        "nwse-resize" => ".cursor_nwse_resize()",
        "col-resize" => ".cursor_col_resize()",
        "row-resize" => ".cursor_row_resize()",
        "n-resize" => ".cursor_n_resize()",
        "e-resize" => ".cursor_e_resize()",
        "s-resize" => ".cursor_s_resize()",
        "w-resize" => ".cursor_w_resize()",
        _ => return None,
    };
    Some(method.to_owned())
}

fn parse_border_with_method(value: &str, method: &str) -> Option<String> {
    if matches!(normalize_css_keyword(value).as_str(), "none" | "0") {
        return Some(format!(".{method}(gpui::px(0.0))"));
    }

    let mut width = None;
    let mut color = None;
    let mut dashed = false;

    for part in value.split_whitespace() {
        if width.is_none() {
            width = parse_length(part, LengthOptions::absolute());
        }

        if color.is_none() {
            color = parse_color(part);
        }

        if part.eq_ignore_ascii_case("dashed") {
            dashed = true;
        }
    }

    let mut methods = Vec::new();
    if let Some(width) = width {
        methods.push(format!(".{method}({width})"));
    }
    if dashed {
        methods.push(".border_dashed()".to_owned());
    }
    if let Some(color) = color {
        methods.push(format!(".border_color({color})"));
    }

    join_style_methods(methods)
}

fn parse_border_style(value: &str) -> Option<String> {
    let normalized = normalize_css_keyword(value);
    if normalized
        .split_whitespace()
        .any(|part| part.eq_ignore_ascii_case("dashed"))
    {
        return Some(".border_dashed()".to_owned());
    }

    if normalized
        .split_whitespace()
        .all(|part| matches!(part, "solid" | "none" | "hidden" | "initial"))
    {
        return Some(String::new());
    }

    None
}

fn parse_border_radius(value: &str) -> Option<String> {
    let value = value.trim();
    let normalized = value.to_ascii_lowercase();
    if let Some(percent) = parse_numeric_suffix(&normalized, "%")
        && percent >= 50.0
    {
        return Some(".rounded_full()".to_owned());
    }

    parse_length(value, LengthOptions::absolute()).map(|value| format!(".rounded({value})"))
}

fn border_width_method(property: &StyleProperty) -> Option<&'static str> {
    match property {
        StyleProperty::Border => Some("border"),
        StyleProperty::BorderTop => Some("border_t"),
        StyleProperty::BorderRight => Some("border_r"),
        StyleProperty::BorderBottom => Some("border_b"),
        StyleProperty::BorderLeft => Some("border_l"),
        _ => None,
    }
}

fn parse_box_shadow(value: &str) -> Option<String> {
    if matches!(normalize_css_keyword(value).as_str(), "none" | "0") {
        return Some(".shadow_none()".to_owned());
    }

    let shadows = split_top_level_commas(value)
        .into_iter()
        .filter_map(parse_single_box_shadow)
        .collect::<Vec<_>>();

    (!shadows.is_empty()).then(|| format!(".shadow(std::vec![{}])", shadows.join(", ")))
}

fn parse_element_inset_shadow_border(
    element: &RenderElement,
    style: &StyleDeclaration,
) -> Option<String> {
    if element_has_explicit_border(element) {
        return None;
    }

    let shadows = split_top_level_commas(style.value.as_str());
    let [shadow] = shadows.as_slice() else {
        return None;
    };
    let edge = parse_inset_shadow_edge(shadow)?;
    Some(format!(
        ".{}({})\n    .border_color({})",
        edge.method, edge.width, edge.color
    ))
}

#[derive(Debug)]
struct InsetShadowEdge {
    method: &'static str,
    width: String,
    color: String,
}

fn parse_inset_shadow_edge(value: &str) -> Option<InsetShadowEdge> {
    let mut inset = false;
    let mut lengths = Vec::new();
    let mut color = None;

    for part in split_top_level_whitespace(value) {
        if part.eq_ignore_ascii_case("inset") {
            inset = true;
            continue;
        }
        if let Some(length) = parse_px_number(part) {
            lengths.push(length);
            continue;
        }
        if color.is_none() {
            color = parse_color(part);
        }
    }

    if !inset || lengths.len() < 2 {
        return None;
    }

    let offset_x = lengths[0];
    let offset_y = lengths[1];
    let blur = lengths.get(2).copied().unwrap_or(0.0);
    let spread = lengths.get(3).copied().unwrap_or(0.0);
    if !float_is_zero(blur) || !float_is_zero(spread) {
        return None;
    }

    let (method, width) = if float_is_zero(offset_x) && offset_y < 0.0 {
        ("border_b", -offset_y)
    } else if float_is_zero(offset_x) && offset_y > 0.0 {
        ("border_t", offset_y)
    } else if float_is_zero(offset_y) && offset_x < 0.0 {
        ("border_r", -offset_x)
    } else if float_is_zero(offset_y) && offset_x > 0.0 {
        ("border_l", offset_x)
    } else {
        return None;
    };

    Some(InsetShadowEdge {
        method,
        width: format!("gpui::px({})", format_float(width)),
        color: color.unwrap_or_else(|| "gpui::rgba(0x000000FF)".to_owned()),
    })
}

fn element_has_explicit_border(element: &RenderElement) -> bool {
    element.styles.iter().any(|style| {
        matches!(
            style.property,
            StyleProperty::Border
                | StyleProperty::BorderTop
                | StyleProperty::BorderRight
                | StyleProperty::BorderBottom
                | StyleProperty::BorderLeft
                | StyleProperty::BorderWidth
                | StyleProperty::BorderColor
                | StyleProperty::BorderStyle
                | StyleProperty::BorderTopStyle
                | StyleProperty::BorderRightStyle
                | StyleProperty::BorderBottomStyle
                | StyleProperty::BorderLeftStyle
        )
    })
}

fn parse_single_box_shadow(value: &str) -> Option<String> {
    let mut lengths = Vec::new();
    let mut color = None;

    for part in value.split_whitespace() {
        if part.eq_ignore_ascii_case("inset") {
            return None;
        }
        if color.is_none() {
            color = parse_color(part);
            if color.is_some() {
                continue;
            }
        }
        if let Some(length) = parse_pixel_length(part) {
            lengths.push(length);
        }
    }

    if lengths.len() < 2 {
        return None;
    }

    let offset_x = &lengths[0];
    let offset_y = &lengths[1];
    let blur_radius = lengths
        .get(2)
        .cloned()
        .unwrap_or_else(|| "gpui::px(0.0)".to_owned());
    let spread_radius = lengths
        .get(3)
        .cloned()
        .unwrap_or_else(|| "gpui::px(0.0)".to_owned());
    let color = color.unwrap_or_else(|| "gpui::rgba(0x000000FF)".to_owned());

    Some(format!(
        "gpui::BoxShadow {{ color: {color}.into(), offset: gpui::point({offset_x}, {offset_y}), blur_radius: {blur_radius}, spread_radius: {spread_radius}, inset: false }}"
    ))
}

fn parse_pixel_length(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    if value == "0" {
        return Some("gpui::px(0.0)".to_owned());
    }
    parse_numeric_suffix(&value, "px").map(|numeric| format!("gpui::px({})", format_float(numeric)))
}

fn split_top_level_commas(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;

    for (index, ch) in value.char_indices() {
        if let Some(quote_ch) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == quote_ch {
                quote = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' => quote = Some(ch),
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(value[start..index].trim());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }

    parts.push(value[start..].trim());
    parts.retain(|part| !part.is_empty());
    parts
}

fn format_style_variant(variant: &RenderStyleVariant) -> String {
    format!(
        "conditional CSS {} {} {{{}}}",
        format_style_conditions(&variant.conditions),
        variant.selector,
        format_style_declarations(&variant.declarations)
    )
}

fn format_pseudo_element(pseudo: &RenderPseudoElement) -> String {
    let text = pseudo
        .children
        .iter()
        .filter_map(|child| match child {
            RenderNode::Text(text) => Some(format!("content: {:?}", text.value)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let declarations = pseudo
        .styles
        .iter()
        .map(format_style_declaration)
        .chain(text)
        .collect::<Vec<_>>()
        .join("; ");

    format!(
        "pseudo-element CSS selector={:?}; kind={:?}; conditions={:?} {{{declarations}}}",
        pseudo.selector.as_str(),
        pseudo.kind.as_str(),
        format_pseudo_conditions_metadata(&pseudo.conditions)
    )
}

fn format_pseudo_conditions_metadata(conditions: &[RenderStyleCondition]) -> String {
    conditions
        .iter()
        .map(|condition| match condition {
            RenderStyleCondition::PseudoClass(value) => {
                format!("pseudo-class={:?}", value.as_str())
            }
            RenderStyleCondition::PseudoElement(value) => {
                format!("pseudo-element={:?}", value.as_str())
            }
            RenderStyleCondition::Media(value) => format!("media={:?}", value.as_str()),
            RenderStyleCondition::Supports(value) => format!("supports={:?}", value.as_str()),
            RenderStyleCondition::Container(value) => format!("container={:?}", value.as_str()),
            RenderStyleCondition::StartingStyle => "starting-style".to_owned(),
            RenderStyleCondition::ActiveViewTransitionType(types) => {
                format!("active-view-transition-type={:?}", types.join(","))
            }
            RenderStyleCondition::ElementState {
                pseudo,
                ancestor,
                negated,
            } => format!(
                "element-state={:?} ancestor={ancestor} negated={negated}",
                pseudo.as_str()
            ),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn format_style_conditions(conditions: &[RenderStyleCondition]) -> String {
    conditions
        .iter()
        .map(format_style_condition)
        .collect::<Vec<_>>()
        .join(" and ")
}

fn format_style_condition(condition: &RenderStyleCondition) -> String {
    match condition {
        RenderStyleCondition::PseudoClass(value) => format!(":{value}"),
        RenderStyleCondition::PseudoElement(value) => format!("::{value}"),
        RenderStyleCondition::Media(value) => format!("@media {value}"),
        RenderStyleCondition::Supports(value) => format!("@supports {value}"),
        RenderStyleCondition::Container(value) => format!("@container {value}"),
        RenderStyleCondition::StartingStyle => "@starting-style".to_owned(),
        RenderStyleCondition::ActiveViewTransitionType(types) => {
            format!(":active-view-transition-type({})", types.join(", "))
        }
        RenderStyleCondition::ElementState {
            pseudo,
            ancestor,
            negated,
        } => {
            let state = if *negated {
                format!(":not(:{pseudo})")
            } else {
                format!(":{pseudo}")
            };
            if *ancestor == 0 {
                state
            } else {
                format!("ancestor {ancestor} up {state}")
            }
        }
    }
}

fn format_style_declarations(styles: &[StyleDeclaration]) -> String {
    styles
        .iter()
        .map(format_style_declaration)
        .collect::<Vec<_>>()
        .join("; ")
}

fn format_style_declaration(style: &StyleDeclaration) -> String {
    if style.important {
        format!("{}: {} !important", style.property, style.value)
    } else {
        format!("{}: {}", style.property, style.value)
    }
}

fn format_dynamic_expression(expression: &TemplateString) -> String {
    let expressions = expression
        .expressions()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if expressions.is_empty() {
        return format!("{:?}", expression.raw.as_str());
    }

    format!("{:?} [{}]", expression.raw.as_str(), expressions.join(", "))
}

fn dynamic_style_comment(
    dynamic_style: &RenderDynamicStyleBinding,
    element: &RenderElement,
) -> MappedComment {
    let state = dynamic_style
        .state
        .as_ref()
        .map_or_else(|| "style".to_owned(), |state| format!("{state} style"));
    MappedComment::new(
        format!(
            "dynamic {state}: {}",
            format_dynamic_expression(&dynamic_style.expression)
        ),
        dynamic_style
            .span
            .or(dynamic_style.expression.span)
            .or(element.span),
        SourceMappingKind::Style,
    )
}

fn dynamic_style_declarations(
    expression: &TemplateString,
) -> Option<Vec<(StyleProperty, TemplateString)>> {
    let mut declarations = Vec::new();
    for declaration in expression.raw.split(';') {
        let declaration = declaration.trim();
        if declaration.is_empty() {
            continue;
        }
        let (property, value) = declaration.split_once(':')?;
        let property = StyleProperty::from(property.trim());
        let value = parse_dynamic_style_value_template(value.trim(), expression.span)?;
        declarations.push((property, value));
    }

    (!declarations.is_empty()).then_some(declarations)
}

fn parse_dynamic_style_value_template(value: &str, span: Option<Span>) -> Option<TemplateString> {
    if !value.contains("{{") {
        return Some(TemplateString::new(value, Vec::new(), span));
    }

    let mut rest = value;
    let mut segments = Vec::new();
    while let Some(start) = rest.find("{{") {
        let literal = &rest[..start];
        if !literal.is_empty() {
            segments.push(TemplateSegment::Literal(CompactString::from(literal)));
        }
        let after_start = &rest[start + 2..];
        let end = after_start.find("}}")?;
        let expression = after_start[..end].trim();
        segments.push(TemplateSegment::Expression(parse_dynamic_style_expr(
            expression,
        )?));
        rest = &after_start[end + 2..];
    }
    if !rest.is_empty() {
        segments.push(TemplateSegment::Literal(CompactString::from(rest)));
    }

    Some(TemplateString::new(value, segments, span))
}

fn parse_dynamic_style_expr(value: &str) -> Option<Expr> {
    let path = value
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(CompactString::from)
        .collect::<Vec<_>>();
    (!path.is_empty()).then(|| Expr::Path(path))
}

fn template_text_expression(expression: &TemplateString, scope: &RenderScope) -> String {
    template_text_expression_with_root(expression, scope, "self")
}

fn render_text_expression(text: &RenderText, scope: &RenderScope) -> String {
    if let Some(template) = text.template.as_ref() {
        return apply_text_transform_expression(
            template_text_expression(template, scope),
            scope.text_transform,
        );
    }

    rust_string(&transform_static_text(&text.value, scope.text_transform))
}

fn material_symbol_text_expression(element: &RenderElement, scope: &RenderScope) -> String {
    let [RenderNode::Text(text)] = element.children.as_slice() else {
        return rust_string("");
    };

    render_text_expression(text, scope)
}

fn transform_static_text(value: &str, text_transform: TextTransform) -> String {
    let value = collapse_html_text(value);
    match text_transform {
        TextTransform::None => value,
        TextTransform::Uppercase => value.to_uppercase(),
        TextTransform::Lowercase => value.to_lowercase(),
    }
}

fn collapse_html_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn apply_text_transform_expression(expression: String, text_transform: TextTransform) -> String {
    match text_transform {
        TextTransform::None => expression,
        TextTransform::Uppercase => format!("({expression}).to_uppercase()"),
        TextTransform::Lowercase => format!("({expression}).to_lowercase()"),
    }
}

fn text_transform_for_element(element: &RenderElement, scope: &RenderScope) -> TextTransform {
    let mut text_transform = scope.text_transform;
    for style in &element.styles {
        if style.property != StyleProperty::TextTransform {
            continue;
        }
        if let Some(next) = text_transform_from_css(style.value.as_str()) {
            text_transform = next;
        }
    }
    text_transform
}

fn text_transform_from_css(value: &str) -> Option<TextTransform> {
    match normalize_css_keyword(value).as_str() {
        "none" => Some(TextTransform::None),
        "uppercase" => Some(TextTransform::Uppercase),
        "lowercase" => Some(TextTransform::Lowercase),
        _ => None,
    }
}

fn gpui_tooltip_method(value: &str) -> String {
    format!(
        ".tooltip({{\n    let htmlswap_tooltip = ({value}).to_string();\n    move |_window, _cx| {{\n        let htmlswap_tooltip = htmlswap_tooltip.clone();\n        _cx.new(|_| HtmlswapTooltipView {{ text: htmlswap_tooltip }}).into()\n    }}\n}})"
    )
}

fn template_text_expression_with_root(
    expression: &TemplateString,
    scope: &RenderScope,
    root: &str,
) -> String {
    let mut format_string = String::new();
    let mut arguments = Vec::new();

    for part in &expression.segments {
        match part {
            TemplateSegment::Literal(literal) => push_format_literal(&mut format_string, literal),
            TemplateSegment::Expression(expression) => {
                format_string.push_str("{}");
                arguments.push(rust_binding_expression_with_root(expression, scope, root));
            }
        }
    }

    let format_string = collapse_html_text(&format_string);
    if arguments.is_empty() {
        return rust_string(&format_string);
    }

    format!(
        "format!({}, {})",
        rust_string(&format_string),
        arguments.join(", ")
    )
}

fn push_format_literal(output: &mut String, literal: &str) {
    for ch in literal.chars() {
        match ch {
            '{' => output.push_str("{{"),
            '}' => output.push_str("}}"),
            _ => output.push(ch),
        }
    }
}

fn rust_binding_expression(expression: &Expr, scope: &RenderScope) -> String {
    rust_binding_expression_with_root(expression, scope, "self")
}

fn rust_binding_expression_with_root(expression: &Expr, scope: &RenderScope, root: &str) -> String {
    match expression {
        Expr::Path(segments) => rust_path_expression(segments, scope, root),
        Expr::Call { callee, arguments } => {
            let arguments = arguments
                .iter()
                .map(|argument| rust_binding_expression_with_root(argument, scope, root))
                .collect::<Vec<_>>()
                .join(", ");
            if let Expr::Path(segments) = callee.as_ref() {
                rust_path_call_expression(segments, scope, root, &arguments)
            } else {
                format!(
                    "{}({arguments})",
                    rust_binding_expression_with_root(callee, scope, root)
                )
            }
        }
        Expr::Literal(literal) => rust_literal_expression(literal),
        Expr::Opaque(value) => {
            let fallback = rust_identifier_from_segment(value, "value");
            format!("{root}.{fallback}()")
        }
        Expr::Member { .. }
        | Expr::Index { .. }
        | Expr::Binary { .. }
        | Expr::Logical { .. }
        | Expr::Conditional { .. }
        | Expr::TemplateLiteral { .. }
        | Expr::Array(_)
        | Expr::Object(_) => {
            let fallback = rust_identifier_from_segment(&expression.to_string(), "value");
            format!("{root}.{fallback}()")
        }
    }
}

fn rust_path_expression(segments: &[CompactString], scope: &RenderScope, root: &str) -> String {
    let Some((first, rest)) = segments.split_first() else {
        return format!("{root}.value()");
    };
    let first = rust_identifier_from_segment(first, "value");
    let rest = rest
        .iter()
        .map(|segment| rust_identifier_from_segment(segment, "value"))
        .collect::<Vec<_>>();

    if scope.is_local(&first) {
        if rest.is_empty() {
            first.clone()
        } else {
            format!("{first}.{}", rest.join("."))
        }
    } else if rest.is_empty() {
        format!("{root}.{first}()")
    } else {
        format!("{root}.{first}().{}", rest.join("."))
    }
}

fn rust_path_call_expression(
    segments: &[CompactString],
    scope: &RenderScope,
    root: &str,
    arguments: &str,
) -> String {
    let Some((first, rest)) = segments.split_first() else {
        return format!("{root}.value({arguments})");
    };
    let first = rust_identifier_from_segment(first, "value");
    let rest = rest
        .iter()
        .map(|segment| rust_identifier_from_segment(segment, "value"))
        .collect::<Vec<_>>();

    if scope.is_local(&first) {
        if rest.is_empty() {
            format!("{first}({arguments})")
        } else {
            format!("{first}.{}({arguments})", rest.join("."))
        }
    } else if rest.is_empty() {
        format!("{root}.{first}({arguments})")
    } else {
        format!("{root}.{first}().{}({arguments})", rest.join("."))
    }
}

fn rust_literal_expression(literal: &ExprLiteral) -> String {
    match literal {
        ExprLiteral::String(value) => rust_string(value),
        ExprLiteral::Bool(value) => value.to_string(),
        ExprLiteral::Number(value) => value.to_string(),
        ExprLiteral::Null => "None".to_owned(),
    }
}

fn template_action_call(
    action: &ActionBinding,
    scope: &RenderScope,
    event_argument: &str,
) -> Option<String> {
    let expression = action.template.as_ref()?.single_expression()?;
    template_handler_call(expression, scope, event_argument)
}

fn template_handler_call(
    expression: &Expr,
    scope: &RenderScope,
    event_argument: &str,
) -> Option<String> {
    template_handler_call_with_context(expression, scope, event_argument, "_window", "_cx")
}

fn template_handler_call_with_context(
    expression: &Expr,
    scope: &RenderScope,
    event_argument: &str,
    window_argument: &str,
    cx_argument: &str,
) -> Option<String> {
    let segments = match expression {
        Expr::Path(segments) => segments.as_slice(),
        Expr::Call { callee, arguments } if arguments.is_empty() => {
            if let Expr::Path(segments) = callee.as_ref() {
                segments.as_slice()
            } else {
                return None;
            }
        }
        Expr::Opaque(value) => {
            let function = rust_identifier_from_segment(value, "handle_event");
            return Some(format!(
                "this.{function}({event_argument}, {window_argument}, {cx_argument});"
            ));
        }
        Expr::Literal(_)
        | Expr::Call { .. }
        | Expr::Member { .. }
        | Expr::Index { .. }
        | Expr::Binary { .. }
        | Expr::Logical { .. }
        | Expr::Conditional { .. }
        | Expr::TemplateLiteral { .. }
        | Expr::Array(_)
        | Expr::Object(_) => return None,
    };
    let (first, rest) = segments.split_first()?;
    let arguments = format!("{event_argument}, {window_argument}, {cx_argument}");
    let first = rust_identifier_from_segment(first, "handle_event");
    let rest = rest
        .iter()
        .map(|segment| rust_identifier_from_segment(segment, "handle_event"))
        .collect::<Vec<_>>();

    if scope.is_local(&first) {
        if rest.is_empty() {
            Some(format!("{first}({arguments});"))
        } else {
            Some(format!("{first}.{}({arguments});", rest.join(".")))
        }
    } else if rest.is_empty() {
        Some(format!("this.{first}({arguments});"))
    } else {
        Some(format!("this.{first}().{}({arguments});", rest.join(".")))
    }
}

fn action_local_captures(action: &ActionBinding, scope: &RenderScope) -> Vec<String> {
    let mut captures = Vec::new();
    let mut seen = BTreeSet::new();

    if let Some(template) = &action.template {
        for expression in template.expressions() {
            collect_expr_local_captures(expression, scope, &mut seen, &mut captures);
        }
    }

    if let Some(action) = &action.action {
        collect_path_string_local_capture(action, scope, &mut seen, &mut captures);
    }

    for invocation in &action.handler.invocations {
        collect_path_string_local_capture(&invocation.action, scope, &mut seen, &mut captures);
        for argument in &invocation.arguments {
            collect_action_argument_local_captures(argument, scope, &mut seen, &mut captures);
        }
    }

    captures
}

fn collect_expr_local_captures(
    expression: &Expr,
    scope: &RenderScope,
    seen: &mut BTreeSet<String>,
    captures: &mut Vec<String>,
) {
    match expression {
        Expr::Path(segments) => {
            collect_path_segments_local_capture(segments, scope, seen, captures)
        }
        Expr::Call { callee, arguments } => {
            collect_expr_local_captures(callee, scope, seen, captures);
            for argument in arguments {
                collect_expr_local_captures(argument, scope, seen, captures);
            }
        }
        Expr::Member { object, .. } => {
            collect_expr_local_captures(object, scope, seen, captures);
        }
        Expr::Index { object, index } => {
            collect_expr_local_captures(object, scope, seen, captures);
            collect_expr_local_captures(index, scope, seen, captures);
        }
        Expr::Binary { left, right, .. } | Expr::Logical { left, right, .. } => {
            collect_expr_local_captures(left, scope, seen, captures);
            collect_expr_local_captures(right, scope, seen, captures);
        }
        Expr::Conditional {
            test,
            consequent,
            alternate,
        } => {
            collect_expr_local_captures(test, scope, seen, captures);
            collect_expr_local_captures(consequent, scope, seen, captures);
            collect_expr_local_captures(alternate, scope, seen, captures);
        }
        Expr::TemplateLiteral { segments } => {
            for segment in segments {
                if let TemplateSegment::Expression(expression) = segment {
                    collect_expr_local_captures(expression, scope, seen, captures);
                }
            }
        }
        Expr::Array(items) => {
            for item in items {
                collect_expr_local_captures(item, scope, seen, captures);
            }
        }
        Expr::Object(entries) => {
            for entry in entries {
                collect_expr_local_captures(&entry.value, scope, seen, captures);
            }
        }
        Expr::Literal(_) | Expr::Opaque(_) => {}
    }
}

fn collect_action_argument_local_captures(
    argument: &RenderActionArgument,
    scope: &RenderScope,
    seen: &mut BTreeSet<String>,
    captures: &mut Vec<String>,
) {
    if let RenderActionArgument::Unknown(value) = argument {
        collect_path_string_local_capture(value, scope, seen, captures);
    }
}

fn collect_path_segments_local_capture(
    segments: &[CompactString],
    scope: &RenderScope,
    seen: &mut BTreeSet<String>,
    captures: &mut Vec<String>,
) {
    let Some(first) = segments.first() else {
        return;
    };
    push_local_capture(
        rust_identifier_from_segment(first, "value"),
        scope,
        seen,
        captures,
    );
}

fn collect_path_string_local_capture(
    value: &str,
    scope: &RenderScope,
    seen: &mut BTreeSet<String>,
    captures: &mut Vec<String>,
) {
    let first = value
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .find(|segment| !segment.is_empty());
    let Some(first) = first else {
        return;
    };
    push_local_capture(
        rust_identifier_from_segment(first, "value"),
        scope,
        seen,
        captures,
    );
}

fn push_local_capture(
    local: String,
    scope: &RenderScope,
    seen: &mut BTreeSet<String>,
    captures: &mut Vec<String>,
) {
    if scope.is_local(&local) && seen.insert(local.clone()) {
        captures.push(local);
    }
}

fn wrap_listener_with_local_captures(captures: &[String], listener: String) -> String {
    if captures.is_empty() {
        return listener;
    }

    let listener = rewrite_local_capture_references(&listener, captures);
    let mut output = "{\n".to_owned();
    output.push_str(&indent_expression(
        &local_capture_binding_statements(captures),
        1,
    ));
    output.push('\n');
    output.push_str(&indent_expression(&listener, 1));
    output.push_str("\n}");
    output
}

fn rewrite_local_capture_references(source: &str, captures: &[String]) -> String {
    if captures.is_empty() || source.is_empty() {
        return source.to_owned();
    }

    let mut output = String::with_capacity(source.len());
    let mut index = 0;
    while index < source.len() {
        if let Some((capture, alias)) = captures.iter().find_map(|capture| {
            source[index..]
                .starts_with(capture)
                .then(|| (capture, local_capture_alias(capture)))
                .filter(|(capture, _)| {
                    rust_identifier_reference_has_boundary(source, index, index + capture.len())
                })
        }) {
            output.push_str(&alias);
            index += capture.len();
            continue;
        }

        let ch = source[index..]
            .chars()
            .next()
            .expect("index is inside a valid string slice");
        output.push(ch);
        index += ch.len_utf8();
    }
    output
}

fn local_capture_alias(capture: &str) -> String {
    let capture = capture.strip_prefix("r#").unwrap_or(capture);
    format!("htmlswap_capture_{capture}")
}

fn rust_identifier_reference_has_boundary(source: &str, start: usize, end: usize) -> bool {
    let before = source[..start].chars().next_back();
    let after = source[end..].chars().next();
    !before.is_some_and(is_rust_identifier_continue)
        && !after.is_some_and(is_rust_identifier_continue)
}

fn is_rust_identifier_continue(ch: char) -> bool {
    ch == '_' || ch == '#' || ch.is_ascii_alphanumeric()
}

fn rust_identifier_from_segment(segment: &str, fallback: &str) -> String {
    let snake = segment.to_snake_case();
    if RUST_KEYWORDS.contains(&snake.as_str()) {
        return format!("r#{snake}");
    }

    sanitize_rust_identifier(&snake, fallback)
}

fn format_action_comment(action: &ActionBinding) -> String {
    if let Some(template) = &action.template {
        return format!(
            "template action: {}={}",
            action.event,
            format_dynamic_expression(template)
        );
    }

    format!("unmapped action: {}={}", action.event, action.expression)
}

fn action_payload_comment(payload: &ActionPayload) -> Option<String> {
    match payload {
        ActionPayload::None => None,
        ActionPayload::ElementState { state_id } => {
            Some(format!("action payload: element state `{state_id}`"))
        }
        ActionPayload::FormData { controls } => Some(format!(
            "action payload: form data [{}]",
            controls
                .iter()
                .map(|control| {
                    let state = control
                        .state_id
                        .as_ref()
                        .map_or_else(|| "static".to_owned(), |state| format!("state `{state}`"));
                    format!("{} ({:?}, {state})", control.name, control.control_type)
                })
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn gpui_event_method(event: &str, emission: ActionEmission) -> Option<&'static str> {
    match (emission, event) {
        (ActionEmission::Events, "click") => Some("on_click"),
        (ActionEmission::Events, "dblclick" | "doubleclick") => Some("on_click"),
        (ActionEmission::Events, "mouseenter" | "mouseover" | "mouseleave") => Some("on_hover"),
        (ActionEmission::Events, "mousedown") => Some("on_any_mouse_down"),
        (ActionEmission::Events, "mousemove") => Some("on_mouse_move"),
        (ActionEmission::Events, "wheel") => Some("on_scroll_wheel"),
        (ActionEmission::Events, "keydown") => Some("on_key_down"),
        (ActionEmission::Events, "keyup") => Some("on_key_up"),
        (ActionEmission::Events, "input") => Some("on_input"),
        (ActionEmission::Events, "change") => Some("on_change"),
        (ActionEmission::Events, "submit") => Some("on_submit"),
        (ActionEmission::Comment, _)
        | (ActionEmission::Delegate, _)
        | (ActionEmission::Events, _) => None,
    }
}

fn is_double_click_event(event: &str) -> bool {
    matches!(event, "dblclick" | "doubleclick")
}

fn listener_event_argument(method_name: &str) -> &'static str {
    if method_name == "on_hover" {
        "is_hovered"
    } else {
        "_event"
    }
}

fn state_value_source_for_element(element: &RenderElement) -> RenderStateValueSource {
    if let Some(control) = &element.form_control
        && matches!(
            control.control_type,
            RenderFormControlType::Checkbox | RenderFormControlType::Radio
        )
    {
        return RenderStateValueSource::EventChecked;
    }

    RenderStateValueSource::EventValue
}

fn dynamic_state_key_expression(state_id: &str, scope: &RenderScope) -> String {
    if scope.loop_indices.is_empty() {
        return rust_string(state_id);
    }

    let mut pattern = rust_format_literal(state_id);
    for _ in &scope.loop_indices {
        pattern.push_str(":{}");
    }
    format!(
        "format!({}, {})",
        rust_string(&pattern),
        scope.loop_indices.join(", ")
    )
}

fn gpui_generated_element_id_expression(id: &str, scope: &RenderScope) -> String {
    if scope.loop_indices.is_empty() {
        return rust_string(id);
    }

    let mut pattern = rust_format_literal(id);
    for _ in &scope.loop_indices {
        pattern.push_str(":{}");
    }
    format!(
        "format!({}, {})",
        rust_string(&pattern),
        scope.loop_indices.join(", ")
    )
}

fn rust_format_literal(value: &str) -> String {
    value.replace('{', "{{").replace('}', "}}")
}

fn text_input_render_value_expression(input: &RenderTextInputState, scope: &RenderScope) -> String {
    if let Some(template) = &input.initial_template {
        return template_text_expression(template, scope);
    }

    input
        .initial_value
        .as_ref()
        .map_or_else(|| "String::new()".to_owned(), |value| rust_string(value))
}

fn text_input_placeholder_expression(input: &RenderTextInputState, scope: &RenderScope) -> String {
    if let Some(template) = &input.placeholder_template {
        return template_text_expression(template, scope);
    }

    input
        .placeholder
        .as_ref()
        .map_or_else(|| "String::new()".to_owned(), |value| rust_string(value))
}

fn element_action_local_captures(element: &RenderElement, scope: &RenderScope) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut captures = Vec::new();
    for action in &element.actions {
        for capture in action_local_captures(action, scope) {
            if seen.insert(capture.clone()) {
                captures.push(capture);
            }
        }
    }

    captures
}

fn local_capture_binding_statements(captures: &[String]) -> String {
    captures
        .iter()
        .map(|capture| format!("let {} = {capture}.clone();", local_capture_alias(capture)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_float(value: f32) -> String {
    let mut formatted = format!("{value:.3}");
    while formatted.contains('.') && formatted.ends_with('0') {
        formatted.pop();
    }
    if formatted.ends_with('.') {
        formatted.push('0');
    }
    formatted
}

fn action_call(action: &ActionBinding, event_argument: &str) -> Option<String> {
    action_call_with_arguments(action, event_argument, "_event.into()", "_event")
}

fn input_subscription_action_call(
    action: &ActionBinding,
    scope: &RenderScope,
    value_argument: &str,
) -> Option<String> {
    if let Some(template) = &action.template {
        let expression = template.single_expression()?;
        return template_handler_call_with_context(
            expression,
            scope,
            value_argument,
            "window",
            "cx",
        );
    }

    action_call_with_arguments(action, value_argument, value_argument, value_argument)
}

fn action_call_with_arguments(
    action: &ActionBinding,
    event_argument: &str,
    element_value_argument: &str,
    element_checked_argument: &str,
) -> Option<String> {
    let invocation = action.handler.primary_invocation();
    if invocation.is_some_and(|invocation| !invocation.resolved)
        || invocation.is_none() && !action.resolved
    {
        return None;
    }

    let target = invocation
        .map(|invocation| invocation.action.as_str())
        .or(action.action.as_deref())?;
    if !is_rust_identifier(target) {
        return None;
    }

    let arguments = invocation.map_or_else(
        || Some(Vec::new()),
        |invocation| {
            invocation
                .arguments
                .iter()
                .map(|argument| {
                    gpui_action_argument(
                        argument,
                        event_argument,
                        element_value_argument,
                        element_checked_argument,
                    )
                })
                .collect::<Option<Vec<_>>>()
        },
    )?;
    Some(format!("{target}({});", arguments.join(", ")))
}

fn gpui_action_argument(
    argument: &RenderActionArgument,
    event_argument: &str,
    element_value_argument: &str,
    element_checked_argument: &str,
) -> Option<String> {
    match argument {
        RenderActionArgument::Event => Some(event_argument.to_owned()),
        RenderActionArgument::Element => Some("this".to_owned()),
        RenderActionArgument::ElementValue => Some(element_value_argument.to_owned()),
        RenderActionArgument::ElementChecked => Some(element_checked_argument.to_owned()),
        RenderActionArgument::Literal(value) => Some(rust_string(value)),
        RenderActionArgument::Unknown(_) => None,
    }
}

fn collect_state_bindings(plan: &RenderPlan) -> BTreeMap<CompactString, RenderStateBinding> {
    plan.state
        .bindings
        .iter()
        .map(|binding| (binding.id.clone(), binding.clone()))
        .collect()
}

fn state_fields(
    bindings: &BTreeMap<CompactString, RenderStateBinding>,
) -> BTreeMap<CompactString, String> {
    let mut used = BTreeSet::new();
    bindings
        .iter()
        .filter(|(_, binding)| state_needs_gpui_field(binding))
        .map(|(id, _)| (id.clone(), state_field_name(id, &mut used)))
        .collect()
}

fn state_needs_gpui_field(binding: &RenderStateBinding) -> bool {
    binding.owner == RenderStateOwner::Target
        || (binding.owner == RenderStateOwner::Source
            && matches!(&binding.kind, RenderStateKind::TextInput(_)))
}

fn state_field_name(id: &str, used: &mut BTreeSet<String>) -> String {
    let mut field = id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    while field.contains("__") {
        field = field.replace("__", "_");
    }
    field = field.trim_matches('_').to_owned();
    if field.is_empty() || field.as_bytes()[0].is_ascii_digit() || !is_rust_identifier(&field) {
        field = format!("input_{field}");
        field = field.trim_end_matches('_').to_owned();
    }

    if used.insert(field.clone()) {
        return field;
    }

    let base = field;
    let mut suffix = 2;
    loop {
        let candidate = format!("{base}_{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
}

fn state_field_type(state: &RenderStateBinding, backend: GpuiStateFieldBackend) -> &'static str {
    match &state.kind {
        RenderStateKind::TextInput(_) => match backend {
            GpuiStateFieldBackend::ComponentTextInput => {
                "gpui::Entity<gpui_component::input::InputState>"
            }
            GpuiStateFieldBackend::GpuiTextInput => "gpui::Entity<HtmlswapGpuiTextInput>",
        },
        RenderStateKind::Choice(_) => {
            "gpui::Entity<gpui_component::select::SelectState<gpui_component::select::SearchableVec<String>>>"
        }
        RenderStateKind::Toggle(_) => "bool",
    }
}

fn state_uses_entity(state: &RenderStateBinding) -> bool {
    !matches!(&state.kind, RenderStateKind::Toggle(_))
}

fn state_initializer(state: &RenderStateBinding, backend: GpuiStateFieldBackend) -> String {
    match &state.kind {
        RenderStateKind::TextInput(input) => text_input_state_initializer(input, backend),
        RenderStateKind::Choice(choice) => choice_state_initializer(choice),
        RenderStateKind::Toggle(toggle) => toggle_state_initializer(toggle),
    }
}

fn text_input_state_initializer(
    input: &RenderTextInputState,
    backend: GpuiStateFieldBackend,
) -> String {
    if backend == GpuiStateFieldBackend::GpuiTextInput {
        let value = input
            .initial_value
            .as_ref()
            .map_or_else(|| "String::new()".to_owned(), |value| rust_string(value));
        let placeholder = input
            .placeholder
            .as_ref()
            .map_or_else(|| "String::new()".to_owned(), |value| rust_string(value));
        return format!("HtmlswapGpuiTextInput::new({value}, {placeholder}, cx)");
    }

    let mut expression = "gpui_component::input::InputState::new(window, cx)".to_owned();
    if let Some(placeholder) = &input.placeholder {
        push_method(
            &mut expression,
            0,
            &format!(".placeholder({})", rust_string(placeholder)),
        );
    }
    if let Some(initial_value) = &input.initial_value {
        push_method(
            &mut expression,
            0,
            &format!(".default_value({})", rust_string(initial_value)),
        );
    }
    if input.multiline {
        push_method(&mut expression, 0, ".multi_line(true)");
    }
    if let Some(rows) = input.rows {
        push_method(&mut expression, 0, &format!(".rows({rows})"));
    }
    if input.password {
        push_method(&mut expression, 0, ".masked(true)");
    }
    expression
}

fn choice_state_initializer(choice: &RenderChoiceState) -> String {
    let options = choice
        .options
        .iter()
        .map(|option| format!("String::from({})", rust_string(&option.label)))
        .collect::<Vec<_>>()
        .join(", ");
    let selected_index = choice.selected_index.map_or_else(
        || "None".to_owned(),
        |index| format!("Some(gpui_component::IndexPath::new({index}))"),
    );

    format!(
        "gpui_component::select::SelectState::new(gpui_component::select::SearchableVec::new(vec![{options}]), {selected_index}, window, cx)"
    )
}

fn toggle_state_initializer(toggle: &RenderToggleState) -> String {
    toggle.checked.to_string()
}

fn is_legend_child(child: &RenderNode) -> bool {
    matches!(child, RenderNode::Element(element) if element.role == UiRole::Legend)
}

fn is_consumed_form_label(form: &RenderElement, child: &RenderNode) -> bool {
    let RenderNode::Element(label) = child else {
        return false;
    };

    if label.source_tag != "label" {
        return false;
    }

    let Some(target_id) = attribute_value(label, "for") else {
        return false;
    };

    form.children
        .iter()
        .any(|node| form_node_has_control_id(node, target_id))
}

fn form_node_has_control_id(node: &RenderNode, target_id: &str) -> bool {
    let RenderNode::Element(element) = node else {
        return false;
    };

    element.form_control.is_some() && element_id(element) == Some(target_id)
        || element
            .children
            .iter()
            .any(|child| form_node_has_control_id(child, target_id))
}

fn route_targets(element: &RenderElement) -> SmallVec<[RouteTargetRef<'_>; 8]> {
    let mut targets = SmallVec::with_capacity(2 + element.styles.len() + element.actions.len());
    if let Some(component) = element
        .source_intent
        .as_ref()
        .and_then(|hints| hints.component.as_ref())
    {
        targets.push(RouteTargetRef::Component(component));
    }
    if let Some(region) = &element.region {
        targets.push(RouteTargetRef::Region(region));
    }
    targets.push(RouteTargetRef::Role(&element.role));
    targets.extend(
        element
            .classes
            .iter()
            .map(|class| RouteTargetRef::Class(class.as_str())),
    );
    targets.push(RouteTargetRef::Tag(&element.source_tag));
    targets.extend(
        element
            .styles
            .iter()
            .map(|style| RouteTargetRef::Style(&style.property)),
    );
    targets.extend(
        element
            .actions
            .iter()
            .map(|action| RouteTargetRef::Event(&action.event)),
    );
    targets
}

fn is_title_bar_window_control_child(node: &RenderNode) -> bool {
    let RenderNode::Element(element) = node else {
        return false;
    };

    if let Some(hints) = element.source_intent.as_ref()
        && hints.slot.as_ref().is_some_and(|slot| {
            slot.is("window-controls") || slot.is("titlebar-controls") || slot.is("traffic-lights")
        })
    {
        return true;
    }

    looks_like_title_bar_window_controls(element)
}

fn looks_like_title_bar_window_controls(element: &RenderElement) -> bool {
    if !(2..=4).contains(&element.children.len()) {
        return false;
    }

    if !has_style_value(element, StyleProperty::MarginLeft, "auto") {
        return false;
    }

    let labels = element
        .children
        .iter()
        .filter_map(window_control_child_label)
        .collect::<Vec<_>>();
    labels.len() == element.children.len()
        && labels
            .iter()
            .filter(|label| is_window_control_label(label))
            .count()
            >= 2
}

fn window_control_child_label(node: &RenderNode) -> Option<String> {
    let mut visible_text = String::new();
    collect_visible_text_content(node, &mut visible_text);

    let mut candidates = Vec::new();
    let visible_label = normalize_window_control_label(&visible_text);
    if !visible_label.is_empty() {
        candidates.push(visible_label);
    }
    collect_aria_label_candidates(node, &mut candidates);

    candidates
        .iter()
        .find(|candidate| is_window_control_label(candidate))
        .cloned()
        .or_else(|| candidates.into_iter().next())
}

fn collect_visible_text_content(node: &RenderNode, output: &mut String) {
    match node {
        RenderNode::Element(element) => {
            for child in &element.children {
                collect_visible_text_content(child, output);
            }
        }
        RenderNode::Text(text) => {
            if text.template.is_none() {
                output.push_str(&text.value);
            }
        }
        RenderNode::Raw(_) => {}
    }
}

fn collect_aria_label_candidates(node: &RenderNode, candidates: &mut Vec<String>) {
    let RenderNode::Element(element) = node else {
        return;
    };

    if let Some(label) = attribute_value(element, "aria-label") {
        let normalized = normalize_window_control_label(label);
        if !normalized.is_empty() {
            candidates.push(normalized);
        }
    }

    for child in &element.children {
        collect_aria_label_candidates(child, candidates);
    }
}

fn normalize_window_control_label(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

fn is_window_control_label(label: &str) -> bool {
    matches!(
        label,
        "close"
            | "closewindow"
            | "windowclose"
            | "remove"
            | "minimize"
            | "minimise"
            | "minimizewindow"
            | "minimisewindow"
            | "windowminimize"
            | "windowminimise"
            | "cropsquare"
            | "maximize"
            | "maximise"
            | "maximizewindow"
            | "maximisewindow"
            | "windowmaximize"
            | "windowmaximise"
            | "restore"
            | "restorewindow"
            | "windowrestore"
            | "fullscreen"
            | "openinfull"
            | "fullscreenexit"
    )
}

fn has_style_value(element: &RenderElement, property: StyleProperty, expected: &str) -> bool {
    element
        .styles
        .iter()
        .any(|style| style.property == property && style.value.as_str().trim() == expected)
}

pub(crate) fn text_only_children(element: &RenderElement) -> Option<String> {
    if element.children.is_empty() {
        return None;
    }

    let mut value = String::new();
    for child in &element.children {
        let RenderNode::Text(text) = child else {
            return None;
        };
        value.push_str(&text.value);
    }

    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn dynamic_text_only_children(element: &RenderElement, scope: &RenderScope) -> Option<String> {
    if element.children.is_empty() {
        return None;
    }

    let [RenderNode::Text(text)] = element.children.as_slice() else {
        return None;
    };

    text.template.as_ref().map(|template| {
        apply_text_transform_expression(
            template_text_expression(template, scope),
            scope.text_transform,
        )
    })
}

fn text_children_span(element: &RenderElement) -> Option<Span> {
    let mut span: Option<Span> = None;
    for child in &element.children {
        let RenderNode::Text(text) = child else {
            return None;
        };
        let child_span = text.span?;
        span = Some(match span {
            None => child_span,
            Some(current) if current.source == child_span.source => Span::new(
                current.source,
                current.start.min(child_span.start),
                current.end.max(child_span.end),
            ),
            Some(_) => return None,
        });
    }

    span
}

pub(crate) fn attribute_value<'a>(element: &'a RenderElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .map(|attribute| attribute.value.as_str())
}

pub(crate) fn attribute_span(element: &RenderElement, name: &str) -> Option<Span> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .and_then(|attribute| attribute.span)
}

pub(crate) fn has_boolean_attribute(element: &RenderElement, name: &str) -> bool {
    element
        .attributes
        .iter()
        .any(|attribute| attribute.name == name)
}

pub(crate) fn element_id(element: &RenderElement) -> Option<&str> {
    attribute_value(element, "id").filter(|value| !value.trim().is_empty())
}

fn push_method(expression: &mut String, base_depth: usize, method: &str) {
    expression.push('\n');
    expression.push_str(&indent(base_depth + 1));
    expression.push_str(method);
}

fn indent_expression(expression: &str, depth: usize) -> String {
    let line_indent = indent(depth);
    expression
        .lines()
        .map(|line| format!("{line_indent}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn indent(depth: usize) -> String {
    "    ".repeat(depth)
}

pub(crate) fn rust_string(value: &str) -> String {
    format!("{value:?}")
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn source_html_start_tag(source: &str) -> Option<&str> {
    let source = source.trim_start();
    if !source.starts_with('<') {
        return None;
    }

    let mut quote = None;
    for (index, ch) in source.char_indices() {
        match quote {
            Some(quoted) if ch == quoted => quote = None,
            Some(_) => {}
            None if matches!(ch, '"' | '\'') => quote = Some(ch),
            None if ch == '>' => return source.get(..index + ch.len_utf8()),
            None => {}
        }
    }

    None
}

fn is_rust_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        && !RUST_KEYWORDS.contains(&value)
}

const RUST_KEYWORDS: &[&str] = &[
    "Self", "as", "async", "await", "box", "break", "const", "continue", "crate", "do", "dyn",
    "else", "enum", "extern", "false", "fn", "for", "gen", "if", "impl", "in", "let", "loop",
    "macro", "match", "mod", "move", "mut", "priv", "pub", "ref", "return", "self", "static",
    "struct", "super", "trait", "true", "try", "type", "typeof", "unsafe", "unsized", "use",
    "virtual", "where", "while", "yield",
];
