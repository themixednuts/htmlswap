use compact_str::CompactString;
use heck::ToLowerCamelCase;
use oxc_allocator::Allocator;
use oxc_parser::{ParseOptions, Parser};
use oxc_span::SourceType;
use smallvec::SmallVec;

use crate::diagnostics::{Diagnostic, Diagnostics};
use crate::expr::{BindingPattern, Expr, ExprLiteral, TemplateSegment, TemplateString};
use crate::ir::{HtmlAttribute, HtmlComment, HtmlElement, HtmlName, HtmlNode};
use crate::plan::{ComponentId, RenderControlFlow, RenderControlFlowHost, RenderControlFlowKind};
use crate::source::Span;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttributeRewrite {
    attributes: SmallVec<[HtmlAttribute; 2]>,
}

impl AttributeRewrite {
    #[must_use]
    pub fn remove() -> Self {
        Self {
            attributes: SmallVec::new(),
        }
    }

    #[must_use]
    pub fn replace(attribute: HtmlAttribute) -> Self {
        let mut attributes: SmallVec<[HtmlAttribute; 2]> = SmallVec::new();
        attributes.push(attribute);
        Self { attributes }
    }

    #[must_use]
    pub fn with_attributes(attributes: impl IntoIterator<Item = HtmlAttribute>) -> Self {
        Self {
            attributes: attributes.into_iter().collect(),
        }
    }

    #[must_use]
    pub fn attributes(&self) -> &[HtmlAttribute] {
        &self.attributes
    }

    #[must_use]
    pub fn into_attributes(self) -> SmallVec<[HtmlAttribute; 2]> {
        self.attributes
    }
}

pub trait SourceDialect: Send + Sync {
    fn name(&self) -> &'static str;

    fn rewrite_attribute(
        &self,
        _element: &HtmlElement,
        _attribute: &HtmlAttribute,
        _cx: &mut DialectContext<'_>,
    ) -> Option<AttributeRewrite> {
        None
    }

    fn infer_element_attributes(
        &self,
        _element: &HtmlElement,
        _siblings: &[HtmlNode],
        _sibling_index: usize,
        _cx: &mut DialectContext<'_>,
    ) -> SmallVec<[HtmlAttribute; 2]> {
        SmallVec::new()
    }

    fn lower_element(
        &self,
        _tag: &str,
        _element: &HtmlElement,
        _cx: &mut DialectContext<'_>,
    ) -> ElementDirectives {
        ElementDirectives::default()
    }

    fn parse_template(
        &self,
        _raw: &str,
        _span: Option<Span>,
        _cx: &mut DialectContext<'_>,
    ) -> Option<TemplateString> {
        None
    }

    fn unwrap_root_element(&self, _element: &HtmlElement) -> bool {
        false
    }
}

pub struct Frontend {
    dialects: Vec<Box<dyn SourceDialect>>,
}

impl Frontend {
    #[must_use]
    pub fn new() -> Self {
        Self {
            dialects: Vec::new(),
        }
    }

    #[must_use]
    pub fn html() -> Self {
        Self::new()
    }

    #[must_use]
    pub fn dc() -> Self {
        Self::new().with_dialect(DcDialect)
    }

    #[must_use]
    pub fn vue() -> Self {
        Self::new().with_dialect(VueDialect)
    }

    #[must_use]
    pub fn with_dialect(mut self, dialect: impl SourceDialect + 'static) -> Self {
        self.dialects.push(Box::new(dialect));
        self
    }

    #[must_use]
    pub fn dialects(&self) -> &[Box<dyn SourceDialect>] {
        &self.dialects
    }

    #[must_use]
    pub fn has_dialects(&self) -> bool {
        !self.dialects.is_empty()
    }

    pub fn rewrite_attribute(
        &self,
        element: &HtmlElement,
        attribute: &HtmlAttribute,
        cx: &mut DialectContext<'_>,
    ) -> Option<AttributeRewrite> {
        self.dialects
            .iter()
            .find_map(|dialect| dialect.rewrite_attribute(element, attribute, cx))
    }

    pub fn infer_element_attributes(
        &self,
        element: &HtmlElement,
        siblings: &[HtmlNode],
        sibling_index: usize,
        cx: &mut DialectContext<'_>,
    ) -> SmallVec<[HtmlAttribute; 4]> {
        self.dialects
            .iter()
            .flat_map(|dialect| {
                dialect
                    .infer_element_attributes(element, siblings, sibling_index, cx)
                    .into_iter()
            })
            .collect()
    }

