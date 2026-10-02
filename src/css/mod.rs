use std::fmt;

mod index;
mod source_spans;

use compact_str::CompactString;
use lightningcss::declaration::DeclarationBlock;
use lightningcss::properties::Property;
use lightningcss::rules::keyframes::{KeyframeSelector, KeyframesName, KeyframesRule};
use lightningcss::rules::style::StyleRule;
use lightningcss::rules::view_transition::{Navigation, ViewTransitionProperty};
use lightningcss::rules::{CssRule as LightningCssRule, CssRuleList};
use lightningcss::selector::{
    Combinator, Component, Direction, PseudoClass, PseudoElement, Selector, SelectorList,
};
use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleAttribute, StyleSheet};
use lightningcss::traits::{IntoOwned, ToCss};
use lightningcss::values::ident::NoneOrCustomIdentList;
use parcel_selectors::attr::{
    AttrSelectorOperator, CaseSensitivity, NamespaceConstraint, ParsedAttrSelectorOperation,
};
use parcel_selectors::parser::NthType;

use crate::diagnostics::{Compilation, Diagnostic, Diagnostics};
use crate::ir::{HtmlDocument, HtmlElement, HtmlNode};
use crate::plan::{
    RenderAnnotation, RenderAnnotationKind, RenderKeyframe, RenderKeyframes, RenderMotionPlan,
    RenderStyleCondition, RenderViewTransitionRule, ViewTransitionName, ViewTransitionPart,
};
use crate::source::{SourceId, Span};
use crate::style::StyleDeclaration;

pub(crate) use index::StyleIndex;
use source_spans::LineIndex;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Stylesheet {
    pub(crate) rules: Vec<CssRule>,
    pub(crate) annotations: Vec<RenderAnnotation>,
    pub(crate) imports: Vec<StylesheetImport>,
    pub(crate) motion: RenderMotionPlan,
}

