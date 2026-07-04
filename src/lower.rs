use std::collections::BTreeSet;

use compact_str::CompactString;
use url::Url;

use crate::css::{
    StyleAncestor, StyleElement, StyleIndex, parse_style_attribute as parse_css_style_attribute,
};
use crate::diagnostics::{Compilation, Diagnostic, Diagnostics};
use crate::expr::{Expr, TemplateString};
use crate::frontend::{DialectContext, Frontend};
use crate::ir::{HtmlDocument, HtmlElement, HtmlNode};
use crate::plan::{
    ActionBinding, ActionPayload, ComponentId, RegionId, RenderAccessibility, RenderActionHandler,
    RenderAnnotation, RenderAnnotationKind, RenderAttribute, RenderChoiceOption, RenderChoiceState,
    RenderControlFlow, RenderControlFlowKind, RenderDensity, RenderDynamicStyleBinding,
    RenderElement, RenderFormControl, RenderFormControlType, RenderFormDataField,
    RenderHeadElement, RenderNode, RenderPlan, RenderPseudoElement, RenderRaw, RenderRoot,
    RenderScriptKind, RenderScriptReference, RenderSemanticExtra, RenderSemantics, RenderSize,
    RenderSourceIntent, RenderSourceLogic, RenderSourceProp, RenderStateBinding, RenderStateKind,
    RenderStateOwner, RenderStyleCondition, RenderStyleVariant, RenderText, RenderTextInputState,
    RenderThemePlan, RenderToggleState, RenderTone, RenderValidation, RenderValidationConstraint,
    RenderVariant, SlotId, UiRole,
};
use crate::script::{ActionIndex, ScriptStyleDeclaration, ScriptStyleIndex, analyze_event_handler};
use crate::source::{SourceId, SourceKind, SourceMap, Span};
use crate::style::{StyleDeclaration, StyleProperty};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LowerOptions {
    pub keep_whitespace_text: bool,
    pub drop_script_and_style: bool,
    pub drop_document_metadata: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LowerContext<'a> {
    pub(crate) styles: Option<&'a StyleIndex>,
    pub actions: Option<&'a ActionIndex>,
    pub(crate) script_styles: Option<&'a ScriptStyleIndex>,
    pub sources: Option<&'a SourceMap>,
    pub frontend: Option<&'a Frontend>,
}

#[derive(Debug, Default)]
struct LowerRuntime {
    diagnostics: Diagnostics,
    state_planner: StatePlanner,
    scripts: Vec<RenderScriptReference>,
    source_logic: Vec<RenderSourceLogic>,
    head: Vec<RenderHeadElement>,
    regions: Vec<RegionId>,
    bindings: Vec<ScopedBinding>,
}

#[derive(Debug, Clone)]
struct ScopedBinding {
    name: CompactString,
    path: Vec<CompactString>,
}

impl Default for LowerOptions {
    fn default() -> Self {
        Self {
            keep_whitespace_text: false,
            drop_script_and_style: true,
            drop_document_metadata: true,
        }
    }
}

#[must_use]
pub fn lower_document(document: &HtmlDocument, options: &LowerOptions) -> Compilation<RenderPlan> {
    lower_document_with_context(document, options, &LowerContext::default())
}

#[must_use]
pub(crate) fn lower_document_with_context(
    document: &HtmlDocument,
    options: &LowerOptions,
    context: &LowerContext<'_>,
) -> Compilation<RenderPlan> {
    let mut runtime = LowerRuntime::default();
    let default_frontend = Frontend::default();
    let context = LowerContext {
        frontend: Some(context.frontend.unwrap_or(&default_frontend)),
        ..*context
    };
    let mut ancestors = Vec::new();
    let mut annotations = Vec::new();
    collect_html_comment_annotations(&document.nodes, &mut annotations);
    collect_source_metadata_annotations(&document.nodes, &mut annotations);
    collect_head_elements(&document.nodes, context.sources, false, &mut runtime.head);

    let root_nodes = render_root_nodes(&document.nodes, context.frontend.expect("frontend is set"));
    let nodes: Vec<RenderNode> = root_nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| {
            lower_node(
                node,
                root_nodes,
                index,
                &mut ancestors,
                options,
                &context,
                &mut runtime,
            )
        })
        .collect();
    let root = render_root_for_document(document, &context, &mut runtime);
    let mut theme = context
        .styles
        .map_or_else(RenderThemePlan::default, StyleIndex::theme_plan);
    theme.collect_from_root(&root);
    theme.collect_from_nodes(&nodes);

    Compilation::new(
        RenderPlan::new(nodes)
            .with_root(root)
            .with_head(runtime.head)
            .with_annotations(annotations)
            .with_scripts(runtime.scripts)
            .with_source_logic(runtime.source_logic)
            .with_theme(theme),
        runtime.diagnostics,
    )
}

fn render_root_nodes<'a>(nodes: &'a [HtmlNode], frontend: &Frontend) -> &'a [HtmlNode] {
    let nodes = html_render_root_nodes(nodes);
    frontend_render_root_nodes(nodes, frontend)
}

fn html_render_root_nodes(nodes: &[HtmlNode]) -> &[HtmlNode] {
    let [HtmlNode::Element(root)] = nodes else {
        return nodes;
    };

    if !root.name.local().eq_ignore_ascii_case("html") {
        return nodes;
    }

    root.children
        .iter()
        .find_map(|node| {
            let HtmlNode::Element(element) = node else {
                return None;
            };
            element
                .name
                .local()
                .eq_ignore_ascii_case("body")
                .then_some(element.children.as_slice())
        })
        .unwrap_or(root.children.as_slice())
}

fn frontend_render_root_nodes<'a>(nodes: &'a [HtmlNode], frontend: &Frontend) -> &'a [HtmlNode] {
    let [HtmlNode::Element(root)] = nodes else {
        return nodes;
    };

    if frontend.unwrap_root_element(root) {
        root.children.as_slice()
    } else {
        nodes
    }
}

fn render_root_for_document(
    document: &HtmlDocument,
    context: &LowerContext<'_>,
    runtime: &mut LowerRuntime,
) -> RenderRoot {
    let [HtmlNode::Element(html)] = document.nodes.as_slice() else {
        return RenderRoot::default();
    };

    if !html.name.local().eq_ignore_ascii_case("html") {
        return RenderRoot::default();
    }

    let mut root = RenderRoot::default();
    merge_root_element_styles(&mut root, html, &document.nodes, 0, &[], context, runtime);

    if let Some((body_index, body)) = html.children.iter().enumerate().find_map(|(index, node)| {
        let HtmlNode::Element(element) = node else {
            return None;
        };
        element
            .name
            .local()
            .eq_ignore_ascii_case("body")
            .then_some((index, element))
    }) {
        let ancestors = [StyleAncestor::new(html, &document.nodes, 0)];
        merge_root_element_styles(
            &mut root,
            body,
            &html.children,
            body_index,
            &ancestors,
            context,
            runtime,
        );
    }

    root
}

fn merge_root_element_styles<'a>(
    root: &mut RenderRoot,
    element: &'a HtmlElement,
    siblings: &'a [HtmlNode],
    sibling_index: usize,
    ancestors: &[StyleAncestor<'a>],
    context: &LowerContext<'_>,
    runtime: &mut LowerRuntime,
) {
    let (styles, style_variants) = root_styles_for_element(
        element,
        siblings,
        sibling_index,
        ancestors,
        context,
        runtime,
    );

    if !styles.is_empty() || !style_variants.is_empty() {
        root.span = element.span.or(root.span);
    }
    merge_inline_styles(&mut root.styles, styles);
    root.style_variants.extend(style_variants);
}