    pub fn lower_element(
        &self,
        tag: &str,
        element: &HtmlElement,
        cx: &mut DialectContext<'_>,
    ) -> ElementDirectives {
        let mut output = ElementDirectives::default();
        for dialect in &self.dialects {
            let directives = dialect.lower_element(tag, element, cx);
            if output.control_flow.is_some() && directives.control_flow.is_some() {
                cx.warn(format!(
                    "source dialect `{}` produced control flow for an element already claimed by an earlier dialect",
                    dialect.name()
                ), element.span);
                output
                    .consumed_attributes
                    .extend(directives.consumed_attributes);
                continue;
            }

            output.merge(directives);
        }

        output
    }

    pub fn parse_template(
        &self,
        raw: &str,
        span: Option<Span>,
        cx: &mut DialectContext<'_>,
    ) -> Option<TemplateString> {
        self.dialects
            .iter()
            .find_map(|dialect| dialect.parse_template(raw, span, cx))
    }

    pub fn unwrap_root_element(&self, element: &HtmlElement) -> bool {
        self.dialects
            .iter()
            .any(|dialect| dialect.unwrap_root_element(element))
    }
}

impl Default for Frontend {
    fn default() -> Self {
        Self::html()
    }
}

impl std::fmt::Debug for Frontend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frontend")
            .field(
                "dialects",
                &self
                    .dialects
                    .iter()
                    .map(|dialect| dialect.name())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

pub struct DialectContext<'a> {
    diagnostics: &'a mut Diagnostics,
}

impl<'a> DialectContext<'a> {
    pub fn new(diagnostics: &'a mut Diagnostics) -> Self {
        Self { diagnostics }
    }