impl Stylesheet {
    #[must_use]
    pub(crate) fn new(
        rules: Vec<CssRule>,
        annotations: Vec<RenderAnnotation>,
        imports: Vec<StylesheetImport>,
    ) -> Self {
        Self {
            rules,
            annotations,
            imports,
            motion: RenderMotionPlan::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StylesheetImport {
    pub(crate) specifier: String,
    pub(crate) span: Option<Span>,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct CssRule {
    pub(crate) selector: CssSelector,
    pub(crate) declarations: Vec<StyleDeclaration>,
    pub(crate) conditions: Vec<RenderStyleCondition>,
    pub(crate) span: Option<Span>,
}

impl fmt::Debug for CssRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CssRule")
            .field("selector", &self.selector)
            .field("declarations", &self.declarations)
            .field("conditions", &self.conditions)
            .field("span", &self.span)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct CssSelector {
    pub(crate) raw: String,
    selectors: SelectorList<'static>,
}

impl fmt::Debug for CssSelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CssSelector")
            .field("raw", &self.raw)
            .finish_non_exhaustive()
    }
}

impl CssSelector {
    #[must_use]
    pub(crate) fn matching_specificity(&self, element: &StyleElement<'_, '_>) -> Option<u32> {
        self.selectors
            .0
            .iter()
            .filter(|selector| selector_matches(selector, element))
            .map(Selector::specificity)
            .max()
    }

    #[must_use]
    pub(crate) fn matching_dynamic_branches(
        &self,
        element: &StyleElement<'_, '_>,
    ) -> Vec<(u32, Vec<RenderStyleCondition>)> {
        self.selectors
            .0
            .iter()
            .filter_map(|selector| {
                if !selector_matches_with_dynamic(selector, element, true) {
                    return None;
                }

                let conditions = selector_dynamic_conditions(selector);
                (!conditions.is_empty()).then_some((selector.specificity(), conditions))
            })
            .collect()
    }

    #[must_use]
    pub(crate) fn has_static_anchor(&self) -> bool {
        self.selectors.0.iter().any(selector_has_static_anchor)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StyleElement<'tree, 'path> {
    pub element: &'tree HtmlElement,
    pub ancestors: &'path [StyleAncestor<'tree>],
    pub siblings: &'tree [HtmlNode],
    pub sibling_index: usize,
}

impl<'tree, 'path> StyleElement<'tree, 'path> {
    #[must_use]
    pub fn new(
        element: &'tree HtmlElement,
        ancestors: &'path [StyleAncestor<'tree>],
        siblings: &'tree [HtmlNode],
        sibling_index: usize,
    ) -> Self {
        Self {
            element,
            ancestors,
            siblings,
            sibling_index,
        }
    }

    #[must_use]
    pub fn as_ancestor(self) -> StyleAncestor<'tree> {
        StyleAncestor {
            element: self.element,
            siblings: self.siblings,
            sibling_index: self.sibling_index,
        }
    }

    fn parent(self) -> Option<Self> {
        let (parent, ancestors) = self.ancestors.split_last()?;
        Some(Self {
            element: parent.element,
            ancestors,
            siblings: parent.siblings,
            sibling_index: parent.sibling_index,
        })
    }

    fn previous_sibling(self) -> Option<Self> {
        self.previous_siblings().into_iter().next()
    }

    fn previous_siblings(self) -> Vec<Self> {
        (0..self.sibling_index)
            .rev()
            .filter_map(|index| {
                let HtmlNode::Element(element) = &self.siblings[index] else {
                    return None;
                };

                Some(Self {
                    element,
                    ancestors: self.ancestors,
                    siblings: self.siblings,
                    sibling_index: index,
                })
            })
            .collect()
    }

    fn ancestors_from_nearest(self) -> Vec<Self> {
        (0..self.ancestors.len())
            .rev()
            .map(|index| {
                let ancestor = self.ancestors[index];
                Self {
                    element: ancestor.element,
                    ancestors: &self.ancestors[..index],
                    siblings: ancestor.siblings,
                    sibling_index: ancestor.sibling_index,
                }
            })
            .collect()
    }

    fn is_root(self) -> bool {
        self.ancestors.is_empty()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StyleAncestor<'a> {
    pub element: &'a HtmlElement,
    pub siblings: &'a [HtmlNode],
    pub sibling_index: usize,
}

impl<'a> StyleAncestor<'a> {
    #[must_use]
    pub fn new(element: &'a HtmlElement, siblings: &'a [HtmlNode], sibling_index: usize) -> Self {
        Self {
            element,
            siblings,
            sibling_index,
        }
    }
}

#[must_use]
pub(crate) fn parse_stylesheet_with_source(
    source: &str,
    source_id: SourceId,
) -> Compilation<Stylesheet> {
    parse_stylesheet_with_offset(source, source_id, 0)
}

#[must_use]
pub(crate) fn parse_stylesheet_with_offset(
    source: &str,
    source_id: SourceId,
    offset: usize,
) -> Compilation<Stylesheet> {
    let mut diagnostics = Diagnostics::new();
    let line_index = LineIndex::with_offset(source, source_id, offset);
    let annotations = stylesheet_annotations(source, source_id, offset);

    let stylesheet = match StyleSheet::parse(source, ParserOptions::default()) {
        Ok(stylesheet) => stylesheet,
        Err(error) => {
            let span = error
                .loc
                .as_ref()
                .and_then(|location| line_index.location_span(location.line, location.column));
            diagnostics.push(Diagnostic::error(format!("CSS parse error: {error}"), span));
            return Compilation::new(
                Stylesheet::new(Vec::new(), annotations, Vec::new()),
                diagnostics,
            );
        }
    };

    let mut collector = Collector {
        line_index: &line_index,
        rules: Vec::new(),
        imports: Vec::new(),
        motion: RenderMotionPlan::default(),
    };
    collect_rule_list(&stylesheet.rules, &[], None, &mut collector);
    let mut collected = Stylesheet::new(collector.rules, annotations, collector.imports);
    collected.motion = collector.motion;
    Compilation::new(collected, diagnostics)
}

fn stylesheet_annotations(
    source: &str,
    source_id: SourceId,
    offset: usize,
) -> Vec<RenderAnnotation> {
    let bytes = source.as_bytes();
    let mut annotations = Vec::new();
    let mut index = 0;
    let mut quote = None;
    let mut escaped = false;

    while index < bytes.len() {
        let byte = bytes[index];

        if let Some(quote_byte) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == quote_byte {
                quote = None;
            }
            index += 1;
            continue;
        }

        match byte {
            b'\'' | b'"' => {
                quote = Some(byte);
                index += 1;
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                let start = index;
                let content_start = index + 2;
                index = content_start;

                while index < bytes.len()
                    && !(bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/'))
                {
                    index += 1;
                }

                let (content_end, end) = if index < bytes.len() {
                    (index, index + 2)
                } else {
                    (source.len(), source.len())
                };
                let value = source
                    .get(content_start..content_end)
                    .unwrap_or_default()
                    .to_owned();
                annotations.push(RenderAnnotation::new(
                    RenderAnnotationKind::CssComment,
                    value,
                    Some(Span::new(source_id, offset + start, offset + end)),
                ));
                index = end;
            }
            _ => index += 1,
        }
    }

    annotations
}

#[must_use]
pub(crate) fn parse_style_attribute(
    source: &str,
    span: Option<Span>,
) -> Compilation<Vec<StyleDeclaration>> {
    match StyleAttribute::parse(source, ParserOptions::default()) {
        Ok(attribute) => Compilation::clean(declarations_from_block(&attribute.declarations, span)),
        Err(error) => {
            let mut diagnostics = Diagnostics::new();
            diagnostics.push(Diagnostic::warning(
                format!("CSS style attribute parse error: {error}"),
                span,
            ));
            Compilation::new(Vec::new(), diagnostics)
        }
    }
}

#[must_use]
pub(crate) fn inline_stylesheets(document: &HtmlDocument) -> Vec<InlineStylesheet> {
    let mut stylesheets = Vec::new();
    for node in &document.nodes {
        collect_inline_stylesheets(node, &mut stylesheets);
    }
    stylesheets
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InlineStylesheet {
    pub(crate) source: String,
    pub(crate) span: Option<Span>,
}

#[must_use]
pub(crate) fn linked_stylesheets(document: &HtmlDocument) -> Vec<LinkedStylesheet> {
    let mut stylesheets = Vec::new();
    for node in &document.nodes {
        collect_linked_stylesheets(node, &mut stylesheets);
    }
    stylesheets
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkedStylesheet {
    pub(crate) href: String,
    pub(crate) span: Option<Span>,
}

fn collect_inline_stylesheets(node: &HtmlNode, stylesheets: &mut Vec<InlineStylesheet>) {
    let HtmlNode::Element(element) = node else {
        return;
    };

    if element.name.local().eq_ignore_ascii_case("style") {
        let stylesheet = style_text(element);
        if !stylesheet.source.trim().is_empty() {
            stylesheets.push(stylesheet);
        }
    }

    for child in &element.children {
        collect_inline_stylesheets(child, stylesheets);
    }
}

fn collect_linked_stylesheets(node: &HtmlNode, stylesheets: &mut Vec<LinkedStylesheet>) {
    let HtmlNode::Element(element) = node else {
        return;
    };

    if element.name.local().eq_ignore_ascii_case("link")
        && link_rel_includes_stylesheet(element)
        && link_type_is_css(element)
        && !has_html_boolean_attribute(element, "disabled")
        && let Some((href, span)) = html_attribute_value(element, "href")
    {
        let href = href.trim();
        if !href.is_empty() {
            stylesheets.push(LinkedStylesheet {
                href: href.to_owned(),
                span,
            });
        }
    }

    for child in &element.children {
        collect_linked_stylesheets(child, stylesheets);
    }
}

fn link_rel_includes_stylesheet(element: &HtmlElement) -> bool {
    html_attribute_value(element, "rel").is_some_and(|(rel, _)| {
        rel.split_ascii_whitespace()
            .any(|token| token.eq_ignore_ascii_case("stylesheet"))
    })
}

fn link_type_is_css(element: &HtmlElement) -> bool {
    html_attribute_value(element, "type").is_none_or(|(link_type, _)| {
        matches!(
            link_type.trim().to_ascii_lowercase().as_str(),
            "" | "text/css"
        )
    })
}

fn has_html_boolean_attribute(element: &HtmlElement, name: &str) -> bool {
    element
        .attributes
        .iter()
        .any(|attribute| attribute.name.local().eq_ignore_ascii_case(name))
}

fn html_attribute_value<'a>(
    element: &'a HtmlElement,
    name: &str,
) -> Option<(&'a str, Option<Span>)> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local().eq_ignore_ascii_case(name))
        .map(|attribute| (attribute.value.as_str(), attribute.span))
}

fn style_text(element: &HtmlElement) -> InlineStylesheet {
    let mut source = String::new();
    let mut span_source = None;
    let mut span_start = None;
    let mut span_end = None;

    for child in &element.children {
        let HtmlNode::Text(text) = child else {
            continue;
        };

        if let Some(span) = text.span {
            span_source.get_or_insert(span.source);
            if span_start.is_none() {
                span_start = Some(span.start);
            }

            if span_end.is_none_or(|end| end == span.start) {
                span_end = Some(span.end);
            }
        }

        source.push_str(&text.value);
    }

    InlineStylesheet {
        source,
        span: span_start
            .zip(span_end)
            .and_then(|(start, end)| span_source.map(|source| Span::new(source, start, end))),
    }
}

/// Rules, imports and motion collected from one stylesheet.
struct Collector<'a> {
    line_index: &'a LineIndex<'a>,
    rules: Vec<CssRule>,
    imports: Vec<StylesheetImport>,
    motion: RenderMotionPlan,
}

fn collect_rule_list(
    rule_list: &CssRuleList<'_>,
    conditions: &[RenderStyleCondition],
    parent: Option<&SelectorList<'static>>,
    out: &mut Collector<'_>,
) {
    for rule in &rule_list.0 {
        collect_rule(rule, conditions, parent, out);
    }
}

fn nested_conditions(
    conditions: &[RenderStyleCondition],
    condition: Option<RenderStyleCondition>,
) -> Vec<RenderStyleCondition> {
    let mut nested = conditions.to_vec();
    nested.extend(condition);
    nested
}

fn collect_rule(
    rule: &LightningCssRule<'_>,
    conditions: &[RenderStyleCondition],
    parent: Option<&SelectorList<'static>>,
    out: &mut Collector<'_>,
) {
    match rule {
        LightningCssRule::Import(rule) => {
            out.imports.push(StylesheetImport {
                specifier: rule.url.to_string(),
                span: out.line_index.rule_span(rule.loc),
            });
        }
        LightningCssRule::Style(style_rule) => {
            lower_style_rule(style_rule, conditions, parent, out);
        }
        LightningCssRule::Media(rule) => {
            let condition = rule
                .query
                .to_css_string(PrinterOptions::default())
                .ok()
                .map(|query| RenderStyleCondition::Media(query.into()));
            let nested = nested_conditions(conditions, condition);
            collect_rule_list(&rule.rules, &nested, parent, out);
        }
        LightningCssRule::Supports(rule) => {
            let condition = rule
                .condition
                .to_css_string(PrinterOptions::default())
                .ok()
                .map(|condition| RenderStyleCondition::Supports(condition.into()));
            let nested = nested_conditions(conditions, condition);
            collect_rule_list(&rule.rules, &nested, parent, out);
        }
        LightningCssRule::MozDocument(rule) => {
            collect_rule_list(&rule.rules, conditions, parent, out);
        }
        LightningCssRule::Nesting(rule) => {
            lower_style_rule(&rule.style, conditions, parent, out);
        }
        LightningCssRule::NestedDeclarations(rule) => {
            // Declarations after nested rules apply to the parent selector.
            if let Some(parent) = parent {
                let span = out.line_index.rule_span(rule.loc);
                push_style_rule(
                    parent.clone(),
                    declarations_from_block(&rule.declarations, span),
                    conditions,
                    span,
                    out,
                );
            }
        }
        LightningCssRule::LayerBlock(rule) => {
            collect_rule_list(&rule.rules, conditions, parent, out);
        }
        LightningCssRule::Container(rule) => {
            let condition = container_condition(rule)
                .map(|condition| RenderStyleCondition::Container(condition.into()));
            let nested = nested_conditions(conditions, condition);
            collect_rule_list(&rule.rules, &nested, parent, out);
        }
        LightningCssRule::Scope(rule) => {
            collect_rule_list(&rule.rules, conditions, parent, out);
        }
        LightningCssRule::StartingStyle(rule) => {
            let nested = nested_conditions(conditions, Some(RenderStyleCondition::StartingStyle));
            collect_rule_list(&rule.rules, &nested, parent, out);
        }
        LightningCssRule::Keyframes(rule) => {
            if let Some(keyframes) = lower_keyframes(rule, conditions, out.line_index) {
                out.motion
                    .keyframes
                    .retain(|existing| existing.name != keyframes.name);
                out.motion.keyframes.push(keyframes);
            }
        }
        LightningCssRule::ViewTransition(rule) => {
            let view_transition = &mut out.motion.view_transition;
            view_transition.span = out.line_index.rule_span(rule.loc);
            for property in &rule.properties {
                match property {
                    ViewTransitionProperty::Navigation(navigation) => {
                        view_transition.navigation = matches!(navigation, Navigation::Auto);
                    }
                    ViewTransitionProperty::Types(types) => {
                        view_transition.types = match types {
                            NoneOrCustomIdentList::None => Vec::new(),
                            NoneOrCustomIdentList::Idents(idents) => idents
                                .iter()
                                .map(|ident| CompactString::from(ident.0.as_ref()))
                                .collect(),
                        };
                    }
                    ViewTransitionProperty::Custom(_) => {}
                }
            }
        }
        _ => {}
    }
}

fn lower_keyframes(
    rule: &KeyframesRule<'_>,
    conditions: &[RenderStyleCondition],
    line_index: &LineIndex<'_>,
) -> Option<RenderKeyframes> {
    let name = match &rule.name {
        KeyframesName::Ident(ident) => CompactString::from(ident.0.as_ref()),
        KeyframesName::Custom(name) => CompactString::from(name.as_ref()),
    };
    let span = line_index.rule_span(rule.loc);
    let mut frames = Vec::new();
    for keyframe in &rule.keyframes {
        let declarations = declarations_from_block(&keyframe.declarations, span);
        for selector in &keyframe.selectors {
            let offset = match selector {
                KeyframeSelector::From => 0.0,
                KeyframeSelector::To => 1.0,
                KeyframeSelector::Percentage(percentage) => percentage.0,
                // Scroll-driven ranges have no time-based offset.
                KeyframeSelector::TimelineRangePercentage(_) => continue,
            };
            if offset.is_finite() && (0.0..=1.0).contains(&offset) {
                frames.push(RenderKeyframe {
                    offset,
                    declarations: declarations.clone(),
                });
            }
        }
    }
    frames.sort_by(|left, right| left.offset.total_cmp(&right.offset));
    (!frames.is_empty()).then(|| RenderKeyframes {
        name,
        frames,
        conditions: conditions.to_vec(),
        span,
    })
}

fn lower_style_rule(
    style_rule: &StyleRule<'_>,
    conditions: &[RenderStyleCondition],
    parent: Option<&SelectorList<'static>>,
    out: &mut Collector<'_>,
) {
    let span = out.line_index.rule_span(style_rule.loc);
    let selectors = match parent {
        Some(parent) => resolve_nesting(&style_rule.selectors, parent),
        None => Some(style_rule.selectors.clone().into_owned()),
    };
    let Some(selectors) = selectors else {
        return;
    };
    let declarations = declarations_from_block(&style_rule.declarations, span);
    if !declarations.is_empty() {
        push_style_rule(selectors.clone(), declarations, conditions, span, out);
    }

    collect_rule_list(&style_rule.rules, conditions, Some(&selectors), out);
}

/// Record one style rule, routing `::view-transition-*` selectors to the
/// motion plan and the rest to element rules.
fn push_style_rule(
    selectors: SelectorList<'static>,
    declarations: Vec<StyleDeclaration>,
    conditions: &[RenderStyleCondition],
    span: Option<Span>,
    out: &mut Collector<'_>,
) {
    if declarations.is_empty() {
        return;
    }
    let (view_transition, element): (Vec<_>, Vec<_>) = selectors
        .0
        .into_iter()
        .partition(|selector| view_transition_part(selector).is_some());
    for selector in view_transition {
        if let Some(rule) = lower_view_transition_rule(
            &selector,
            &declarations,
            conditions,
            span,
            out.motion.view_transition.rules.len(),
        ) {
            out.motion.view_transition.rules.push(rule);
        }
    }
    if element.is_empty() {
        return;
    }
    let selectors = SelectorList::new(element.into());
    let raw = selectors
        .to_css_string(PrinterOptions::default())
        .unwrap_or_else(|_| "<unprintable selector>".to_owned());
    out.rules.push(CssRule {
        selector: CssSelector { raw, selectors },
        declarations,
        conditions: conditions.to_vec(),
        span,
    });
}

/// Resolve `&` in nested selectors as `:is(<parent>)`, as CSS Nesting
/// defines it. A nested selector without `&` is relative to its parent as
/// a descendant.
fn resolve_nesting(
    selectors: &SelectorList<'_>,
    parent: &SelectorList<'static>,
) -> Option<SelectorList<'static>> {
    let parent = parent.to_css_string(PrinterOptions::default()).ok()?;
    let parent = format!(":is({parent})");
    let resolved = selectors
        .0
        .iter()
        .filter_map(|selector| selector.to_css_string(PrinterOptions::default()).ok())
        .map(|selector| {
            if contains_nesting(&selector) {
                replace_nesting(&selector, &parent)
            } else {
                format!("{parent} {selector}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    parse_selector_list(&resolved)
}

fn parse_selector_list(selectors: &str) -> Option<SelectorList<'static>> {
    let source = format!("{selectors} {{}}");
    let stylesheet = StyleSheet::parse(&source, ParserOptions::default()).ok()?;
    stylesheet.rules.0.into_iter().find_map(|rule| match rule {
        LightningCssRule::Style(rule) => Some(rule.selectors.into_owned()),
        _ => None,
    })
}

/// Visit the `&` tokens of a serialized selector, skipping strings and escapes.
fn nesting_positions(selector: &str) -> impl Iterator<Item = usize> + '_ {
    let mut quote = None;
    let mut escaped = false;
    selector
        .char_indices()
        .filter_map(move |(index, character)| {
            if escaped {
                escaped = false;
                return None;
            }
            match (quote, character) {
                (_, '\\') => escaped = true,
                (Some(open), _) if character == open => quote = None,
                (Some(_), _) => {}
                (None, '"' | '\'') => quote = Some(character),
                (None, '&') => return Some(index),
                (None, _) => {}
            }
            None
        })
}

fn contains_nesting(selector: &str) -> bool {
    nesting_positions(selector).next().is_some()
}

fn replace_nesting(selector: &str, parent: &str) -> String {
    let mut resolved = String::with_capacity(selector.len() + parent.len());
    let mut last = 0;
    for index in nesting_positions(selector) {
        resolved.push_str(&selector[last..index]);
        resolved.push_str(parent);
        last = index + 1;
    }
    resolved.push_str(&selector[last..]);
    resolved
}

/// The view-transition pseudo-element a selector targets, with its
/// serialized argument (`root`, `*`, `card.hero`, …).
fn view_transition_part(selector: &Selector<'_>) -> Option<(ViewTransitionPart, String)> {
    let part = selector
        .iter_raw_match_order()
        .find_map(|component| match component {
            Component::PseudoElement(PseudoElement::ViewTransition) => {
                Some(ViewTransitionPart::Overlay)
            }
            Component::PseudoElement(PseudoElement::ViewTransitionGroup { .. }) => {
                Some(ViewTransitionPart::Group)
            }
            Component::PseudoElement(PseudoElement::ViewTransitionImagePair { .. }) => {
                Some(ViewTransitionPart::ImagePair)
            }
            Component::PseudoElement(PseudoElement::ViewTransitionOld { .. }) => {
                Some(ViewTransitionPart::Old)
            }
            Component::PseudoElement(PseudoElement::ViewTransitionNew { .. }) => {
                Some(ViewTransitionPart::New)
            }
            _ => None,
        })?;
    // The part selector's fields are private; read its argument back from
    // the serialized selector, where the pseudo-element is always last.
    let css = selector.to_css_string(PrinterOptions::default()).ok()?;
    let pseudo = &css[css.rfind("::view-transition")?..];
    let argument = pseudo
        .split_once('(')
        .and_then(|(_, rest)| rest.strip_suffix(')'))
        .unwrap_or("*")
        .trim()
        .to_owned();
    Some((part, argument))
}

fn lower_view_transition_rule(
    selector: &Selector<'_>,
    declarations: &[StyleDeclaration],
    conditions: &[RenderStyleCondition],
    span: Option<Span>,
    source_order: usize,
) -> Option<RenderViewTransitionRule> {
    let (part, argument) = view_transition_part(selector)?;
    let mut segments = argument.split('.');
    let name = match segments.next().unwrap_or("*").trim() {
        "" | "*" => ViewTransitionName::Any,
        name => ViewTransitionName::Named(name.into()),
    };
    let classes = segments
        .filter(|class| !class.is_empty())
        .map(CompactString::from)
        .collect();
    let mut types = Vec::new();
    for component in selector.iter_raw_match_order() {
        if let Component::NonTSPseudoClass(PseudoClass::ActiveViewTransitionType { kind }) =
            component
        {
            types.extend(
                kind.iter()
                    .map(|ident| CompactString::from(ident.0.as_ref())),
            );
        }
    }
    Some(RenderViewTransitionRule {
        part,
        name,
        classes,
        types,
        conditions: conditions
            .iter()
            .filter(|condition| {
                matches!(
                    condition,
                    RenderStyleCondition::Media(_) | RenderStyleCondition::Supports(_)
                )
            })
            .cloned()
            .collect(),
        declarations: declarations.to_vec(),
        specificity: selector.specificity(),
        source_order,
        selector: selector
            .to_css_string(PrinterOptions::default())
            .unwrap_or_default()
            .into(),
        span,
    })
}

fn container_condition(rule: &lightningcss::rules::container::ContainerRule<'_>) -> Option<String> {
    let name = rule
        .name
        .as_ref()
        .and_then(|name| name.to_css_string(PrinterOptions::default()).ok());
    let condition = rule
        .condition
        .as_ref()
        .and_then(|condition| condition.to_css_string(PrinterOptions::default()).ok());

    match (name, condition) {
        (Some(name), Some(condition)) => Some(format!("{name} {condition}")),
        (Some(name), None) => Some(name),
        (None, Some(condition)) => Some(condition),
        (None, None) => None,
    }
}

fn declarations_from_block(
    block: &DeclarationBlock<'_>,
    span: Option<Span>,
) -> Vec<StyleDeclaration> {
    let mut declarations = Vec::with_capacity(block.len());

    for (property, important) in block.iter() {
        if let Some(declaration) = declaration_from_property(property, important, span) {
            declarations.push(declaration);
        }
    }

    declarations
}

fn declaration_from_property(
    property: &Property<'_>,
    important: bool,
    span: Option<Span>,
) -> Option<StyleDeclaration> {
    let property_name = property
        .property_id()
        .to_css_string(PrinterOptions::default())
        .ok()?;
    let value = property
        .value_to_css_string(PrinterOptions::default())
        .ok()?;

    if property_name.trim().is_empty() || value.trim().is_empty() {
        return None;
    }

    Some(StyleDeclaration::new(property_name, value, important, span))
}

fn selector_matches(selector: &Selector<'_>, element: &StyleElement<'_, '_>) -> bool {
    selector_matches_with_dynamic(selector, element, false)
}

fn selector_matches_with_dynamic(
    selector: &Selector<'_>,
    element: &StyleElement<'_, '_>,
    allow_dynamic: bool,
) -> bool {
    let components = selector
        .iter_raw_match_order()
        .collect::<Vec<&Component<'_>>>();
    components_match(&components, 0, element, allow_dynamic)
}

fn components_match(
    components: &[&Component<'_>],
    mut index: usize,
    element: &StyleElement<'_, '_>,
    allow_dynamic: bool,
) -> bool {
    while let Some(component) = components.get(index) {
        if let Component::Combinator(combinator) = component {
            return combinator_matches(*combinator, components, index + 1, element, allow_dynamic);
        }

        if !simple_selector_matches(component, element, allow_dynamic) {
            return false;
        }

        index += 1;
    }

    true
}

fn combinator_matches(
    combinator: Combinator,
    components: &[&Component<'_>],
    next_index: usize,
    element: &StyleElement<'_, '_>,
    allow_dynamic: bool,
) -> bool {
    match combinator {
        Combinator::Child => element
            .parent()
            .is_some_and(|parent| components_match(components, next_index, &parent, allow_dynamic)),
        Combinator::Descendant | Combinator::DeepDescendant | Combinator::Deep => element
            .ancestors_from_nearest()
            .iter()
            .any(|ancestor| components_match(components, next_index, ancestor, allow_dynamic)),
        Combinator::NextSibling => element.previous_sibling().is_some_and(|sibling| {
            components_match(components, next_index, &sibling, allow_dynamic)
        }),
        Combinator::LaterSibling => element
            .previous_siblings()
            .iter()
            .any(|sibling| components_match(components, next_index, sibling, allow_dynamic)),
        Combinator::PseudoElement if allow_dynamic => {
            components_match(components, next_index, element, allow_dynamic)
        }
        Combinator::PseudoElement | Combinator::SlotAssignment | Combinator::Part => false,
    }
}

fn simple_selector_matches(
    component: &Component<'_>,
    element: &StyleElement<'_, '_>,
    allow_dynamic: bool,
) -> bool {
    match component {
        Component::Combinator(_) => unreachable!("combinators are handled between compounds"),
        Component::ExplicitAnyNamespace | Component::ExplicitUniversalType => true,
        Component::ExplicitNoNamespace => {
            element.element.name.namespace().is_none_or(str::is_empty)
        }
        Component::DefaultNamespace(namespace) | Component::Namespace(_, namespace) => element
            .element
            .name
            .namespace()
            .is_some_and(|element_namespace| element_namespace == namespace.as_ref()),
        Component::LocalName(local_name) => {
            let expected = if is_html_element(element) {
                local_name.lower_name.as_ref()
            } else {
                local_name.name.as_ref()
            };
            element.element.name.local().eq_ignore_ascii_case(expected)
        }
        Component::ID(id) => attribute_value(element.element, "id").is_some_and(|value| {
            CaseSensitivity::CaseSensitive.eq(value.as_bytes(), id.as_ref().as_bytes())
        }),
        Component::Class(class_name) => element_classes(element.element).any(|class| {
            CaseSensitivity::CaseSensitive.eq(class.as_bytes(), class_name.as_ref().as_bytes())
        }),
        Component::AttributeInNoNamespaceExists {
            local_name,
            local_name_lower,
        } => {
            let name =
                selector_attribute_name(element, local_name.as_ref(), local_name_lower.as_ref());
            attribute_value(element.element, name).is_some()
        }
        Component::AttributeInNoNamespace {
            local_name,
            operator,
            value,
            case_sensitivity,
            never_matches,
        } => {
            if *never_matches {
                return false;
            }

            attribute_value(element.element, local_name.as_ref()).is_some_and(|attribute| {
                attr_operation_matches(
                    *operator,
                    case_sensitivity.to_unconditional(is_html_element(element)),
                    attribute,
                    value.as_ref(),
                )
            })
        }
        Component::AttributeOther(selector) => {
            if selector.never_matches {
                return false;
            }

            let is_html = is_html_element(element);
            let name = selector_attribute_name(
                element,
                selector.local_name.as_ref(),
                selector.local_name_lower.as_ref(),
            );
            element.element.attributes.iter().any(|attribute| {
                if !attribute_namespace_matches(attribute.name.namespace(), selector.namespace()) {
                    return false;
                }

                if !attribute.name.local().eq_ignore_ascii_case(name) {
                    return false;
                }

                match &selector.operation {
                    ParsedAttrSelectorOperation::Exists => true,
                    ParsedAttrSelectorOperation::WithValue {
                        operator,
                        case_sensitivity,
                        expected_value,
                    } => attr_operation_matches(
                        *operator,
                        case_sensitivity.to_unconditional(is_html),
                        &attribute.value,
                        expected_value.as_ref(),
                    ),
                }
            })
        }
        Component::Root | Component::Scope => element.is_root(),
        Component::Empty => element_is_empty(element.element),
        Component::Nth(data) => nth_matches(element, data.ty, data.a, data.b),
        Component::NthOf(data) => {
            let nth_data = data.nth_data();
            nth_of_matches(
                element,
                nth_data.ty,
                nth_data.a,
                nth_data.b,
                data.selectors(),
            )
        }
        Component::Is(selectors) | Component::Where(selectors) | Component::Any(_, selectors) => {
            selectors
                .iter()
                .any(|selector| selector_matches_with_dynamic(selector, element, allow_dynamic))
        }
        Component::Negation(selectors) => selectors
            .iter()
            .all(|selector| !selector_matches_with_dynamic(selector, element, allow_dynamic)),
        Component::Has(selectors) => selectors
            .iter()
            .any(|selector| descendant_matches(selector, element, allow_dynamic)),
        Component::NonTSPseudoClass(pseudo_class) => {
            pseudo_class_matches(pseudo_class, element, allow_dynamic)
        }
        Component::PseudoElement(_) if allow_dynamic => true,
        Component::Nesting
        | Component::PseudoElement(_)
        | Component::Slotted(_)
        | Component::Part(_)
        | Component::Host(_) => false,
    }
}

fn selector_attribute_name<'a>(
    element: &StyleElement<'_, '_>,
    local_name: &'a str,
    local_name_lower: &'a str,
) -> &'a str {
    if is_html_element(element) {
        local_name_lower
    } else {
        local_name
    }
}

fn attr_operation_matches(
    operator: AttrSelectorOperator,
    case_sensitivity: CaseSensitivity,
    attribute_value: &str,
    expected_value: &str,
) -> bool {
    operator.eval_str(attribute_value, expected_value, case_sensitivity)
}

fn attribute_namespace_matches(
    attribute_namespace: Option<&str>,
    selector_namespace: Option<NamespaceConstraint<&lightningcss::values::string::CowArcStr<'_>>>,
) -> bool {
    match selector_namespace {
        Some(NamespaceConstraint::Any) => true,
        Some(NamespaceConstraint::Specific(namespace)) => {
            attribute_namespace.is_some_and(|value| value == namespace.as_ref())
        }
        None => attribute_namespace.is_none_or(str::is_empty),
    }
}

fn pseudo_class_matches(
    pseudo_class: &PseudoClass<'_>,
    element: &StyleElement<'_, '_>,
    allow_dynamic: bool,
) -> bool {
    #[allow(
        unreachable_patterns,
        reason = "lightningcss adds pseudo variants in compatible alpha releases"
    )]
    match pseudo_class {
        PseudoClass::Lang { languages } => {
            inherited_attribute(element, "lang").is_some_and(|lang| {
                languages
                    .iter()
                    .any(|expected| lang_matches(lang, expected))
            })
        }
        PseudoClass::Dir { direction } => inherited_attribute(element, "dir")
            .is_some_and(|dir| direction_matches(dir, *direction)),
        PseudoClass::Defined => true,
        PseudoClass::AnyLink(_) | PseudoClass::Link | PseudoClass::LocalLink => is_link(element),
        PseudoClass::Disabled => is_disabled(element),
        PseudoClass::Enabled => is_disableable(element.element) && !is_disabled(element),
        PseudoClass::Checked => {
            has_boolean_attribute(element.element, "checked")
                || has_boolean_attribute(element.element, "selected")
        }
        PseudoClass::Required => has_boolean_attribute(element.element, "required"),
        PseudoClass::Optional => {
            is_form_control(element.element) && !has_boolean_attribute(element.element, "required")
        }
        PseudoClass::ReadOnly(_) => has_boolean_attribute(element.element, "readonly"),
        PseudoClass::ReadWrite(_) => {
            is_text_editable(element.element) && !has_boolean_attribute(element.element, "readonly")
        }
        PseudoClass::PlaceholderShown(_) => attribute_value(element.element, "placeholder")
            .is_some_and(|_| attribute_value(element.element, "value").is_none_or(str::is_empty)),
        PseudoClass::Open | PseudoClass::PopoverOpen => {
            has_boolean_attribute(element.element, "open")
        }
        PseudoClass::Local { selector } | PseudoClass::Global { selector } => {
            selector_matches_with_dynamic(selector, element, allow_dynamic)
        }
        PseudoClass::Hover
        | PseudoClass::Active
        | PseudoClass::Focus
        | PseudoClass::FocusVisible
        | PseudoClass::FocusWithin
        | PseudoClass::Current
        | PseudoClass::Past
        | PseudoClass::Future
        | PseudoClass::Playing
        | PseudoClass::Paused
        | PseudoClass::Seeking
        | PseudoClass::Buffering
        | PseudoClass::Stalled
        | PseudoClass::Muted
        | PseudoClass::VolumeLocked
        | PseudoClass::Fullscreen(_)
        | PseudoClass::Closed
        | PseudoClass::Modal
        | PseudoClass::PictureInPicture
        | PseudoClass::Target
        | PseudoClass::TargetCurrent
        | PseudoClass::TargetBefore
        | PseudoClass::TargetAfter
        | PseudoClass::TargetWithin
        | PseudoClass::Visited
        | PseudoClass::Default
        | PseudoClass::Indeterminate
        | PseudoClass::Blank
        | PseudoClass::Valid
        | PseudoClass::Invalid
        | PseudoClass::InRange
        | PseudoClass::OutOfRange
        | PseudoClass::UserValid
        | PseudoClass::UserInvalid
        | PseudoClass::Autofill(_)
        | PseudoClass::ActiveViewTransition
        | PseudoClass::ActiveViewTransitionType { .. }
        | PseudoClass::State { .. }
        | PseudoClass::WebKitScrollbar(_)
        | PseudoClass::Custom { .. }
        | PseudoClass::CustomFunction { .. } => allow_dynamic,
        // New parser variants are unsupported until htmlswap can model their
        // runtime state explicitly. Ignore them instead of applying the rule
        // unconditionally or breaking consumers on a compatible parser update.
        _ => false,
    }
}

fn selector_dynamic_conditions(selector: &Selector<'_>) -> Vec<RenderStyleCondition> {
    let mut conditions = Vec::new();
    for component in selector.iter_raw_match_order() {
        collect_component_dynamic_conditions(component, &mut conditions);
    }
    conditions
}

fn selector_has_static_anchor(selector: &Selector<'_>) -> bool {
    selector
        .iter_raw_match_order()
        .any(component_is_static_anchor)
}

fn component_is_static_anchor(component: &Component<'_>) -> bool {
    match component {
        Component::LocalName(_)
        | Component::ID(_)
        | Component::Class(_)
        | Component::AttributeInNoNamespaceExists { .. }
        | Component::AttributeInNoNamespace { .. }
        | Component::AttributeOther(_)
        | Component::Root
        | Component::Scope
        | Component::Empty
        | Component::Nth(_)
        | Component::NthOf(_)
        | Component::Host(_) => true,
        Component::Is(selectors)
        | Component::Where(selectors)
        | Component::Any(_, selectors)
        | Component::Has(selectors)
        | Component::Negation(selectors) => selectors.iter().any(selector_has_static_anchor),
        Component::Slotted(selector) => selector_has_static_anchor(selector),
        Component::ExplicitAnyNamespace
        | Component::ExplicitNoNamespace
        | Component::DefaultNamespace(_)
        | Component::Namespace(_, _)
        | Component::ExplicitUniversalType
        | Component::Combinator(_)
        | Component::NonTSPseudoClass(_)
        | Component::PseudoElement(_)
        | Component::Part(_)
        | Component::Nesting => false,
    }
}

fn collect_component_dynamic_conditions(
    component: &Component<'_>,
    conditions: &mut Vec<RenderStyleCondition>,
) {
    match component {
        Component::NonTSPseudoClass(PseudoClass::ActiveViewTransitionType { kind }) => {
            push_unique_condition(
                conditions,
                RenderStyleCondition::ActiveViewTransitionType(
                    kind.iter()
                        .map(|ident| CompactString::from(ident.0.as_ref()))
                        .collect(),
                ),
            );
        }
        Component::NonTSPseudoClass(pseudo_class) => {
            if let Some(name) = dynamic_pseudo_class_name(pseudo_class) {
                push_unique_condition(
                    conditions,
                    RenderStyleCondition::PseudoClass(CompactString::from(name)),
                );
            }
        }
        Component::PseudoElement(pseudo_element) => {
            if let Some(name) = pseudo_element_name(pseudo_element) {
                push_unique_condition(
                    conditions,
                    RenderStyleCondition::PseudoElement(CompactString::from(name)),
                );
            }
        }
        Component::Is(selectors)
        | Component::Where(selectors)
        | Component::Any(_, selectors)
        | Component::Has(selectors) => {
            for selector in selectors.iter() {
                for component in selector.iter_raw_match_order() {
                    collect_component_dynamic_conditions(component, conditions);
                }
            }
        }
        Component::Negation(_) => {}
        _ => {}
    }
}

fn pseudo_element_name(pseudo_element: &PseudoElement<'_>) -> Option<&'static str> {
    #[allow(
        unreachable_patterns,
        reason = "lightningcss adds pseudo variants in compatible alpha releases"
    )]
    match pseudo_element {
        PseudoElement::Before => Some("before"),
        PseudoElement::After => Some("after"),
        PseudoElement::FirstLine => Some("first-line"),
        PseudoElement::FirstLetter => Some("first-letter"),
        PseudoElement::Marker => Some("marker"),
        PseudoElement::Selection(_) => Some("selection"),
        PseudoElement::Placeholder(_) => Some("placeholder"),
        PseudoElement::FileSelectorButton(_) => Some("file-selector-button"),
        PseudoElement::Backdrop(_) => Some("backdrop"),
        PseudoElement::Cue => Some("cue"),
        PseudoElement::CueFunction { .. } => Some("cue"),
        PseudoElement::CueRegion => Some("cue-region"),
        PseudoElement::CueRegionFunction { .. } => Some("cue-region"),
        PseudoElement::DetailsContent => Some("details-content"),
        PseudoElement::TargetText => Some("target-text"),
        PseudoElement::SearchText => Some("search-text"),
        PseudoElement::HighlightFunction { .. } => Some("highlight"),
        PseudoElement::ViewTransition => Some("view-transition"),
        PseudoElement::ViewTransitionGroup { .. } => Some("view-transition-group"),
        PseudoElement::ViewTransitionImagePair { .. } => Some("view-transition-image-pair"),
        PseudoElement::ViewTransitionNew { .. } => Some("view-transition-new"),
        PseudoElement::ViewTransitionOld { .. } => Some("view-transition-old"),
        PseudoElement::PickerFunction { .. } => Some("picker"),
        PseudoElement::PickerIcon => Some("picker-icon"),
        PseudoElement::Checkmark => Some("checkmark"),
        PseudoElement::GrammarError => Some("grammar-error"),
        PseudoElement::SpellingError => Some("spelling-error"),
        PseudoElement::WebKitScrollbar(_) => Some("-webkit-scrollbar"),
        PseudoElement::Custom { .. } => Some("custom"),
        PseudoElement::CustomFunction { .. } => Some("custom-function"),
        // Preserve forward compatibility with lightningcss while keeping
        // unsupported generated boxes out of the target-neutral render plan.
        _ => None,
    }
}