fn root_styles_for_element<'a>(
    element: &'a HtmlElement,
    siblings: &'a [HtmlNode],
    sibling_index: usize,
    ancestors: &[StyleAncestor<'a>],
    context: &LowerContext<'_>,
    runtime: &mut LowerRuntime,
) -> (Vec<StyleDeclaration>, Vec<RenderStyleVariant>) {
    let (mut styles, style_variants) = context.styles.map_or_else(
        || (Vec::new(), Vec::new()),
        |styles| {
            let style_element = StyleElement::new(element, ancestors, siblings, sibling_index);
            let element_styles = styles.styles_for_element(&style_element);
            (element_styles.declarations, element_styles.variants)
        },
    );

    let mut inline_styles = Vec::new();
    let mut inline_style_variants = Vec::new();
    for attribute in &element.attributes {
        let name = attribute.name.local().to_ascii_lowercase();
        match name.as_str() {
            "style" => {
                if let Some(expression) =
                    parse_template(context, &attribute.value, attribute.span, runtime)
                {
                    if let Some(styles) = script_styles_for_dynamic_expression(
                        context,
                        &runtime.bindings,
                        &expression,
                    ) {
                        inline_styles.extend(style_declarations_from_script(styles));
                    } else {
                        inline_styles.extend(static_declarations_from_dynamic_style_value(
                            &attribute.value,
                            attribute.span,
                            runtime,
                        ));
                        runtime.diagnostics.push(Diagnostic::warning(
                            "dynamic root style attributes are not supported yet",
                            attribute.span.or(element.span),
                        ));
                    }
                } else {
                    let parsed_style = parse_css_style_attribute(&attribute.value, attribute.span);
                    runtime.diagnostics.extend(parsed_style.diagnostics);
                    inline_styles.extend(parsed_style.value);
                }
            }
            _ if name
                .strip_prefix("style-")
                .is_some_and(|state| !state.is_empty()) =>
            {
                if let Some(expression) =
                    parse_template(context, &attribute.value, attribute.span, runtime)
                {
                    if let Some(styles) = script_styles_for_dynamic_expression(
                        context,
                        &runtime.bindings,
                        &expression,
                    ) {
                        inline_style_variants.push(RenderStyleVariant {
                            conditions: vec![RenderStyleCondition::PseudoClass(
                                CompactString::from(
                                    name.strip_prefix("style-")
                                        .expect("style-* attribute was matched"),
                                ),
                            )],
                            selector: CompactString::from(format!("[{name}]")),
                            declarations: style_declarations_from_script(styles),
                            span: attribute.span,
                        });
                    } else {
                        let declarations = static_declarations_from_dynamic_style_value(
                            &attribute.value,
                            attribute.span,
                            runtime,
                        );
                        if !declarations.is_empty() {
                            inline_style_variants.push(RenderStyleVariant {
                                conditions: vec![RenderStyleCondition::PseudoClass(
                                    CompactString::from(
                                        name.strip_prefix("style-")
                                            .expect("style-* attribute was matched"),
                                    ),
                                )],
                                selector: CompactString::from(format!("[{name}]")),
                                declarations,
                                span: attribute.span,
                            });
                        }
                        runtime.diagnostics.push(Diagnostic::warning(
                            "dynamic root style variants are not supported yet",
                            attribute.span.or(element.span),
                        ));
                    }
                } else {
                    let parsed_style = parse_css_style_attribute(&attribute.value, attribute.span);
                    runtime.diagnostics.extend(parsed_style.diagnostics);
                    if !parsed_style.value.is_empty() {
                        inline_style_variants.push(RenderStyleVariant {
                            conditions: vec![RenderStyleCondition::PseudoClass(
                                CompactString::from(
                                    name.strip_prefix("style-")
                                        .expect("style-* attribute was matched"),
                                ),
                            )],
                            selector: CompactString::from(format!("[{name}]")),
                            declarations: parsed_style.value,
                            span: attribute.span,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    merge_inline_styles(&mut styles, inline_styles);
    let mut style_variants = style_variants;
    style_variants.extend(inline_style_variants);
    let (style_variants, _pseudo_elements) = split_pseudo_elements(style_variants);
    (styles, style_variants)
}

fn collect_html_comment_annotations(nodes: &[HtmlNode], annotations: &mut Vec<RenderAnnotation>) {
    for node in nodes {
        match node {
            HtmlNode::Element(element) => {
                collect_html_comment_annotations(&element.children, annotations);
            }
            HtmlNode::Comment(comment) => {
                annotations.push(RenderAnnotation::new(
                    RenderAnnotationKind::HtmlComment,
                    comment.value.clone(),
                    comment.span,
                ));
            }
            HtmlNode::Text(_) => {}
        }
    }
}

fn collect_source_metadata_annotations(
    nodes: &[HtmlNode],
    annotations: &mut Vec<RenderAnnotation>,
) {
    for node in nodes {
        let HtmlNode::Element(element) = node else {
            continue;
        };

        if element.name.local().eq_ignore_ascii_case("script")
            && has_html_boolean_attribute(element, "data-dc-script")
            && let Some(data_props) = html_attribute_value(element, "data-props")
        {
            annotations.push(RenderAnnotation::new(
                RenderAnnotationKind::SourceMetadata,
                format!("data-dc-script data-props={data_props}"),
                html_attribute_span(element, "data-props").or(element.span),
            ));
        }

        collect_source_metadata_annotations(&element.children, annotations);
    }
}

fn collect_head_elements(
    nodes: &[HtmlNode],
    sources: Option<&SourceMap>,
    in_head: bool,
    head: &mut Vec<RenderHeadElement>,
) {
    for node in nodes {
        let HtmlNode::Element(element) = node else {
            continue;
        };
        let tag = element.name.local().to_ascii_lowercase();
        let element_is_head = matches!(tag.as_str(), "head" | "helmet");
        let child_in_head = in_head || element_is_head;

        if child_in_head && is_preserved_head_element(&tag) {
            head.push(RenderHeadElement {
                html: head_element_html(element, sources),
                span: element.span,
            });
            continue;
        }

        collect_head_elements(&element.children, sources, child_in_head, head);
    }
}

fn is_preserved_head_element(tag: &str) -> bool {
    matches!(tag, "link" | "meta" | "style" | "title")
}

fn head_element_html(element: &HtmlElement, sources: Option<&SourceMap>) -> String {
    if let Some(html) = element
        .span
        .and_then(|span| sources.and_then(|sources| sources.source_text(span)))
    {
        return html.to_owned();
    }

    render_head_element_fallback(element)
}

fn render_head_element_fallback(element: &HtmlElement) -> String {
    let mut output = String::new();
    output.push('<');
    output.push_str(element.name.local());
    for attribute in &element.attributes {
        output.push(' ');
        output.push_str(attribute.name.local());
        if !attribute.value.is_empty() {
            output.push_str("=\"");
            output.push_str(&escape_head_attribute(&attribute.value));
            output.push('"');
        }
    }
    if element.children.is_empty() && matches!(element.name.local(), "link" | "meta") {
        output.push('>');
        return output;
    }
    output.push('>');
    for child in &element.children {
        match child {
            HtmlNode::Text(text) => output.push_str(&text.value),
            HtmlNode::Comment(comment) => {
                output.push_str("<!--");
                output.push_str(&comment.value);
                output.push_str("-->");
            }
            HtmlNode::Element(_) => {}
        }
    }
    output.push_str("</");
    output.push_str(element.name.local());
    output.push('>');
    output
}

fn escape_head_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn lower_node<'a>(
    node: &'a HtmlNode,
    siblings: &'a [HtmlNode],
    sibling_index: usize,
    ancestors: &mut Vec<StyleAncestor<'a>>,
    options: &LowerOptions,
    context: &LowerContext<'_>,
    runtime: &mut LowerRuntime,
) -> Option<RenderNode> {
    match node {
        HtmlNode::Element(element) => lower_element(
            element,
            siblings,
            sibling_index,
            ancestors,
            options,
            context,
            runtime,
        ),
        HtmlNode::Text(text) => {
            if !options.keep_whitespace_text && text.value.trim().is_empty() {
                return None;
            }

            let template = parse_template(context, &text.value, text.span, runtime);
            Some(RenderNode::Text(RenderText {
                value: text.value.clone(),
                template,
                span: text.span,
            }))
        }
        HtmlNode::Comment(_) => None,
    }
}

fn lower_element<'a>(
    element: &'a HtmlElement,
    siblings: &'a [HtmlNode],
    sibling_index: usize,
    ancestors: &mut Vec<StyleAncestor<'a>>,
    options: &LowerOptions,
    context: &LowerContext<'_>,
    runtime: &mut LowerRuntime,
) -> Option<RenderNode> {
    let tag = CompactString::from(element.name.local().to_ascii_lowercase());

    if options.drop_document_metadata && is_document_metadata_element(&tag) {
        collect_script_references(element, context, &mut runtime.scripts);
        return None;
    }

    if is_raw_escape(element) || is_foreign_element(element) {
        return Some(lower_raw_element(
            element,
            context,
            &mut runtime.diagnostics,
        ));
    }

    if tag == "script" {
        if let Some(logic) = source_logic_for_element(element) {
            runtime.source_logic.push(logic);
            if options.drop_script_and_style {
                return None;
            }
        }
        if let Some(script) = script_reference_for_element(element, context) {
            runtime.scripts.push(script);
        }
        if options.drop_script_and_style {
            return None;
        }
    }

    if options.drop_script_and_style && tag == "style" {
        return None;
    }

    let frontend = context
        .frontend
        .expect("lowering context should have a frontend");
    let normalized_element;
    let lowered_element = if frontend.has_dialects() {
        normalized_element =
            normalize_source_element(frontend, element, siblings, sibling_index, runtime);
        &normalized_element
    } else {
        element
    };

    let mut attributes = Vec::new();
    let mut classes = Vec::new();
    let mut inline_styles = Vec::new();
    let mut inline_style_variants = Vec::new();
    let mut dynamic_styles = Vec::new();
    let mut actions = Vec::new();
    let directives = {
        let mut dialect_context = DialectContext::new(&mut runtime.diagnostics);
        frontend.lower_element(&tag, lowered_element, &mut dialect_context)
    };
    let control_flow = directives.control_flow.clone();

    for attribute in &lowered_element.attributes {
        let name = CompactString::from(attribute.name.local().to_ascii_lowercase());

        match name.as_str() {
            "class" => {
                if let Some(template) =
                    parse_template(context, &attribute.value, attribute.span, runtime)
                {
                    attributes.push(RenderAttribute {
                        name,
                        value: attribute.value.clone(),
                        template: Some(template),
                        span: attribute.span,
                    });
                } else {
                    classes.extend(attribute.value.split_whitespace().map(CompactString::from));
                }
            }
            "style" => {
                if let Some(expression) =
                    parse_template(context, &attribute.value, attribute.span, runtime)
                {
                    if let Some(styles) = script_styles_for_dynamic_expression(
                        context,
                        &runtime.bindings,
                        &expression,
                    ) {
                        inline_styles.extend(style_declarations_from_script(styles));
                    } else {
                        inline_styles.extend(static_declarations_from_dynamic_style_value(
                            &attribute.value,
                            attribute.span,
                            runtime,
                        ));
                    }
                    dynamic_styles.push(RenderDynamicStyleBinding {
                        state: None,
                        expression,
                        span: attribute.span,
                    });
                } else {
                    let parsed_style = parse_css_style_attribute(&attribute.value, attribute.span);
                    runtime.diagnostics.extend(parsed_style.diagnostics);
                    inline_styles.extend(parsed_style.value);
                }
            }
            "data-htmlswap-style" => {
                if let Some(expression) =
                    parse_template(context, &attribute.value, attribute.span, runtime)
                {
                    inline_styles.extend(static_declarations_from_dynamic_style_value(
                        &attribute.value,
                        attribute.span,
                        runtime,
                    ));
                    dynamic_styles.push(RenderDynamicStyleBinding {
                        state: None,
                        expression,
                        span: attribute.span,
                    });
                }
            }
            _ if let Some(state) = name
                .strip_prefix("data-htmlswap-style-")
                .filter(|state| !state.is_empty()) =>
            {
                if let Some(expression) =
                    parse_template(context, &attribute.value, attribute.span, runtime)
                {
                    let declarations = static_declarations_from_dynamic_style_value(
                        &attribute.value,
                        attribute.span,
                        runtime,
                    );
                    if !declarations.is_empty() {
                        inline_style_variants.push(RenderStyleVariant {
                            conditions: vec![RenderStyleCondition::PseudoClass(
                                CompactString::from(state),
                            )],
                            selector: CompactString::from(format!("[{name}]")),
                            declarations,
                            span: attribute.span,
                        });
                    }
                    dynamic_styles.push(RenderDynamicStyleBinding {
                        state: Some(CompactString::from(state)),
                        expression,
                        span: attribute.span,
                    });
                }
            }
            _ if let Some(state) = name
                .strip_prefix("style-")
                .filter(|state| !state.is_empty()) =>
            {
                if let Some(expression) =
                    parse_template(context, &attribute.value, attribute.span, runtime)
                {
                    if let Some(styles) = script_styles_for_dynamic_expression(
                        context,
                        &runtime.bindings,
                        &expression,
                    ) {
                        inline_style_variants.push(RenderStyleVariant {
                            conditions: vec![RenderStyleCondition::PseudoClass(
                                CompactString::from(state),
                            )],
                            selector: CompactString::from(format!("[{name}]")),
                            declarations: style_declarations_from_script(styles),
                            span: attribute.span,
                        });
                    } else {
                        let declarations = static_declarations_from_dynamic_style_value(
                            &attribute.value,
                            attribute.span,
                            runtime,
                        );
                        if !declarations.is_empty() {
                            inline_style_variants.push(RenderStyleVariant {
                                conditions: vec![RenderStyleCondition::PseudoClass(
                                    CompactString::from(state),
                                )],
                                selector: CompactString::from(format!("[{name}]")),
                                declarations,
                                span: attribute.span,
                            });
                        }
                    }
                    dynamic_styles.push(RenderDynamicStyleBinding {
                        state: Some(CompactString::from(state)),
                        expression,
                        span: attribute.span,
                    });
                } else {
                    let parsed_style = parse_css_style_attribute(&attribute.value, attribute.span);
                    runtime.diagnostics.extend(parsed_style.diagnostics);
                    if !parsed_style.value.is_empty() {
                        inline_style_variants.push(RenderStyleVariant {
                            conditions: vec![RenderStyleCondition::PseudoClass(
                                CompactString::from(state),
                            )],
                            selector: CompactString::from(format!("[{name}]")),
                            declarations: parsed_style.value,
                            span: attribute.span,
                        });
                    }
                }
            }
            _ if name.starts_with("on") => {
                let binding =
                    lower_action_binding(&name, &attribute.value, attribute.span, context, runtime);
                if binding.action.is_some() && !binding.resolved {
                    runtime.diagnostics.push(Diagnostic::warning(
                        format!("event handler attribute `{name}` is unresolved"),
                        attribute.span.or(element.span),
                    ));
                }
                actions.push(binding);
            }
            _ if directives.consumes_attribute(&name) => {}
            _ => {
                let template = parse_template(context, &attribute.value, attribute.span, runtime);
                attributes.push(RenderAttribute {
                    name,
                    value: attribute.value.clone(),
                    template,
                    span: attribute.span,
                });
            }
        }
    }

    let (mut styles, style_variants, stylesheet_rules, stylesheet_winners) =
        context.styles.map_or_else(
            || (Vec::new(), Vec::new(), Vec::new(), Vec::new()),
            |styles| {
                let style_element = StyleElement::new(element, ancestors, siblings, sibling_index);
                let element_styles = styles.styles_for_element(&style_element);
                let stylesheet_winners = element_styles.declarations;
                (
                    stylesheet_winners.clone(),
                    element_styles.variants,
                    element_styles.matched_rules,
                    stylesheet_winners,
                )
            },
        );
    let inline_properties = inline_styles
        .iter()
        .map(|style| style.property.clone())
        .collect::<Vec<_>>();
    merge_inline_styles(&mut styles, inline_styles);
    let stylesheet_declarations = stylesheet_winners
        .into_iter()
        .filter(|declaration| !inline_properties.contains(&declaration.property))
        .filter(|declaration| {
            styles
                .iter()
                .find(|style| style.property == declaration.property)
                .is_some_and(|style| style_declarations_match(style, declaration))
        })
        .collect::<Vec<_>>();
    let mut style_variants = style_variants;
    style_variants.extend(inline_style_variants);
    let (style_variants, pseudo_elements) = split_pseudo_elements(style_variants);
    let semantics = semantics_for_element(lowered_element);
    let own_region = region_for_element(lowered_element, &mut runtime.diagnostics);
    let region = own_region
        .clone()
        .or_else(|| runtime.regions.last().cloned());

    if let Some(region) = &own_region {
        runtime.regions.push(region.clone());
    }
    let scoped_binding = control_flow
        .as_ref()
        .and_then(|control_flow| scoped_binding_for_control_flow(control_flow, &runtime.bindings));
    if let Some(binding) = &scoped_binding {
        runtime.bindings.push(binding.clone());
    }
    ancestors.push(StyleAncestor::new(element, siblings, sibling_index));
    let children: Vec<RenderNode> = element
        .children
        .iter()
        .enumerate()
        .filter_map(|(index, node)| {
            lower_node(
                node,
                &element.children,
                index,
                ancestors,
                options,
                context,
                runtime,
            )
        })
        .collect();
    ancestors.pop();
    if scoped_binding.is_some() {
        runtime.bindings.pop();
    }
    if own_region.is_some() {
        runtime.regions.pop();
    }

    let state = state_binding_for_control(lowered_element, &tag, &children, context, runtime);
    let form_control = form_control_for_text_control(lowered_element, &tag, ancestors, siblings);
    attach_action_payloads(&mut actions, state.as_ref(), &children);
    let accessibility = accessibility_for_element(lowered_element);
    let source_intent = source_intent_for_element(lowered_element, context, runtime);

    Some(RenderNode::Element(Box::new(RenderElement {
        role: role_for_tag(&tag),
        source_tag: tag,
        attributes,
        classes,
        styles,
        stylesheet_rules,
        stylesheet_declarations,
        style_variants,
        dynamic_styles,
        pseudo_elements,
        actions,
        state: state.map(Box::new),
        form_control: form_control.map(Box::new),
        accessibility: accessibility.map(Box::new),
        control_flow: control_flow.map(Box::new),
        source_intent: source_intent.map(Box::new),
        semantics: semantics.map(Box::new),
        region,
        children,
        span: element.span,
    })))
}

fn normalize_source_element(
    frontend: &Frontend,
    element: &HtmlElement,
    siblings: &[HtmlNode],
    sibling_index: usize,
    runtime: &mut LowerRuntime,
) -> HtmlElement {
    let mut normalized = element.clone();
    let mut attributes = Vec::with_capacity(element.attributes.len());
    let mut dialect_context = DialectContext::new(&mut runtime.diagnostics);
    for attribute in &element.attributes {
        if let Some(rewrite) = frontend.rewrite_attribute(element, attribute, &mut dialect_context)
        {
            attributes.extend(rewrite.into_attributes());
        } else {
            attributes.push(attribute.clone());
        }
    }
    normalized.attributes = attributes;
    let inferred_attributes = frontend.infer_element_attributes(
        &normalized,
        siblings,
        sibling_index,
        &mut dialect_context,
    );
    normalized.attributes.extend(inferred_attributes);
    normalized
}

fn is_document_metadata_element(tag: &str) -> bool {
    matches!(
        tag,
        "head" | "base" | "link" | "meta" | "title" | "helmet" | "noscript"
    )
}

fn collect_script_references(
    element: &HtmlElement,
    context: &LowerContext<'_>,
    scripts: &mut Vec<RenderScriptReference>,
) {
    for node in &element.children {
        let HtmlNode::Element(child) = node else {
            continue;
        };

        if child.name.local().eq_ignore_ascii_case("script")
            && let Some(script) = script_reference_for_element(child, context)
        {
            scripts.push(script);
        }

        collect_script_references(child, context, scripts);
    }
}

fn lower_raw_element(
    element: &HtmlElement,
    context: &LowerContext<'_>,
    diagnostics: &mut Diagnostics,
) -> RenderNode {
    let html = element
        .span
        .and_then(|span| {
            context
                .sources
                .and_then(|sources| sources.source_text(span))
        })
        .map(str::to_owned)
        .unwrap_or_else(|| {
            diagnostics.push(Diagnostic::warning(
                "raw element source is unavailable",
                element.span,
            ));
            String::new()
        });

    RenderNode::Raw(RenderRaw {
        html,
        span: element.span,
    })
}

fn is_raw_escape(element: &HtmlElement) -> bool {
    element.attributes.iter().any(|attribute| {
        attribute
            .name
            .local()
            .eq_ignore_ascii_case("data-htmlswap-raw")
    })
}

fn is_foreign_element(element: &HtmlElement) -> bool {
    element
        .name
        .namespace()
        .is_some_and(|namespace| namespace != "http://www.w3.org/1999/xhtml" && namespace != "html")
}

fn merge_inline_styles(styles: &mut Vec<StyleDeclaration>, inline_styles: Vec<StyleDeclaration>) {
    for inline_style in inline_styles {
        if let Some(existing) = styles
            .iter_mut()
            .find(|style| style.property == inline_style.property)
        {
            *existing = inline_style;
        } else {
            styles.push(inline_style);
        }
    }
}

fn style_declarations_match(left: &StyleDeclaration, right: &StyleDeclaration) -> bool {
    left.property == right.property
        && left.value == right.value
        && left.important == right.important
}

fn script_styles_for_dynamic_expression<'a>(
    context: &'a LowerContext<'_>,
    bindings: &[ScopedBinding],
    expression: &TemplateString,
) -> Option<&'a [ScriptStyleDeclaration]> {
    let script_styles = context.script_styles?;
    if let Some(path) = style_object_lookup_path(expression, bindings)
        && let Some(declarations) =
            script_styles.unique_declarations_for_path(path.iter().map(AsRef::as_ref))
    {
        return Some(declarations);
    }

    script_styles.unique_declarations(style_object_lookup_name(expression)?)
}

fn style_object_lookup_name(expression: &TemplateString) -> Option<&str> {
    let Expr::Path(path) = expression.single_expression()? else {
        return None;
    };
    path.last().map(AsRef::as_ref)
}

fn style_object_lookup_path(
    expression: &TemplateString,
    bindings: &[ScopedBinding],
) -> Option<Vec<CompactString>> {
    let Expr::Path(path) = expression.single_expression()? else {
        return None;
    };
    Some(resolve_scoped_path(path, bindings))
}

fn scoped_binding_for_control_flow(
    control_flow: &RenderControlFlow,
    bindings: &[ScopedBinding],
) -> Option<ScopedBinding> {
    if control_flow.kind != RenderControlFlowKind::For {
        return None;
    }
    let binding = control_flow.binding.as_ref()?;
    let Expr::Path(path) = control_flow.expression.as_ref()? else {
        return None;
    };

    Some(ScopedBinding {
        name: binding.name.clone(),
        path: resolve_scoped_path(path, bindings),
    })
}

fn resolve_scoped_path(path: &[CompactString], bindings: &[ScopedBinding]) -> Vec<CompactString> {
    let Some(first) = path.first() else {
        return Vec::new();
    };

    if let Some(binding) = bindings.iter().rev().find(|binding| binding.name == *first) {
        return binding
            .path
            .iter()
            .cloned()
            .chain(path.iter().skip(1).cloned())
            .collect();
    }

    path.to_vec()
}

fn style_declarations_from_script(
    declarations: &[ScriptStyleDeclaration],
) -> Vec<StyleDeclaration> {
    declarations
        .iter()
        .map(|declaration| {
            StyleDeclaration::new(
                declaration.name.clone(),
                declaration.value.clone(),
                false,
                declaration.span,
            )
        })
        .collect()
}

fn static_declarations_from_dynamic_style_value(
    source: &str,
    span: Option<Span>,
    runtime: &mut LowerRuntime,
) -> Vec<StyleDeclaration> {
    let mut declarations = Vec::new();
    for declaration in source.split(';').map(str::trim) {
        if declaration.is_empty() || declaration.contains("{{") || declaration.contains("}}") {
            continue;
        }

        let parsed_style = parse_css_style_attribute(&format!("{declaration};"), span);
        runtime.diagnostics.extend(parsed_style.diagnostics);
        let raw_flex_basis = static_flex_basis_from_declaration(declaration);
        let parsed_start = declarations.len();
        declarations.extend(parsed_style.value);
        if let Some(flex_basis) = raw_flex_basis
            && declarations[parsed_start..].iter().any(|style| {
                style.property == StyleProperty::Flex
                    && style.value.as_str().eq_ignore_ascii_case("none")
            })
            && !declarations[parsed_start..]
                .iter()
                .any(|style| style.property == StyleProperty::FlexBasis)
        {
            declarations.push(StyleDeclaration::new(
                StyleProperty::FlexBasis,
                flex_basis,
                false,
                span,
            ));
        }
    }

    declarations
}

fn static_flex_basis_from_declaration(declaration: &str) -> Option<&'static str> {
    let (property, value) = declaration.split_once(':')?;
    if !property.trim().eq_ignore_ascii_case("flex") {
        return None;
    }

    let value = value
        .split_once('!')
        .map_or(value, |(before_important, _)| before_important)
        .trim();
    let parts = value.split_whitespace().collect::<Vec<_>>();
    matches!(parts.as_slice(), ["0", "0", basis] if basis.eq_ignore_ascii_case("auto"))
        .then_some("auto")
}

fn parse_template(
    context: &LowerContext<'_>,
    source: &str,
    span: Option<Span>,
    runtime: &mut LowerRuntime,
) -> Option<TemplateString> {
    let frontend = context
        .frontend
        .expect("lowering context should have a frontend");
    let mut dialect_context = DialectContext::new(&mut runtime.diagnostics);
    frontend.parse_template(source, span, &mut dialect_context)
}

fn split_pseudo_elements(
    variants: Vec<RenderStyleVariant>,
) -> (Vec<RenderStyleVariant>, Vec<RenderPseudoElement>) {
    let mut remaining = Vec::new();
    let mut pseudo_elements = Vec::new();

    for variant in variants {
        if let Some(pseudo_element) = pseudo_element_from_variant(&variant) {
            pseudo_elements.push(pseudo_element);
        } else {
            remaining.push(variant);
        }
    }

    (remaining, pseudo_elements)
}

fn pseudo_element_from_variant(variant: &RenderStyleVariant) -> Option<RenderPseudoElement> {
    let kind = variant.conditions.iter().find_map(|condition| {
        if let RenderStyleCondition::PseudoElement(kind) = condition {
            Some(kind.clone())
        } else {
            None
        }
    })?;
    let mut styles = Vec::new();
    let mut children = Vec::new();

    for declaration in &variant.declarations {
        if declaration.property == StyleProperty::Content {
            if let Some(content) = pseudo_content_text(declaration.value.as_str()) {
                children.push(RenderNode::Text(RenderText {
                    value: content,
                    template: None,
                    span: declaration.span,
                }));
            }
        } else {
            styles.push(declaration.clone());
        }
    }

    Some(RenderPseudoElement {
        kind,
        selector: variant.selector.clone(),
        conditions: variant
            .conditions
            .iter()
            .filter(|condition| !matches!(condition, RenderStyleCondition::PseudoElement(_)))
            .cloned()
            .collect(),
        styles,
        children,
        span: variant.span,
    })
}

fn pseudo_content_text(value: &str) -> Option<String> {
    let value = value.trim();
    if matches!(value, "none" | "normal") {
        return None;
    }

    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .map(unescape_css_string)
}

fn unescape_css_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            output.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            output.push(ch);
        }
    }
    output
}