    pub fn warn(&mut self, message: impl Into<String>, span: Option<Span>) {
        self.diagnostics.push(Diagnostic::warning(message, span));
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ElementDirectives {
    pub control_flow: Option<RenderControlFlow>,
    pub consumed_attributes: SmallVec<[CompactString; 12]>,
}

impl ElementDirectives {
    fn merge(&mut self, mut directives: Self) {
        if self.control_flow.is_none() {
            self.control_flow = directives.control_flow.take();
        }
        self.consumed_attributes
            .extend(directives.consumed_attributes);
    }

    #[must_use]
    pub fn consumes_attribute(&self, attribute: &str) -> bool {
        self.consumed_attributes
            .iter()
            .any(|consumed| consumed == attribute)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DcDialect;

impl SourceDialect for DcDialect {
    fn name(&self) -> &'static str {
        "dc"
    }

    fn rewrite_attribute(
        &self,
        element: &HtmlElement,
        attribute: &HtmlAttribute,
        _cx: &mut DialectContext<'_>,
    ) -> Option<AttributeRewrite> {
        let tag = element.name.local().to_ascii_lowercase();
        if tag == "dc-import" {
            return rewrite_dc_import_attribute(element, attribute);
        }
        if tag == "x-import" {
            return rewrite_x_import_attribute(element, attribute);
        }

        if attribute
            .name
            .local()
            .eq_ignore_ascii_case("data-htmlswap-component")
            && ComponentId::canonical(attribute.value.as_str()) == "menubar"
            && html_attribute(element, "data-htmlswap-region")
                .is_some_and(|region| region.value.eq_ignore_ascii_case("app.window"))
        {
            return Some(AttributeRewrite::replace(attribute_with_value(
                "data-htmlswap-component",
                "titlebar",
                attribute.span,
            )));
        }

        if !attribute.name.local().eq_ignore_ascii_case("class") {
            return None;
        }

        let classes = attribute
            .value
            .split_whitespace()
            .map(ComponentId::canonical)
            .collect::<Vec<_>>();
        let mut attributes: SmallVec<[HtmlAttribute; 3]> = SmallVec::new();
        attributes.push(attribute.clone());

        if classes.iter().any(|class| class == "titlebar")
            && html_attribute(element, "data-htmlswap-component").is_none()
        {
            attributes.push(attribute_with_value(
                "data-htmlswap-component",
                "titlebar",
                attribute.span,
            ));
        }

        if classes.iter().any(|class| {
            matches!(
                class.as_str(),
                "windowcontrols" | "titlebarcontrols" | "trafficlights"
            )
        }) && html_attribute(element, "data-htmlswap-slot").is_none()
        {
            attributes.push(attribute_with_value(
                "data-htmlswap-slot",
                "window-controls",
                attribute.span,
            ));
        }

        (attributes.len() > 1).then(|| AttributeRewrite::with_attributes(attributes))
    }

    fn infer_element_attributes(
        &self,
        element: &HtmlElement,
        siblings: &[HtmlNode],
        sibling_index: usize,
        _cx: &mut DialectContext<'_>,
    ) -> SmallVec<[HtmlAttribute; 2]> {
        let mut attributes = SmallVec::new();
        let Some(comment) = preceding_significant_comment(siblings, sibling_index) else {
            return attributes;
        };

        let comment_component = ComponentId::canonical(comment.value.trim());
        if comment_component == "titlebar"
            && html_attribute(element, "data-htmlswap-component").is_none()
        {
            attributes.push(attribute_with_value(
                "data-htmlswap-component",
                "titlebar",
                comment.span.or(element.span),
            ));
        }

        attributes
    }

    fn lower_element(
        &self,
        tag: &str,
        element: &HtmlElement,
        cx: &mut DialectContext<'_>,
    ) -> ElementDirectives {
        let Some(kind) = dc_control_flow_kind(tag) else {
            return ElementDirectives::default();
        };

        let expression = match kind {
            RenderControlFlowKind::For => expression_from_attributes(
                element,
                &["list", "items", "of", "value", "each"],
                self,
                cx,
            ),
            RenderControlFlowKind::If | RenderControlFlowKind::ElseIf => {
                expression_from_attributes(
                    element,
                    &["value", "if", "test", "when", "condition"],
                    self,
                    cx,
                )
            }
            RenderControlFlowKind::Switch => {
                expression_from_attributes(element, &["value", "switch", "on", "of"], self, cx)
            }
            RenderControlFlowKind::Case => {
                expression_from_attributes(element, &["value", "case", "when"], self, cx)
            }
            RenderControlFlowKind::Else | RenderControlFlowKind::Default => None,
        };
        let binding = if kind == RenderControlFlowKind::For {
            html_attribute(element, "as")
                .or_else(|| html_attribute(element, "item"))
                .or_else(|| html_attribute(element, "let"))
                .map(|attribute| BindingPattern::new(attribute.value.as_str(), attribute.span))
        } else {
            None
        };
        let placeholder = source_control_flow_placeholder(element);

        ElementDirectives {
            control_flow: Some(RenderControlFlow {
                kind,
                host: RenderControlFlowHost::Wrapper,
                expression,
                binding,
                index_binding: None,
                key: None,
                locals: Vec::new(),
                placeholder,
                span: element.span,
            }),
            consumed_attributes: dc_control_flow_attributes(element),
        }
    }

    fn parse_template(
        &self,
        raw: &str,
        span: Option<Span>,
        cx: &mut DialectContext<'_>,
    ) -> Option<TemplateString> {
        parse_template_with(raw, span, |source| parse_dc_expr(source, span, cx))
    }

    fn unwrap_root_element(&self, element: &HtmlElement) -> bool {
        element.name.local().eq_ignore_ascii_case("x-dc")
    }
}

fn rewrite_dc_import_attribute(
    element: &HtmlElement,
    attribute: &HtmlAttribute,
) -> Option<AttributeRewrite> {
    let raw_name = attribute.name.local();
    let name = raw_name.to_ascii_lowercase();

    if name.starts_with("data-htmlswap-") {
        return None;
    }

    if name == "name" {
        if html_attribute(element, "data-htmlswap-component").is_some() {
            return Some(AttributeRewrite::remove());
        }

        return Some(AttributeRewrite::replace(attribute_with_value(
            "data-htmlswap-component",
            attribute.value.clone(),
            attribute.span,
        )));
    }

    Some(AttributeRewrite::replace(component_prop_attribute(
        raw_name,
        attribute.value.clone(),
        attribute.span,
    )))
}

fn rewrite_x_import_attribute(
    element: &HtmlElement,
    attribute: &HtmlAttribute,
) -> Option<AttributeRewrite> {
    let raw_name = attribute.name.local();
    let name = raw_name.to_ascii_lowercase();

    if name.starts_with("data-htmlswap-") {
        return None;
    }

    match name.as_str() {
        "component" | "component-from-global-scope" => {
            if html_attribute(element, "data-htmlswap-component").is_some() {
                return Some(AttributeRewrite::remove());
            }

            Some(AttributeRewrite::replace(attribute_with_value(
                "data-htmlswap-component",
                attribute.value.clone(),
                attribute.span,
            )))
        }
        "from" => {
            let mut attributes: SmallVec<[HtmlAttribute; 2]> = SmallVec::new();
            if html_attribute(element, "data-htmlswap-component-source").is_none() {
                attributes.push(attribute_with_value(
                    "data-htmlswap-component-source",
                    attribute.value.clone(),
                    attribute.span,
                ));
            }
            attributes.push(component_prop_attribute(
                raw_name,
                attribute.value.clone(),
                attribute.span,
            ));
            Some(AttributeRewrite::with_attributes(attributes))
        }
        _ => Some(AttributeRewrite::replace(component_prop_attribute(
            raw_name,
            attribute.value.clone(),
            attribute.span,
        ))),
    }
}

fn component_prop_attribute(
    name: &str,
    value: impl Into<CompactString>,
    span: Option<Span>,
) -> HtmlAttribute {
    attribute_with_value(
        format!("data-htmlswap-prop-{}", component_prop_name(name)),
        value,
        span,
    )
}

fn component_prop_name(name: &str) -> String {
    if name.starts_with("aria-") || name.starts_with("data-") {
        name.to_ascii_lowercase()
    } else {
        name.to_lower_camel_case()
    }
}

fn preceding_significant_comment(
    siblings: &[HtmlNode],
    sibling_index: usize,
) -> Option<&HtmlComment> {
    for node in siblings[..sibling_index].iter().rev() {
        match node {
            HtmlNode::Text(text) if text.value.trim().is_empty() => continue,
            HtmlNode::Comment(comment) => return Some(comment),
            HtmlNode::Text(_) | HtmlNode::Element(_) => return None,
        }
    }

    None
}

#[derive(Debug, Clone, Copy, Default)]
pub struct VueDialect;

impl SourceDialect for VueDialect {
    fn name(&self) -> &'static str {
        "vue"
    }

    fn rewrite_attribute(
        &self,
        element: &HtmlElement,
        attribute: &HtmlAttribute,
        cx: &mut DialectContext<'_>,
    ) -> Option<AttributeRewrite> {
        rewrite_vue_attribute(element, attribute, cx)
    }

    fn lower_element(
        &self,
        _tag: &str,
        element: &HtmlElement,
        cx: &mut DialectContext<'_>,
    ) -> ElementDirectives {
        vue_control_flow(element, self, cx).unwrap_or_default()
    }

    fn parse_template(
        &self,
        raw: &str,
        span: Option<Span>,
        cx: &mut DialectContext<'_>,
    ) -> Option<TemplateString> {
        parse_template_with(raw, span, |source| parse_vue_expr(source, span, cx))
    }
}

fn dc_control_flow_kind(tag: &str) -> Option<RenderControlFlowKind> {
    match tag {
        "sc-for" => Some(RenderControlFlowKind::For),
        "sc-if" => Some(RenderControlFlowKind::If),
        "sc-elseif" | "sc-else-if" => Some(RenderControlFlowKind::ElseIf),
        "sc-else" => Some(RenderControlFlowKind::Else),
        "sc-switch" => Some(RenderControlFlowKind::Switch),
        "sc-case" => Some(RenderControlFlowKind::Case),
        "sc-default" => Some(RenderControlFlowKind::Default),
        _ => None,
    }
}

fn expression_from_attributes(
    element: &HtmlElement,
    names: &[&str],
    dialect: &DcDialect,
    cx: &mut DialectContext<'_>,
) -> Option<Expr> {
    names.iter().find_map(|name| {
        let attribute = html_attribute(element, name)?;
        expression_from_value(&attribute.value, attribute.span, dialect, cx)
    })
}

fn expression_from_value(
    value: &str,
    span: Option<Span>,
    dialect: &DcDialect,
    cx: &mut DialectContext<'_>,
) -> Option<Expr> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(template) = dialect.parse_template(trimmed, span, cx) {
        return template.single_expression().cloned();
    }

    Some(parse_dc_expr(trimmed, span, cx))
}

fn parse_template_with(
    raw: &str,
    span: Option<Span>,
    mut parse_expr: impl FnMut(&str) -> Expr,
) -> Option<TemplateString> {
    let mut segments = Vec::new();
    let mut remainder = raw;
    let mut found_expression = false;

    while let Some(start) = remainder.find("{{") {
        if start > 0 {
            segments.push(TemplateSegment::Literal(CompactString::from(
                &remainder[..start],
            )));
        }

        let after_start = &remainder[start + 2..];
        let Some(end) = after_start.find("}}") else {
            segments.push(TemplateSegment::Literal(CompactString::from(
                &remainder[start..],
            )));
            return found_expression.then(|| TemplateString::new(raw, segments, span));
        };

        let expression = after_start[..end].trim();
        if !expression.is_empty() {
            found_expression = true;
            segments.push(TemplateSegment::Expression(parse_expr(expression)));
        }
        remainder = &after_start[end + 2..];
    }

    if !remainder.is_empty() {
        segments.push(TemplateSegment::Literal(CompactString::from(remainder)));
    }

    found_expression.then(|| TemplateString::new(raw, segments, span))
}

fn parse_source_expr(source: &str) -> Expr {
    let source = source.trim();
    if source.is_empty() {
        return Expr::Opaque(CompactString::new(""));
    }
    if source == "true" {
        return Expr::Literal(ExprLiteral::Bool(true));
    }
    if source == "false" {
        return Expr::Literal(ExprLiteral::Bool(false));
    }
    if source == "null" {
        return Expr::Literal(ExprLiteral::Null);
    }
    if let Some(value) = quoted_string(source) {
        return Expr::Literal(ExprLiteral::String(CompactString::from(value)));
    }
    if is_number_literal(source) {
        return Expr::Literal(ExprLiteral::Number(CompactString::from(source)));
    }
    if let Some(callee) = source.strip_suffix("()")
        && let Some(path) = path_expr(callee)
    {
        return Expr::Call {
            callee: Box::new(path),
            arguments: Vec::new(),
        };
    }
    if let Some(path) = path_expr(source) {
        return path;
    }

    Expr::Opaque(CompactString::from(source))
}

fn parse_dc_expr(source: &str, span: Option<Span>, cx: &mut DialectContext<'_>) -> Expr {
    let source = source.trim();
    if source.is_empty() {
        return Expr::Opaque(CompactString::new(""));
    }
    if source == "true" {
        return Expr::Literal(ExprLiteral::Bool(true));
    }
    if source == "false" {
        return Expr::Literal(ExprLiteral::Bool(false));
    }
    if source == "null" {
        return Expr::Literal(ExprLiteral::Null);
    }
    if is_number_literal(source) {
        return Expr::Literal(ExprLiteral::Number(CompactString::from(source)));
    }
    if source == "$index" {
        return Expr::Path(vec![CompactString::from("$index")]);
    }
    if let Some(path) = dc_path_expr(source) {
        return path;
    }

    cx.warn(
        format!(
            "DC hole `{}` is not a dotted path or supported literal",
            source
        ),
        span,
    );
    Expr::Opaque(CompactString::from(source))
}

fn parse_vue_expr(source: &str, span: Option<Span>, cx: &mut DialectContext<'_>) -> Expr {
    let expression = parse_source_expr(source);
    if matches!(expression, Expr::Opaque(_)) {
        let message = if vue_expression_is_valid(source) {
            format!(
                "Vue expression `{}` is valid JavaScript/TypeScript but cannot be represented as a portable htmlswap expression yet",
                source.trim()
            )
        } else {
            format!(
                "Vue expression `{}` is not valid JavaScript/TypeScript",
                source.trim()
            )
        };
        cx.warn(message, span);
    }
    expression
}

fn vue_expression_is_valid(source: &str) -> bool {
    let wrapped = format!("const __htmlswap = ({});", source.trim());
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, &wrapped, SourceType::ts())
        .with_options(ParseOptions {
            parse_regular_expression: true,
            ..ParseOptions::default()
        })
        .parse();
    !parsed.panicked && parsed.diagnostics.is_empty()
}