fn push_unique_condition(
    conditions: &mut Vec<RenderStyleCondition>,
    condition: RenderStyleCondition,
) {
    if !conditions.contains(&condition) {
        conditions.push(condition);
    }
}

fn dynamic_pseudo_class_name(pseudo_class: &PseudoClass<'_>) -> Option<&'static str> {
    match pseudo_class {
        PseudoClass::Hover => Some("hover"),
        PseudoClass::Active => Some("active"),
        PseudoClass::Focus => Some("focus"),
        PseudoClass::FocusVisible => Some("focus-visible"),
        PseudoClass::FocusWithin => Some("focus-within"),
        PseudoClass::Current => Some("current"),
        PseudoClass::Past => Some("past"),
        PseudoClass::Future => Some("future"),
        PseudoClass::Playing => Some("playing"),
        PseudoClass::Paused => Some("paused"),
        PseudoClass::Seeking => Some("seeking"),
        PseudoClass::Buffering => Some("buffering"),
        PseudoClass::Stalled => Some("stalled"),
        PseudoClass::Muted => Some("muted"),
        PseudoClass::VolumeLocked => Some("volume-locked"),
        PseudoClass::Fullscreen(_) => Some("fullscreen"),
        PseudoClass::Closed => Some("closed"),
        PseudoClass::Modal => Some("modal"),
        PseudoClass::PictureInPicture => Some("picture-in-picture"),
        PseudoClass::Target => Some("target"),
        PseudoClass::TargetCurrent => Some("target-current"),
        PseudoClass::TargetBefore => Some("target-before"),
        PseudoClass::TargetAfter => Some("target-after"),
        PseudoClass::TargetWithin => Some("target-within"),
        PseudoClass::Visited => Some("visited"),
        PseudoClass::Default => Some("default"),
        PseudoClass::Indeterminate => Some("indeterminate"),
        PseudoClass::Blank => Some("blank"),
        PseudoClass::Valid => Some("valid"),
        PseudoClass::Invalid => Some("invalid"),
        PseudoClass::InRange => Some("in-range"),
        PseudoClass::OutOfRange => Some("out-of-range"),
        PseudoClass::UserValid => Some("user-valid"),
        PseudoClass::UserInvalid => Some("user-invalid"),
        PseudoClass::Autofill(_) => Some("autofill"),
        PseudoClass::ActiveViewTransition => Some("active-view-transition"),
        PseudoClass::ActiveViewTransitionType { .. } => Some("active-view-transition-type"),
        PseudoClass::State { .. } => Some("state"),
        PseudoClass::WebKitScrollbar(_) => Some("-webkit-scrollbar"),
        PseudoClass::Custom { .. } => Some("custom"),
        PseudoClass::CustomFunction { .. } => Some("custom-function"),
        _ => None,
    }
}

fn nth_matches(element: &StyleElement<'_, '_>, nth_type: NthType, a: i32, b: i32) -> bool {
    match nth_type {
        NthType::Child => nth_index_matches(element_index(element, false, false), a, b),
        NthType::LastChild => nth_index_matches(element_index(element, false, true), a, b),
        NthType::OnlyChild => {
            nth_index_matches(element_index(element, false, false), 0, 1)
                && nth_index_matches(element_index(element, false, true), 0, 1)
        }
        NthType::OfType => nth_index_matches(element_index(element, true, false), a, b),
        NthType::LastOfType => nth_index_matches(element_index(element, true, true), a, b),
        NthType::OnlyOfType => {
            nth_index_matches(element_index(element, true, false), 0, 1)
                && nth_index_matches(element_index(element, true, true), 0, 1)
        }
        NthType::Col | NthType::LastCol => false,
    }
}

fn nth_of_matches(
    element: &StyleElement<'_, '_>,
    nth_type: NthType,
    a: i32,
    b: i32,
    selectors: &[Selector<'_>],
) -> bool {
    match nth_type {
        NthType::Child | NthType::LastChild => {
            let from_end = nth_type == NthType::LastChild;
            nth_index_matches(
                element_index_matching_selectors(element, selectors, from_end),
                a,
                b,
            )
        }
        _ => nth_matches(element, nth_type, a, b),
    }
}

fn nth_index_matches(index: Option<usize>, a: i32, b: i32) -> bool {
    let Some(index) = index else {
        return false;
    };

    let index = index as i32;
    if a == 0 {
        return index == b;
    }

    let offset = index - b;
    offset % a == 0 && offset / a >= 0
}

fn element_index(element: &StyleElement<'_, '_>, of_type: bool, from_end: bool) -> Option<usize> {
    let siblings: Box<dyn Iterator<Item = (usize, &HtmlNode)> + '_> = if from_end {
        Box::new(element.siblings.iter().enumerate().rev())
    } else {
        Box::new(element.siblings.iter().enumerate())
    };

    let mut index = 0;
    for (sibling_index, sibling) in siblings {
        let HtmlNode::Element(sibling_element) = sibling else {
            continue;
        };

        if of_type && !same_element_type(element.element, sibling_element) {
            continue;
        }

        index += 1;
        if sibling_index == element.sibling_index {
            return Some(index);
        }
    }

    None
}

fn element_index_matching_selectors(
    element: &StyleElement<'_, '_>,
    selectors: &[Selector<'_>],
    from_end: bool,
) -> Option<usize> {
    let siblings: Box<dyn Iterator<Item = (usize, &HtmlNode)> + '_> = if from_end {
        Box::new(element.siblings.iter().enumerate().rev())
    } else {
        Box::new(element.siblings.iter().enumerate())
    };

    let mut index = 0;
    for (sibling_index, sibling) in siblings {
        let HtmlNode::Element(sibling_element) = sibling else {
            continue;
        };

        let sibling = StyleElement {
            element: sibling_element,
            ancestors: element.ancestors,
            siblings: element.siblings,
            sibling_index,
        };

        if !selectors
            .iter()
            .any(|selector| selector_matches(selector, &sibling))
        {
            continue;
        }

        index += 1;
        if sibling_index == element.sibling_index {
            return Some(index);
        }
    }

    None
}

fn descendant_matches(
    selector: &Selector<'_>,
    element: &StyleElement<'_, '_>,
    allow_dynamic: bool,
) -> bool {
    let mut ancestors = element.ancestors.to_vec();
    ancestors.push(element.as_ancestor());
    descendant_matches_with_ancestors(
        selector,
        &element.element.children,
        &mut ancestors,
        allow_dynamic,
    )
}

fn descendant_matches_with_ancestors<'a>(
    selector: &Selector<'_>,
    siblings: &'a [HtmlNode],
    ancestors: &mut Vec<StyleAncestor<'a>>,
    allow_dynamic: bool,
) -> bool {
    for (sibling_index, sibling) in siblings.iter().enumerate() {
        let HtmlNode::Element(element) = sibling else {
            continue;
        };

        if selector_matches_with_dynamic(
            selector,
            &StyleElement {
                element,
                ancestors,
                siblings,
                sibling_index,
            },
            allow_dynamic,
        ) {
            return true;
        }

        ancestors.push(StyleAncestor::new(element, siblings, sibling_index));
        if descendant_matches_with_ancestors(selector, &element.children, ancestors, allow_dynamic)
        {
            return true;
        }
        ancestors.pop();
    }

    false
}