#[derive(Debug, Default)]
struct StatePlanner {
    next_state: usize,
    used_ids: BTreeSet<CompactString>,
}

impl StatePlanner {
    fn bind(&mut self, element: &HtmlElement, fallback_prefix: &str) -> CompactString {
        let seed = html_attribute_value(element, "data-htmlswap-state")
            .or_else(|| html_attribute_value(element, "id"))
            .or_else(|| html_attribute_value(element, "name"))
            .map(str::to_owned)
            .unwrap_or_else(|| {
                let id = format!("{fallback_prefix}_{}", self.next_state);
                self.next_state += 1;
                id
            });
        let base = sanitize_binding_id(&seed, fallback_prefix);

        if self.used_ids.insert(base.clone()) {
            return base;
        }

        let mut suffix = 2;
        loop {
            let candidate = CompactString::from(format!("{base}_{suffix}"));
            if self.used_ids.insert(candidate.clone()) {
                return candidate;
            }
            suffix += 1;
        }
    }
}

fn state_binding_for_control(
    element: &HtmlElement,
    tag: &str,
    children: &[RenderNode],
    context: &LowerContext<'_>,
    runtime: &mut LowerRuntime,
) -> Option<RenderStateBinding> {
    match control_type(element, tag) {
        RenderFormControlType::Text | RenderFormControlType::TextArea => {
            let input_type = html_attribute_value(element, "type")
                .map(str::trim)
                .map(str::to_ascii_lowercase)
                .unwrap_or_else(|| {
                    if tag == "textarea" {
                        "textarea".to_owned()
                    } else {
                        "text".to_owned()
                    }
                });
            let raw_initial_value = if tag == "textarea" {
                text_value(children).map(CompactString::from)
            } else {
                html_attribute_value(element, "value").map(CompactString::from)
            };
            let initial_template = raw_initial_value
                .as_deref()
                .and_then(|value| parse_template(context, value, element.span, runtime));
            let initial_value = if initial_template.is_some() {
                None
            } else {
                raw_initial_value
            };
            let raw_placeholder =
                html_attribute_value(element, "placeholder").map(CompactString::from);
            let placeholder_template = raw_placeholder
                .as_deref()
                .and_then(|value| parse_template(context, value, element.span, runtime));
            let placeholder = if placeholder_template.is_some() {
                None
            } else {
                raw_placeholder
            };
            let rows = html_attribute_value(element, "rows").and_then(parse_positive_u16);

            Some(RenderStateBinding {
                id: runtime.state_planner.bind(element, "input"),
                owner: state_owner_for_element(element, context, initial_template.is_some()),
                kind: RenderStateKind::TextInput(RenderTextInputState {
                    input_type: CompactString::from(input_type.as_str()),
                    initial_value,
                    initial_template,
                    placeholder,
                    placeholder_template,
                    multiline: tag == "textarea",
                    rows,
                    password: input_type == "password",
                    disabled: has_html_boolean_attribute(element, "disabled"),
                    readonly: has_html_boolean_attribute(element, "readonly"),
                }),
                span: element.span,
            })
        }
        RenderFormControlType::Select => {
            let options = choice_options(children);
            Some(RenderStateBinding {
                id: runtime.state_planner.bind(element, "select"),
                owner: state_owner_for_element(element, context, false),
                kind: RenderStateKind::Choice(RenderChoiceState {
                    selected_index: selected_option_index(children),
                    options,
                    multiple: has_html_boolean_attribute(element, "multiple"),
                    disabled: has_html_boolean_attribute(element, "disabled"),
                }),
                span: element.span,
            })
        }
        RenderFormControlType::Checkbox | RenderFormControlType::Radio => {
            Some(RenderStateBinding {
                id: runtime.state_planner.bind(element, "toggle"),
                owner: state_owner_for_element(element, context, false),
                kind: RenderStateKind::Toggle(RenderToggleState {
                    checked: has_html_boolean_attribute(element, "checked"),
                    disabled: has_html_boolean_attribute(element, "disabled"),
                }),
                span: element.span,
            })
        }
        _ => None,
    }
}