fn rewrite_vue_attribute(
    element: &HtmlElement,
    attribute: &HtmlAttribute,
    cx: &mut DialectContext<'_>,
) -> Option<AttributeRewrite> {
    let raw_name = attribute.name.local();
    let name = raw_name.to_ascii_lowercase();

    if let Some(rest) = raw_name.strip_prefix('@') {
        return vue_event_attribute(rest, attribute, cx);
    }

    if let Some(rest) = raw_name.strip_prefix(':') {
        return vue_binding_attribute(rest, attribute, cx);
    }

    if let Some(rest) = name.strip_prefix("v-on:") {
        return vue_event_attribute(rest, attribute, cx);
    }

    if name == "v-on" {
        return Some(preserve_vue_directive_attribute(
            "data-htmlswap-v-on",
            attribute,
            Some("Vue object event bindings are preserved for adapters"),
            cx,
        ));
    }

    if let Some(rest) = name.strip_prefix("v-bind:") {
        return vue_binding_attribute(rest, attribute, cx);
    }

    if name == "v-bind" {
        return Some(preserve_vue_directive_attribute(
            "data-htmlswap-v-bind",
            attribute,
            Some("Vue object bindings are preserved for adapters"),
            cx,
        ));
    }

    if name == "v-model" || name.starts_with("v-model.") || name.starts_with("v-model:") {
        let target = vue_model_target(element, raw_name);
        return Some(AttributeRewrite::replace(attribute_with_template(
            target,
            attribute.value.trim(),
            attribute.span,
        )));
    }

    if matches!(name.as_str(), "v-text" | "v-html" | "v-show") {
        return Some(preserve_vue_directive_attribute(
            format!("data-htmlswap-{}", name.replace(':', "-")),
            attribute,
            Some("Vue content/visibility directives are preserved for adapters"),
            cx,
        ));
    }

    if raw_name.starts_with('#') || name.starts_with("v-slot") {
        return Some(preserve_vue_directive_attribute(
            "data-htmlswap-v-slot",
            attribute,
            Some("Vue slots are preserved for adapters"),
            cx,
        ));
    }

    None
}