fn element_is_empty(element: &HtmlElement) -> bool {
    element.children.iter().all(|child| match child {
        HtmlNode::Element(_) => false,
        HtmlNode::Text(text) => text.value.is_empty(),
        HtmlNode::Comment(_) => true,
    })
}

fn same_element_type(left: &HtmlElement, right: &HtmlElement) -> bool {
    left.name.namespace() == right.name.namespace()
        && left.name.local().eq_ignore_ascii_case(right.name.local())
}

fn is_html_element(element: &StyleElement<'_, '_>) -> bool {
    element
        .element
        .name
        .namespace()
        .is_none_or(|namespace| namespace == "http://www.w3.org/1999/xhtml")
}

fn element_classes(element: &HtmlElement) -> impl Iterator<Item = &str> {
    attribute_value(element, "class")
        .into_iter()
        .flat_map(str::split_whitespace)
}

fn attribute_value<'a>(element: &'a HtmlElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local().eq_ignore_ascii_case(name))
        .map(|attribute| attribute.value.as_str())
}

fn has_boolean_attribute(element: &HtmlElement, name: &str) -> bool {
    element
        .attributes
        .iter()
        .any(|attribute| attribute.name.local().eq_ignore_ascii_case(name))
}

fn inherited_attribute<'tree>(element: &StyleElement<'tree, '_>, name: &str) -> Option<&'tree str> {
    attribute_value(element.element, name).or_else(|| {
        element
            .ancestors
            .iter()
            .rev()
            .find_map(|ancestor| attribute_value(ancestor.element, name))
    })
}

fn lang_matches(lang: &str, expected: &str) -> bool {
    lang.eq_ignore_ascii_case(expected)
        || lang
            .strip_prefix(expected)
            .is_some_and(|suffix| suffix.starts_with('-'))
}

fn direction_matches(dir: &str, expected: Direction) -> bool {
    matches!(
        (dir.to_ascii_lowercase().as_str(), expected),
        ("ltr", Direction::Ltr) | ("rtl", Direction::Rtl)
    )
}

fn is_link(element: &StyleElement<'_, '_>) -> bool {
    matches!(element.element.name.local(), "a" | "area" | "link")
        && attribute_value(element.element, "href").is_some()
}

fn is_form_control(element: &HtmlElement) -> bool {
    matches!(
        element.name.local(),
        "button" | "fieldset" | "input" | "optgroup" | "option" | "select" | "textarea"
    )
}

fn is_disableable(element: &HtmlElement) -> bool {
    is_form_control(element)
}

fn is_disabled(element: &StyleElement<'_, '_>) -> bool {
    has_boolean_attribute(element.element, "disabled")
}

fn is_text_editable(element: &HtmlElement) -> bool {
    matches!(element.name.local(), "input" | "textarea")
}