fn form_control_for_text_control(
    element: &HtmlElement,
    tag: &str,
    ancestors: &[StyleAncestor<'_>],
    siblings: &[HtmlNode],
) -> Option<RenderFormControl> {
    let control_type = control_type(element, tag);
    if !is_form_control_type(&control_type) {
        return None;
    }

    let id = html_attribute_value(element, "id");
    let label = html_attribute_value(element, "aria-label")
        .map(str::to_owned)
        .or_else(|| label_from_ancestor(element, ancestors))
        .or_else(|| id.and_then(|id| associated_label_text(id, siblings, ancestors)));

    Some(RenderFormControl {
        id: id.map(CompactString::from),
        control_type,
        name: html_attribute_value(element, "name").map(CompactString::from),
        value: control_value(element, tag),
        group: html_attribute_value(element, "name").map(CompactString::from),
        label: label
            .map(|label| label.trim().to_owned())
            .filter(|label| !label.is_empty())
            .map(CompactString::from),
        options: choice_options_from_html(element),
        required: has_html_boolean_attribute(element, "required"),
        disabled: has_html_boolean_attribute(element, "disabled"),
        readonly: has_html_boolean_attribute(element, "readonly"),
        validation: validation_for_element(element, tag),
        span: element.span,
    })
}

fn state_owner_for_element(
    element: &HtmlElement,
    context: &LowerContext<'_>,
    source_bound: bool,
) -> RenderStateOwner {
    match html_attribute_value(element, "data-htmlswap-state-owner")
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("source") => return RenderStateOwner::Source,
        Some("external") => return RenderStateOwner::External,
        Some("target") => return RenderStateOwner::Target,
        _ => {}
    }

    if html_attribute_value(element, "data-htmlswap-state").is_some() {
        return RenderStateOwner::Target;
    }

    if source_bound && frontend_has_dialect(context, "dc") {
        return RenderStateOwner::Source;
    }

    RenderStateOwner::Target
}