fn vue_event_attribute(
    rest: &str,
    attribute: &HtmlAttribute,
    cx: &mut DialectContext<'_>,
) -> Option<AttributeRewrite> {
    let (event, modifiers) = split_vue_target_modifiers(rest);
    if event.is_empty() {
        cx.warn("Vue event binding is missing an event name", attribute.span);
        return None;
    }

    let name = format!("on{}", event.to_ascii_lowercase());
    let value = vue_event_handler_value(&attribute.value, &modifiers, attribute.span, cx);
    Some(AttributeRewrite::replace(attribute_with_value(
        name,
        value,
        attribute.span,
    )))
}

fn vue_event_handler_value(
    value: &str,
    modifiers: &[CompactString],
    span: Option<Span>,
    cx: &mut DialectContext<'_>,
) -> CompactString {
    let handler = normalize_vue_event_handler(value);
    let effects = vue_event_modifier_effects(modifiers, span, cx);
    if effects.is_empty() && vue_handler_can_be_template(&handler) {
        return CompactString::from(format!("{{{{ {handler} }}}}"));
    }

    let handler = vue_handler_statement(&handler);
    if effects.is_empty() {
        return CompactString::from(handler);
    }

    if handler.is_empty() {
        CompactString::from(effects.join(" "))
    } else {
        CompactString::from(format!("{} {}", effects.join(" "), handler))
    }
}

fn normalize_vue_event_handler(value: &str) -> String {
    value.trim().replace("$event", "event")
}

fn vue_handler_can_be_template(handler: &str) -> bool {
    match parse_source_expr(handler) {
        Expr::Path(_) => true,
        Expr::Call { arguments, .. } => arguments.is_empty(),
        _ => false,
    }
}

fn vue_handler_statement(handler: &str) -> String {
    if handler.is_empty() {
        return String::new();
    }
    if matches!(parse_source_expr(handler), Expr::Path(_)) {
        return format!("{handler}(event);");
    }
    if handler.ends_with(';') {
        handler.to_owned()
    } else {
        format!("{handler};")
    }
}

fn vue_event_modifier_effects(
    modifiers: &[CompactString],
    span: Option<Span>,
    cx: &mut DialectContext<'_>,
) -> Vec<&'static str> {
    let mut effects = Vec::new();
    for modifier in modifiers {
        match modifier.as_str() {
            "prevent" => effects.push("event.preventDefault();"),
            "stop" => effects.push("event.stopPropagation();"),
            "capture" | "once" | "passive" | "self" | "exact" | "left" | "right" | "middle"
            | "enter" | "tab" | "delete" | "esc" | "space" | "up" | "down" => {
                cx.warn(
                    format!("Vue event modifier `{modifier}` is preserved but not first-class yet"),
                    span,
                );
            }
            _ => {
                cx.warn(
                    format!("unknown Vue event modifier `{modifier}` is preserved"),
                    span,
                );
            }
        }
    }
    effects
}

fn vue_binding_attribute(
    rest: &str,
    attribute: &HtmlAttribute,
    cx: &mut DialectContext<'_>,
) -> Option<AttributeRewrite> {
    let (target, modifiers) = split_vue_target_modifiers(rest);
    if target.is_empty() {
        cx.warn("Vue binding is missing an attribute name", attribute.span);
        return None;
    }
    warn_unsupported_vue_binding_modifiers(&modifiers, attribute.span, cx);

    let target = target.to_ascii_lowercase();
    let output_name = if target == "style" {
        CompactString::from("style")
    } else if target == "class" || target == "key" || html_boolean_attribute_name(&target) {
        CompactString::from(format!("data-htmlswap-bind-{target}"))
    } else {
        target
    };

    Some(AttributeRewrite::replace(attribute_with_template(
        output_name,
        attribute.value.trim(),
        attribute.span,
    )))
}

fn warn_unsupported_vue_binding_modifiers(
    modifiers: &[CompactString],
    span: Option<Span>,
    cx: &mut DialectContext<'_>,
) {
    for modifier in modifiers {
        match modifier.as_str() {
            "camel" | "prop" | "attr" => {
                cx.warn(
                    format!(
                        "Vue binding modifier `{modifier}` is preserved but not first-class yet"
                    ),
                    span,
                );
            }
            _ => {
                cx.warn(
                    format!("unknown Vue binding modifier `{modifier}` is preserved"),
                    span,
                );
            }
        }
    }
}