fn frontend_has_dialect(context: &LowerContext<'_>, name: &str) -> bool {
    context.frontend.is_some_and(|frontend| {
        frontend
            .dialects()
            .iter()
            .any(|dialect| dialect.name() == name)
    })
}

fn validation_for_element(element: &HtmlElement, tag: &str) -> RenderValidation {
    let mut validation = RenderValidation {
        required: has_html_boolean_attribute(element, "required"),
        disabled: has_html_boolean_attribute(element, "disabled"),
        readonly: has_html_boolean_attribute(element, "readonly"),
        constraints: Vec::new(),
    };

    if let Some(input_type) = (tag == "input")
        .then(|| html_attribute_value(element, "type"))
        .flatten()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        validation
            .constraints
            .push(RenderValidationConstraint::InputType(CompactString::from(
                input_type.to_ascii_lowercase(),
            )));
    }

    if let Some(value) = html_attribute_value(element, "minlength").and_then(parse_u32) {
        validation
            .constraints
            .push(RenderValidationConstraint::MinLength(value));
    }

    if let Some(value) = html_attribute_value(element, "maxlength").and_then(parse_u32) {
        validation
            .constraints
            .push(RenderValidationConstraint::MaxLength(value));
    }

    push_validation_attribute(
        element,
        "min",
        &mut validation.constraints,
        RenderValidationConstraint::Min,
    );
    push_validation_attribute(
        element,
        "max",
        &mut validation.constraints,
        RenderValidationConstraint::Max,
    );
    push_validation_attribute(
        element,
        "step",
        &mut validation.constraints,
        RenderValidationConstraint::Step,
    );
    push_validation_attribute(
        element,
        "pattern",
        &mut validation.constraints,
        RenderValidationConstraint::Pattern,
    );

    validation
}