fn split_vue_target_modifiers(rest: &str) -> (CompactString, Vec<CompactString>) {
    let mut parts = rest.split('.').map(str::trim);
    let target = CompactString::from(parts.next().unwrap_or_default());
    let modifiers = parts
        .filter(|part| !part.is_empty())
        .map(CompactString::from)
        .collect();
    (target, modifiers)
}

fn vue_model_target(element: &HtmlElement, name: &str) -> CompactString {
    if let Some(rest) = name.strip_prefix("v-model:") {
        let (target, _) = split_vue_target_modifiers(rest);
        if !target.is_empty() {
            return target;
        }
    }

    if element.name.local().eq_ignore_ascii_case("input")
        && html_attribute(element, "type")
            .map(|attribute| attribute.value.as_str())
            .is_some_and(|kind| matches!(kind.to_ascii_lowercase().as_str(), "checkbox" | "radio"))
    {
        CompactString::from("checked")
    } else {
        CompactString::from("value")
    }
}

fn preserve_vue_directive_attribute(
    name: impl Into<CompactString>,
    attribute: &HtmlAttribute,
    warning: Option<&str>,
    cx: &mut DialectContext<'_>,
) -> AttributeRewrite {
    if let Some(warning) = warning {
        cx.warn(warning, attribute.span);
    }
    AttributeRewrite::replace(attribute_with_template(
        name,
        attribute.value.trim(),
        attribute.span,
    ))
}

fn attribute_with_template(
    name: impl Into<CompactString>,
    expression: &str,
    span: Option<Span>,
) -> HtmlAttribute {
    attribute_with_value(name, format!("{{{{ {} }}}}", expression.trim()), span)
}

fn attribute_with_value(
    name: impl Into<CompactString>,
    value: impl Into<CompactString>,
    span: Option<Span>,
) -> HtmlAttribute {
    HtmlAttribute {
        name: HtmlName::new(None::<CompactString>, None::<CompactString>, name.into()),
        value: value.into(),
        span,
    }
}

fn html_boolean_attribute_name(name: &str) -> bool {
    matches!(
        name,
        "allowfullscreen"
            | "async"
            | "autofocus"
            | "autoplay"
            | "checked"
            | "controls"
            | "default"
            | "defer"
            | "disabled"
            | "formnovalidate"
            | "hidden"
            | "inert"
            | "ismap"
            | "loop"
            | "multiple"
            | "muted"
            | "nomodule"
            | "novalidate"
            | "open"
            | "playsinline"
            | "readonly"
            | "required"
            | "reversed"
            | "selected"
    )
}

fn vue_control_flow(
    element: &HtmlElement,
    dialect: &VueDialect,
    cx: &mut DialectContext<'_>,
) -> Option<ElementDirectives> {
    let for_attribute = html_attribute(element, "v-for");
    let conditional_attribute = html_attribute(element, "v-if")
        .map(|attribute| (RenderControlFlowKind::If, attribute))
        .or_else(|| {
            html_attribute(element, "v-else-if")
                .map(|attribute| (RenderControlFlowKind::ElseIf, attribute))
        })
        .or_else(|| {
            html_attribute(element, "v-else")
                .map(|attribute| (RenderControlFlowKind::Else, attribute))
        });

    let (kind, expression, binding) = if let Some(attribute) = for_attribute {
        if conditional_attribute.is_some() {
            cx.warn(
                "Vue `v-for` and conditional directives on the same element cannot both be represented; lowering `v-for`",
                attribute.span.or(element.span),
            );
        }
        let (binding, expression) = vue_for_parts(&attribute.value, attribute.span, dialect, cx);
        (RenderControlFlowKind::For, expression, binding)
    } else if let Some((kind, attribute)) = conditional_attribute {
        let expression = match kind {
            RenderControlFlowKind::If | RenderControlFlowKind::ElseIf => {
                vue_expression_from_value(&attribute.value, attribute.span, dialect, cx)
            }
            RenderControlFlowKind::Else => None,
            _ => unreachable!("Vue conditional should only produce if/else control flow"),
        };
        (kind, expression, None)
    } else {
        return None;
    };

    Some(ElementDirectives {
        control_flow: Some(RenderControlFlow {
            kind,
            host: RenderControlFlowHost::Element,
            expression,
            binding,
            index_binding: None,
            key: None,
            locals: Vec::new(),
            placeholder: source_control_flow_placeholder(element),
            span: element.span,
        }),
        consumed_attributes: vue_control_flow_attributes(element),
    })
}