fn push_validation_attribute(
    element: &HtmlElement,
    name: &str,
    constraints: &mut Vec<RenderValidationConstraint>,
    make_constraint: fn(CompactString) -> RenderValidationConstraint,
) {
    if let Some(value) = html_attribute_value(element, name)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        constraints.push(make_constraint(CompactString::from(value)));
    }
}

fn control_type(element: &HtmlElement, tag: &str) -> RenderFormControlType {
    match tag {
        "textarea" => RenderFormControlType::TextArea,
        "select" => RenderFormControlType::Select,
        "fieldset" => RenderFormControlType::Fieldset,
        "button" => match html_attribute_value(element, "type")
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("submit") => RenderFormControlType::Submit,
            Some("reset") => RenderFormControlType::Reset,
            _ => RenderFormControlType::Button,
        },
        "input" => {
            let input_type = html_attribute_value(element, "type")
                .map(str::trim)
                .map(str::to_ascii_lowercase)
                .unwrap_or_else(|| "text".to_owned());
            match input_type.as_str() {
                "text" | "search" | "email" | "password" | "url" | "tel" | "number" | "date"
                | "datetime-local" | "month" | "time" | "week" => RenderFormControlType::Text,
                "checkbox" => RenderFormControlType::Checkbox,
                "radio" => RenderFormControlType::Radio,
                "submit" => RenderFormControlType::Submit,
                "reset" => RenderFormControlType::Reset,
                "button" => RenderFormControlType::Button,
                _ => RenderFormControlType::Unknown(input_type.into()),
            }
        }
        _ => RenderFormControlType::Unknown(tag.into()),
    }
}

fn is_form_control_type(control_type: &RenderFormControlType) -> bool {
    !matches!(control_type, RenderFormControlType::Unknown(_))
}

fn control_value(element: &HtmlElement, tag: &str) -> Option<CompactString> {
    if tag == "textarea" {
        return None;
    }

    html_attribute_value(element, "value")
        .or_else(|| {
            matches!(
                control_type(element, tag),
                RenderFormControlType::Checkbox | RenderFormControlType::Radio
            )
            .then_some("on")
        })
        .map(CompactString::from)
}

fn choice_options(children: &[RenderNode]) -> Vec<RenderChoiceOption> {
    children
        .iter()
        .filter_map(render_option_element)
        .map(option_from_element)
        .collect()
}

fn selected_option_index(children: &[RenderNode]) -> Option<usize> {
    children
        .iter()
        .filter_map(render_option_element)
        .position(|option| {
            option
                .attributes
                .iter()
                .any(|attribute| attribute.name == "selected")
        })
}

fn choice_options_from_html(element: &HtmlElement) -> Vec<RenderChoiceOption> {
    element
        .children
        .iter()
        .filter_map(|node| {
            let HtmlNode::Element(option) = node else {
                return None;
            };

            if option.name.local().eq_ignore_ascii_case("option") {
                return Some(option_from_html_element(option));
            }

            if option.name.local().eq_ignore_ascii_case("optgroup") {
                return None;
            }

            None
        })
        .collect()
}

fn render_option_element(node: &RenderNode) -> Option<&RenderElement> {
    let RenderNode::Element(element) = node else {
        return None;
    };

    (element.source_tag == "option").then_some(element)
}

fn option_from_element(element: &RenderElement) -> RenderChoiceOption {
    let label = text_value(&element.children)
        .map(CompactString::from)
        .unwrap_or_default();
    let value = attribute_value_from_render(element, "value")
        .map(CompactString::from)
        .unwrap_or_else(|| label.clone());

    RenderChoiceOption {
        label,
        value,
        disabled: element
            .attributes
            .iter()
            .any(|attribute| attribute.name == "disabled"),
        span: element.span,
    }
}

fn option_from_html_element(element: &HtmlElement) -> RenderChoiceOption {
    let label = html_text_value(&element.children).unwrap_or_default();
    let value = html_attribute_value(element, "value")
        .map(CompactString::from)
        .unwrap_or_else(|| CompactString::from(label.as_str()));

    RenderChoiceOption {
        label: label.into(),
        value,
        disabled: has_html_boolean_attribute(element, "disabled"),
        span: element.span,
    }
}

fn attribute_value_from_render<'a>(element: &'a RenderElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .map(|attribute| attribute.value.as_str())
}

fn html_attribute_value<'a>(element: &'a HtmlElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local().eq_ignore_ascii_case(name))
        .map(|attribute| attribute.value.as_str())
}

fn html_attribute_span(element: &HtmlElement, name: &str) -> Option<Span> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local().eq_ignore_ascii_case(name))
        .and_then(|attribute| attribute.span)
}

fn script_reference_for_element(
    element: &HtmlElement,
    context: &LowerContext<'_>,
) -> Option<RenderScriptReference> {
    let src = html_attribute_value(element, "src")?.trim();
    if src.is_empty() {
        return None;
    }

    Some(RenderScriptReference {
        src: CompactString::from(src),
        kind: script_kind_for_element(element),
        async_script: has_html_boolean_attribute(element, "async"),
        defer: has_html_boolean_attribute(element, "defer"),
        span: html_attribute_span(element, "src").or(element.span),
        resolved_source: context
            .sources
            .and_then(|sources| script_source_id_for_src(sources, src)),
    })
}

fn source_logic_for_element(element: &HtmlElement) -> Option<RenderSourceLogic> {
    if !element.name.local().eq_ignore_ascii_case("script")
        || !has_html_boolean_attribute(element, "data-dc-script")
    {
        return None;
    }

    let body = element_text_content(element);
    if body.trim().is_empty() {
        return None;
    }

    Some(RenderSourceLogic {
        dialect: CompactString::from("dc"),
        script_type: html_attribute_value(element, "type").map(CompactString::from),
        body: body.into(),
        data_props: html_attribute_value(element, "data-props").map(CompactString::from),
        span: element.span,
    })
}

fn element_text_content(element: &HtmlElement) -> String {
    let mut output = String::new();
    for child in &element.children {
        match child {
            HtmlNode::Text(text) => output.push_str(&text.value),
            HtmlNode::Comment(comment) => output.push_str(&comment.value),
            HtmlNode::Element(_) => {}
        }
    }
    output
}

fn script_source_id_for_src(sources: &SourceMap, src: &str) -> Option<SourceId> {
    source_id_by_name(sources, SourceKind::JavaScript, src).or_else(|| {
        let html_name = sources.file(SourceId::primary())?.name()?;
        let resolved = resolved_resource_name(src, html_name)?;
        source_id_by_name(sources, SourceKind::JavaScript, &resolved)
    })
}

fn source_id_by_name(sources: &SourceMap, kind: SourceKind, name: &str) -> Option<SourceId> {
    sources
        .files()
        .iter()
        .find(|source| source.kind() == kind && source.name() == Some(name))
        .map(|source| source.id())
}

fn resolved_resource_name(src: &str, base_name: &str) -> Option<String> {
    if let Ok(url) = Url::parse(src) {
        return Some(url.to_string());
    }

    Url::parse(base_name)
        .ok()
        .filter(|base| matches!(base.scheme(), "http" | "https" | "file"))
        .and_then(|base| base.join(src).ok())
        .map(|url| url.to_string())
}

fn script_kind_for_element(element: &HtmlElement) -> RenderScriptKind {
    let Some(script_type) = html_attribute_value(element, "type") else {
        return RenderScriptKind::Classic;
    };
    match script_type.trim().to_ascii_lowercase().as_str() {
        "" | "text/javascript" | "application/javascript" => RenderScriptKind::Classic,
        "module" => RenderScriptKind::Module,
        _ => RenderScriptKind::Unknown,
    }
}

fn has_html_boolean_attribute(element: &HtmlElement, name: &str) -> bool {
    element
        .attributes
        .iter()
        .any(|attribute| attribute.name.local().eq_ignore_ascii_case(name))
}

fn text_value(children: &[RenderNode]) -> Option<String> {
    let mut value = String::new();
    for child in children {
        let RenderNode::Text(text) = child else {
            return None;
        };
        value.push_str(&text.value);
    }
    Some(value)
}

fn html_text_value(children: &[HtmlNode]) -> Option<String> {
    let mut value = String::new();
    for child in children {
        let HtmlNode::Text(text) = child else {
            return None;
        };
        value.push_str(&text.value);
    }
    Some(value.trim().to_owned()).filter(|value| !value.is_empty())
}

fn attach_action_payloads(
    actions: &mut [ActionBinding],
    state: Option<&RenderStateBinding>,
    children: &[RenderNode],
) {
    for action in actions {
        match action.event.as_str() {
            "input" | "change" => {
                if let Some(state) = state {
                    action.payload = ActionPayload::ElementState {
                        state_id: state.id.clone(),
                    };
                }
            }
            "submit" => {
                let controls = form_data_fields(children);
                if !controls.is_empty() {
                    action.payload = ActionPayload::FormData { controls };
                }
            }
            _ => {}
        }
    }
}

fn form_data_fields(nodes: &[RenderNode]) -> Vec<RenderFormDataField> {
    let mut fields = Vec::new();
    collect_form_data_fields(nodes, &mut fields);
    fields
}

fn collect_form_data_fields(nodes: &[RenderNode], fields: &mut Vec<RenderFormDataField>) {
    for node in nodes {
        let RenderNode::Element(element) = node else {
            continue;
        };

        if let Some(control) = &element.form_control
            && let Some(name) = &control.name
        {
            fields.push(RenderFormDataField {
                name: name.clone(),
                state_id: element.state.as_ref().map(|state| state.id.clone()),
                value: control.value.clone(),
                control_type: control.control_type.clone(),
            });
        }

        collect_form_data_fields(&element.children, fields);
    }
}

fn accessibility_for_element(element: &HtmlElement) -> Option<RenderAccessibility> {
    let role = html_attribute_value(element, "role").map(CompactString::from);
    let label = html_attribute_value(element, "aria-label").map(CompactString::from);
    let labelled_by = html_attribute_value(element, "aria-labelledby").map(CompactString::from);
    let described_by = html_attribute_value(element, "aria-describedby").map(CompactString::from);
    let tab_index = html_attribute_value(element, "tabindex")
        .and_then(|value| value.trim().parse::<isize>().ok());
    let autofocus = has_html_boolean_attribute(element, "autofocus");
    let hidden = has_html_boolean_attribute(element, "hidden")
        || html_attribute_value(element, "aria-hidden").is_some_and(|value| value == "true");
    let aria = element
        .attributes
        .iter()
        .filter(|attribute| attribute.name.local().starts_with("aria-"))
        .map(|attribute| RenderAttribute {
            name: CompactString::from(attribute.name.local().to_ascii_lowercase()),
            value: attribute.value.clone(),
            template: None,
            span: attribute.span,
        })
        .collect::<Vec<_>>();

    let accessibility = RenderAccessibility {
        role,
        label,
        labelled_by,
        described_by,
        tab_index,
        autofocus,
        hidden,
        aria,
    };

    (accessibility.role.is_some()
        || accessibility.label.is_some()
        || accessibility.labelled_by.is_some()
        || accessibility.described_by.is_some()
        || accessibility.tab_index.is_some()
        || accessibility.autofocus
        || accessibility.hidden
        || !accessibility.aria.is_empty())
    .then_some(accessibility)
}