fn vue_for_parts(
    value: &str,
    span: Option<Span>,
    dialect: &VueDialect,
    cx: &mut DialectContext<'_>,
) -> (Option<BindingPattern>, Option<Expr>) {
    let Some((binding_source, expression_source)) = split_vue_for_expression(value) else {
        cx.warn(
            format!(
                "Vue `v-for` expression `{}` should use `item in items` or `item of items`",
                value.trim()
            ),
            span,
        );
        return (None, vue_expression_from_value(value, span, dialect, cx));
    };

    let binding =
        first_vue_for_binding(binding_source).map(|binding| BindingPattern::new(binding, span));
    let expression = vue_expression_from_value(expression_source, span, dialect, cx);
    (binding, expression)
}

fn split_vue_for_expression(value: &str) -> Option<(&str, &str)> {
    let value = value.trim();
    [" in ", " of "].iter().find_map(|separator| {
        let (binding, expression) = value.split_once(separator)?;
        let binding = binding.trim();
        let expression = expression.trim();
        (!binding.is_empty() && !expression.is_empty()).then_some((binding, expression))
    })
}

fn first_vue_for_binding(binding: &str) -> Option<String> {
    let binding = binding.trim();
    let binding = binding
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(binding)
        .trim();
    binding
        .split(',')
        .next()
        .map(str::trim)
        .filter(|binding| !binding.is_empty())
        .map(str::to_owned)
}

fn vue_expression_from_value(
    value: &str,
    span: Option<Span>,
    dialect: &VueDialect,
    cx: &mut DialectContext<'_>,
) -> Option<Expr> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(template) = dialect.parse_template(trimmed, span, cx) {
        return template.single_expression().cloned();
    }

    Some(parse_vue_expr(trimmed, span, cx))
}

fn vue_control_flow_attributes(element: &HtmlElement) -> SmallVec<[CompactString; 12]> {
    element
        .attributes
        .iter()
        .filter_map(|attribute| {
            let name = attribute.name.local().to_ascii_lowercase();
            matches!(name.as_str(), "v-for" | "v-if" | "v-else-if" | "v-else")
                .then(|| CompactString::from(name))
        })
        .collect()
}

fn quoted_string(source: &str) -> Option<&str> {
    let bytes = source.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        return Some(&source[1..source.len() - 1]);
    }

    None
}

fn is_number_literal(source: &str) -> bool {
    let mut chars = source.chars().peekable();
    if chars.peek().is_some_and(|ch| *ch == '-') {
        chars.next();
    }
    let mut has_digit = false;
    let mut has_dot = false;
    for ch in chars {
        if ch.is_ascii_digit() {
            has_digit = true;
        } else if ch == '.' && !has_dot {
            has_dot = true;
        } else {
            return false;
        }
    }

    has_digit
}

fn path_expr(source: &str) -> Option<Expr> {
    if source.is_empty()
        || !source
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$' | '.' | '-'))
    {
        return None;
    }

    source
        .split('.')
        .map(|segment| {
            let segment = segment.trim();
            (!segment.is_empty()).then(|| CompactString::from(segment))
        })
        .collect::<Option<Vec<_>>>()
        .map(Expr::Path)
}

fn dc_path_expr(source: &str) -> Option<Expr> {
    source
        .split('.')
        .map(|segment| {
            let segment = segment.trim();
            is_dc_identifier(segment).then(|| CompactString::from(segment))
        })
        .collect::<Option<Vec<_>>>()
        .map(Expr::Path)
}

fn is_dc_identifier(source: &str) -> bool {
    let mut chars = source.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn source_control_flow_placeholder(element: &HtmlElement) -> Option<CompactString> {
    let parts = element
        .attributes
        .iter()
        .filter_map(|attribute| {
            let name = attribute.name.local().to_ascii_lowercase();
            name.starts_with("hint-placeholder").then(|| {
                if attribute.value.is_empty() {
                    name
                } else {
                    format!("{name}={:?}", attribute.value)
                }
            })
        })
        .collect::<Vec<_>>();

    (!parts.is_empty()).then(|| CompactString::from(parts.join(", ")))
}

fn dc_control_flow_attributes(element: &HtmlElement) -> SmallVec<[CompactString; 12]> {
    element
        .attributes
        .iter()
        .filter_map(|attribute| {
            let name = attribute.name.local().to_ascii_lowercase();
            (matches!(
                name.as_str(),
                "list"
                    | "items"
                    | "of"
                    | "value"
                    | "each"
                    | "if"
                    | "test"
                    | "when"
                    | "condition"
                    | "switch"
                    | "on"
                    | "case"
                    | "as"
                    | "item"
                    | "let"
                    | "key"
                    | "index"
            ) || name.starts_with("hint-placeholder"))
            .then(|| CompactString::from(name))
        })
        .collect()
}

fn html_attribute<'a>(
    element: &'a HtmlElement,
    name: &str,
) -> Option<&'a crate::ir::HtmlAttribute> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local().eq_ignore_ascii_case(name))
}