fn source_intent_for_element(
    element: &HtmlElement,
    context: &LowerContext<'_>,
    runtime: &mut LowerRuntime,
) -> Option<RenderSourceIntent> {
    let component = html_attribute_value(element, "data-htmlswap-component")
        .map(ComponentId::new)
        .or_else(|| aria_component_for_element(element));
    let mut hints = RenderSourceIntent {
        key: html_attribute_value(element, "data-htmlswap-key").map(CompactString::from),
        state_id: html_attribute_value(element, "data-htmlswap-state").map(CompactString::from),
        component,
        component_source: html_attribute_value(element, "data-htmlswap-component-source")
            .map(CompactString::from),
        slot: html_attribute_value(element, "data-htmlswap-slot").map(SlotId::new),
        child_strategy: html_attribute_value(element, "data-htmlswap-children")
            .map(CompactString::from),
        props: Vec::new(),
    };

    for attribute in &element.attributes {
        let Some(prop) = attribute.name.local().strip_prefix("data-htmlswap-prop-") else {
            continue;
        };

        hints.props.push(RenderSourceProp {
            name: CompactString::from(prop),
            value: attribute.value.clone(),
            template: parse_template(context, &attribute.value, attribute.span, runtime),
            span: attribute.span,
        });
    }

    (!hints.is_empty()).then_some(hints)
}

fn aria_component_for_element(element: &HtmlElement) -> Option<ComponentId> {
    let role = html_attribute_value(element, "role")?
        .trim()
        .to_ascii_lowercase();
    match role.as_str() {
        "tablist" => Some(ComponentId::new("tabs")),
        "tab" => Some(ComponentId::new("tab")),
        _ => None,
    }
}

fn semantics_for_element(element: &HtmlElement) -> Option<RenderSemantics> {
    let mut semantics = RenderSemantics {
        variant: html_attribute_value(element, "data-htmlswap-variant").map(RenderVariant::parse),
        tone: html_attribute_value(element, "data-htmlswap-tone").map(RenderTone::parse),
        size: html_attribute_value(element, "data-htmlswap-size").map(RenderSize::parse),
        density: html_attribute_value(element, "data-htmlswap-density").map(RenderDensity::parse),
        extras: Vec::new(),
    };

    for attribute in &element.attributes {
        let Some(axis) = attribute
            .name
            .local()
            .strip_prefix("data-htmlswap-semantic-")
        else {
            continue;
        };
        let axis = axis.trim();
        if axis.is_empty() {
            continue;
        }
        semantics.extras.push(RenderSemanticExtra {
            axis: CompactString::from(axis),
            value: attribute.value.clone(),
            span: attribute.span,
        });
    }

    (!semantics.is_empty()).then_some(semantics)
}

fn region_for_element(element: &HtmlElement, diagnostics: &mut Diagnostics) -> Option<RegionId> {
    let attribute = element.attributes.iter().find(|attribute| {
        attribute
            .name
            .local()
            .eq_ignore_ascii_case("data-htmlswap-region")
    })?;

    let Some(region) = RegionId::parse(attribute.value.as_str()) else {
        diagnostics.push(Diagnostic::warning(
            format!(
                "source region `{}` must be a dotted semantic namespace, not a path or module name",
                attribute.value.trim()
            ),
            attribute.span.or(element.span),
        ));
        return None;
    };

    Some(region)
}

fn label_from_ancestor(element: &HtmlElement, ancestors: &[StyleAncestor<'_>]) -> Option<String> {
    let label = ancestors
        .iter()
        .rev()
        .find(|ancestor| ancestor.element.name.local().eq_ignore_ascii_case("label"))?
        .element;

    label_text_excluding_element(label, element)
}

fn associated_label_text(
    id: &str,
    siblings: &[HtmlNode],
    ancestors: &[StyleAncestor<'_>],
) -> Option<String> {
    find_label_for(id, siblings)
        .or_else(|| {
            ancestors
                .iter()
                .rev()
                .find_map(|ancestor| find_label_for(id, ancestor.siblings))
        })
        .and_then(label_text)
}

fn find_label_for<'a>(id: &str, nodes: &'a [HtmlNode]) -> Option<&'a HtmlElement> {
    for node in nodes {
        let HtmlNode::Element(element) = node else {
            continue;
        };

        if element.name.local().eq_ignore_ascii_case("label")
            && html_attribute_value(element, "for").is_some_and(|value| value == id)
        {
            return Some(element);
        }

        if let Some(label) = find_label_for(id, &element.children) {
            return Some(label);
        }
    }

    None
}

fn label_text(element: &HtmlElement) -> Option<String> {
    let mut text = String::new();
    collect_label_text(&element.children, None, &mut text);
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then_some(text)
}

fn label_text_excluding_element(label: &HtmlElement, excluded: &HtmlElement) -> Option<String> {
    let mut text = String::new();
    collect_label_text(&label.children, excluded.span, &mut text);
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then_some(text)
}

fn collect_label_text(nodes: &[HtmlNode], excluded_span: Option<Span>, output: &mut String) {
    for node in nodes {
        match node {
            HtmlNode::Text(text) => output.push_str(&text.value),
            HtmlNode::Comment(_) => {}
            HtmlNode::Element(element) => {
                if element.span.is_some() && element.span == excluded_span {
                    continue;
                }

                collect_label_text(&element.children, excluded_span, output);
            }
        }
    }
}

fn parse_positive_u16(value: &str) -> Option<u16> {
    value.trim().parse::<u16>().ok().filter(|value| *value > 0)
}

fn parse_u32(value: &str) -> Option<u32> {
    value.trim().parse::<u32>().ok()
}

fn sanitize_binding_id(value: &str, fallback_prefix: &str) -> CompactString {
    let mut output = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
        } else if !output.ends_with('_') {
            output.push('_');
        }
    }

    let output = output.trim_matches('_');
    if output.is_empty() || output.as_bytes()[0].is_ascii_digit() {
        CompactString::from(format!("{fallback_prefix}_{output}").trim_end_matches('_'))
    } else {
        CompactString::from(output)
    }
}

fn lower_action_binding(
    attribute_name: &str,
    expression: &str,
    span: Option<Span>,
    context: &LowerContext<'_>,
    runtime: &mut LowerRuntime,
) -> ActionBinding {
    let event = CompactString::from(attribute_name.strip_prefix("on").unwrap_or(attribute_name));

    if let Some(template) = parse_template(context, expression, span, runtime) {
        return ActionBinding {
            event,
            expression: expression.to_owned(),
            template: Some(template),
            action: None,
            resolved: false,
            handler: RenderActionHandler::default(),
            payload: ActionPayload::None,
            span,
            action_span: None,
        };
    }

    let handler = analyze_event_handler(expression, span, context.actions);
    let primary_invocation = handler.primary_invocation();
    let action = primary_invocation.map(|invocation| invocation.action.clone());
    let resolved = primary_invocation.is_some_and(|invocation| invocation.resolved);
    let action_span = primary_invocation.and_then(|invocation| invocation.action_span);

    ActionBinding {
        event,
        expression: expression.to_owned(),
        template: None,
        action,
        resolved,
        handler,
        payload: ActionPayload::None,
        span,
        action_span,
    }
}

fn role_for_tag(tag: &str) -> UiRole {
    match tag {
        "html" | "body" | "main" | "div" | "section" | "article" | "aside" | "header"
        | "footer" | "nav" => UiRole::Container,
        "span" | "strong" | "b" | "em" | "i" | "small" | "code" => UiRole::Inline,
        "p" => UiRole::Paragraph,
        "button" => UiRole::Button,
        "input" | "textarea" => UiRole::TextInput,
        "select" => UiRole::Select,
        "option" => UiRole::Option,
        "a" => UiRole::Link,
        "img" => UiRole::Image,
        "h1" => UiRole::Heading(1),
        "h2" => UiRole::Heading(2),
        "h3" => UiRole::Heading(3),
        "h4" => UiRole::Heading(4),
        "h5" => UiRole::Heading(5),
        "h6" => UiRole::Heading(6),
        "ul" => UiRole::List { ordered: false },
        "ol" => UiRole::List { ordered: true },
        "li" => UiRole::ListItem,
        "form" => UiRole::Form,
        "fieldset" => UiRole::Fieldset,
        "legend" => UiRole::Legend,
        "label" => UiRole::Label,
        _ => UiRole::Unknown,
    }
}
