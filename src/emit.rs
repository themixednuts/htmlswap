use std::collections::BTreeSet;
use std::fmt::Write;

use crate::adapter::{GeneratedFile, HtmlArtifact};
use crate::assets::CompileAssets;
use crate::diagnostics::Diagnostics;
use crate::expr::{TemplateSegment, TemplateString};
use crate::plan::{
    ActionBinding, RenderAnnotation, RenderDensity, RenderElement, RenderHeadElement, RenderNode,
    RenderPlan, RenderRaw, RenderSemantics, RenderSize, RenderSourceIntent, RenderStateOwner,
    RenderStyleCondition, RenderStyleVariant, RenderTone, RenderVariant, StyleDeclaration,
};

#[derive(Debug, Default)]
pub struct EmitContext {
    diagnostics: Diagnostics,
}

impl EmitContext {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    #[must_use]
    pub fn into_diagnostics(self) -> Diagnostics {
        self.diagnostics
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EmitError {
    #[error("{message}")]
    Message { message: String },
}

pub trait Emitter {
    type Output;

    fn emit(&self, plan: &RenderPlan, cx: &mut EmitContext) -> Result<Self::Output, EmitError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TextEmitter;

impl Emitter for TextEmitter {
    type Output = String;

    fn emit(&self, plan: &RenderPlan, _cx: &mut EmitContext) -> Result<Self::Output, EmitError> {
        let mut output = String::new();
        write_annotations(&mut output, &plan.annotations, 0);
        write_nodes(&mut output, &plan.nodes, 0);
        Ok(output)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct HtmlEmitter;

impl Emitter for HtmlEmitter {
    type Output = HtmlArtifact;

    fn emit(&self, plan: &RenderPlan, _cx: &mut EmitContext) -> Result<Self::Output, EmitError> {
        let mut output = String::new();
        write_html_annotations(&mut output, &plan.annotations);
        let pseudo_rules = if head_contains_style(&plan.head) {
            BTreeSet::new()
        } else {
            collect_html_pseudo_style_rules(&plan.nodes)
        };
        if let Some(nodes) = nodes_with_dc_helmet_head(&plan.nodes, &plan.head, &pseudo_rules) {
            write_html_nodes(&mut output, &nodes, 0);
        } else {
            write_html_head_elements(&mut output, &plan.head);
            write_html_pseudo_styles(&mut output, &pseudo_rules);
            write_html_nodes(&mut output, &plan.nodes, 0);
        }
        Ok(HtmlArtifact::new(
            GeneratedFile::new("index.html", output),
            CompileAssets::new(),
        ))
    }
}

fn write_html_annotations(output: &mut String, annotations: &[RenderAnnotation]) {
    for annotation in annotations {
        if matches!(
            annotation.kind,
            crate::plan::RenderAnnotationKind::HtmlComment
        ) {
            output.push_str("<!--");
            output.push_str(&annotation.value);
            output.push_str("-->\n");
        }
    }
}

fn collect_html_pseudo_style_rules(nodes: &[RenderNode]) -> BTreeSet<String> {
    let mut rules = BTreeSet::new();
    collect_html_pseudo_style_rules_from_nodes(nodes, &mut rules);
    rules
}

fn write_html_pseudo_styles(output: &mut String, rules: &BTreeSet<String>) {
    if rules.is_empty() {
        return;
    }

    output.push_str(&format_html_pseudo_style_block(rules));
}

fn write_html_head_elements(output: &mut String, head: &[RenderHeadElement]) {
    for element in head {
        output.push_str(element.html.trim());
        output.push('\n');
    }
}

fn head_contains_style(head: &[RenderHeadElement]) -> bool {
    head.iter()
        .any(|element| element.html.trim_start().starts_with("<style"))
}

fn collect_html_pseudo_style_rules_from_nodes(nodes: &[RenderNode], rules: &mut BTreeSet<String>) {
    for node in nodes {
        let RenderNode::Element(element) = node else {
            continue;
        };
        for pseudo in &element.pseudo_elements {
            if let Some(rule) = html_pseudo_style_rule(pseudo) {
                rules.insert(rule);
            }
        }
        collect_html_pseudo_style_rules_from_nodes(&element.children, rules);
    }
}

fn nodes_with_dc_helmet_head(
    nodes: &[RenderNode],
    head: &[RenderHeadElement],
    rules: &BTreeSet<String>,
) -> Option<Vec<RenderNode>> {
    if head.is_empty() && rules.is_empty() {
        return None;
    }

    let mut nodes = nodes.to_vec();
    insert_dc_helmet_head(&mut nodes, head, rules).then_some(nodes)
}

fn insert_dc_helmet_head(
    nodes: &mut [RenderNode],
    head: &[RenderHeadElement],
    rules: &BTreeSet<String>,
) -> bool {
    for node in nodes {
        let RenderNode::Element(element) = node else {
            continue;
        };
        if html_tag_for_element(element) == "x-dc" {
            insert_helmet_head_child(element, head, rules);
            return true;
        }
        if insert_dc_helmet_head(&mut element.children, head, rules) {
            return true;
        }
    }
    false
}

fn insert_helmet_head_child(
    element: &mut RenderElement,
    head: &[RenderHeadElement],
    rules: &BTreeSet<String>,
) {
    let mut html = String::from("<helmet>\n");
    for element in head {
        html.push_str(element.html.trim());
        html.push('\n');
    }
    if !rules.is_empty() {
        html.push_str(&format_html_pseudo_style_block(rules));
    }
    html.push_str("</helmet>");

    let raw = RenderNode::Raw(RenderRaw { html, span: None });
    element.children.insert(0, raw);
}

fn format_html_pseudo_style_block(rules: &BTreeSet<String>) -> String {
    let mut output = String::from("<style>\n");
    for rule in rules {
        output.push_str("  ");
        output.push_str(rule);
        output.push('\n');
    }
    output.push_str("</style>\n");
    output
}

fn html_pseudo_style_rule(pseudo: &crate::plan::RenderPseudoElement) -> Option<String> {
    let selector = pseudo.selector.trim();
    if selector.is_empty() {
        return None;
    }
    if pseudo.conditions.is_empty() && matches!(pseudo.kind.as_str(), "before" | "after") {
        return None;
    }

    Some(format!(
        "{} {{{}}}",
        selector,
        format_html_styles(&pseudo.styles)
    ))
}

fn write_html_nodes(output: &mut String, nodes: &[RenderNode], depth: usize) {
    for node in nodes {
        match node {
            RenderNode::Element(element) => write_html_element(output, element, depth),
            RenderNode::Text(text) => {
                write_indent(output, depth);
                escape_html_text(&text.value, output);
                output.push('\n');
            }
            RenderNode::Raw(raw) => write_html_raw(output, raw, depth),
        }
    }
}

pub(crate) fn render_html_fragment(nodes: &[RenderNode]) -> String {
    let mut output = String::new();
    write_html_nodes(&mut output, nodes, 0);
    output
}

fn write_html_raw(output: &mut String, raw: &RenderRaw, depth: usize) {
    write_indent(output, depth);
    output.push_str(&raw.html);
    output.push('\n');
}

fn write_html_element(output: &mut String, element: &RenderElement, depth: usize) {
    let tag = html_tag_for_element(element);
    write_indent(output, depth);
    output.push('<');
    output.push_str(tag);
    write_html_attributes(output, element);

    if is_void_html_tag(tag) {
        output.push_str(">\n");
        return;
    }

    if element.children.is_empty() {
        output.push_str("></");
        output.push_str(tag);
        output.push_str(">\n");
        return;
    }

    output.push_str(">\n");
    write_html_nodes(output, &element.children, depth + 1);
    write_indent(output, depth);
    output.push_str("</");
    output.push_str(tag);
    output.push_str(">\n");
}

fn write_html_attributes(output: &mut String, element: &RenderElement) {
    if !element.classes.is_empty() {
        output.push_str(" class=\"");
        escape_html_attribute(
            &element
                .classes
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>()
                .join(" "),
            output,
        );
        output.push('"');
    }

    for attribute in &element.attributes {
        if attribute.name == "class" || attribute.name == "style" {
            continue;
        }
        write_html_attribute(output, attribute.name.as_str(), attribute.value.as_str());
    }

    write_source_metadata_attributes(output, element);

    if let Some(style) = html_style_attribute_value(element) {
        write_html_attribute(output, "style", &style);
    }
    write_html_style_variant_attributes(output, element);
    write_html_dynamic_style_attributes(output, element);

    for action in &element.actions {
        output.push(' ');
        output.push_str("on");
        output.push_str(action.event.as_str());
        output.push_str("=\"");
        let target = action.action.as_deref().unwrap_or(&action.expression);
        if action.resolved {
            escape_html_attribute(&format!("{{{{ {target} }}}}"), output);
        } else {
            escape_html_attribute(target, output);
        }
        output.push('"');
    }
}

fn html_style_attribute_value(element: &RenderElement) -> Option<String> {
    let mut parts = Vec::new();
    if !element.source_inline_styles.is_empty() {
        parts.push(format_html_styles(&element.source_inline_styles));
    } else if !element.styles.is_empty() {
        parts.push(format_html_styles(&element.styles));
    }
    if element.source_inline_styles.is_empty() && element.styles.is_empty() {
        parts.extend(
            element
                .dynamic_styles
                .iter()
                .filter(|binding| binding.state.is_none())
                .map(|binding| format_template_string(&binding.expression)),
        );
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

fn write_html_style_variant_attributes(output: &mut String, element: &RenderElement) {
    for variant in &element.style_variants {
        let Some(name) = style_variant_attribute_name(variant) else {
            continue;
        };
        write_html_attribute(output, &name, &format_html_styles(&variant.declarations));
    }

    for pseudo in &element.pseudo_elements {
        if !pseudo.conditions.is_empty() {
            continue;
        }
        let kind = pseudo.kind.as_str();
        if !matches!(kind, "before" | "after") {
            continue;
        }
        let mut styles = pseudo.styles.clone();
        if let Some(content) = pseudo.children.iter().find_map(|child| match child {
            RenderNode::Text(text) => Some(text.value.as_str()),
            _ => None,
        }) {
            styles.push(StyleDeclaration::new(
                "content",
                format!("{content:?}"),
                false,
                None,
            ));
        }
        write_html_attribute(
            output,
            &format!("style-{kind}"),
            &format_html_styles(&styles),
        );
    }
}

fn style_variant_attribute_name(variant: &RenderStyleVariant) -> Option<String> {
    let [RenderStyleCondition::PseudoClass(state)] = variant.conditions.as_slice() else {
        return None;
    };
    Some(format!("style-{state}"))
}

fn write_html_dynamic_style_attributes(output: &mut String, element: &RenderElement) {
    for binding in &element.dynamic_styles {
        let value = format_template_string(&binding.expression);
        match &binding.state {
            Some(state) => {
                write_html_attribute(output, &format!("data-htmlswap-style-{state}"), &value)
            }
            None if !element.styles.is_empty() => {
                write_html_attribute(output, "data-htmlswap-style", &value);
            }
            None => {}
        };
    }
}

fn format_template_string(template: &TemplateString) -> String {
    if !template.raw.is_empty() {
        return template.raw.to_string();
    }
    template
        .segments
        .iter()
        .map(|segment| match segment {
            TemplateSegment::Literal(value) => value.to_string(),
            TemplateSegment::Expression(expression) => format!("{{{{ {expression} }}}}"),
        })
        .collect::<Vec<_>>()
        .join("")
}

fn write_source_metadata_attributes(output: &mut String, element: &RenderElement) {
    if let Some(semantics) = &element.semantics {
        write_semantic_attributes(output, element, semantics);
    }

    if let Some(intent) = &element.source_intent {
        write_source_intent_attributes(output, element, intent);
    }

    if let Some(state) = &element.state
        && state.owner != RenderStateOwner::Target
    {
        let owner = match state.owner {
            RenderStateOwner::Target => "target",
            RenderStateOwner::Source => "source",
            RenderStateOwner::External => "external",
        };
        write_html_attribute_if_absent(output, element, "data-htmlswap-state-owner", owner);
    }
}

fn write_semantic_attributes(
    output: &mut String,
    element: &RenderElement,
    semantics: &RenderSemantics,
) {
    if let Some(value) = semantics.variant.as_ref().map(RenderVariant::as_str) {
        write_html_attribute_if_absent(output, element, "data-htmlswap-variant", value);
    }
    if let Some(value) = semantics.tone.as_ref().map(RenderTone::as_str) {
        write_html_attribute_if_absent(output, element, "data-htmlswap-tone", value);
    }
    if let Some(value) = semantics.size.as_ref().map(RenderSize::as_str) {
        write_html_attribute_if_absent(output, element, "data-htmlswap-size", value);
    }
    if let Some(value) = semantics.density.as_ref().map(RenderDensity::as_str) {
        write_html_attribute_if_absent(output, element, "data-htmlswap-density", value);
    }
    for extra in &semantics.extras {
        let name = format!("data-htmlswap-semantic-{}", extra.axis);
        write_html_attribute_if_absent(output, element, &name, extra.value.as_str());
    }
}

fn write_source_intent_attributes(
    output: &mut String,
    element: &RenderElement,
    intent: &RenderSourceIntent,
) {
    if let Some(key) = &intent.key {
        write_html_attribute_if_absent(output, element, "data-htmlswap-key", key.as_str());
    }
    if let Some(state_id) = &intent.state_id {
        write_html_attribute_if_absent(output, element, "data-htmlswap-state", state_id.as_str());
    }
    if let Some(component) = &intent.component {
        write_html_attribute_if_absent(
            output,
            element,
            "data-htmlswap-component",
            component.as_str(),
        );
    }
    if let Some(source) = &intent.component_source {
        write_html_attribute_if_absent(
            output,
            element,
            "data-htmlswap-component-source",
            source.as_str(),
        );
    }
    if let Some(slot) = &intent.slot {
        write_html_attribute_if_absent(output, element, "data-htmlswap-slot", slot.as_str());
    }
    if let Some(child_strategy) = &intent.child_strategy {
        write_html_attribute_if_absent(
            output,
            element,
            "data-htmlswap-children",
            child_strategy.as_str(),
        );
    }
    for prop in &intent.props {
        let name = format!("data-htmlswap-prop-{}", prop.name);
        write_html_attribute_if_absent(output, element, &name, prop.value.as_str());
    }
}

fn write_html_attribute_if_absent(
    output: &mut String,
    element: &RenderElement,
    name: &str,
    value: &str,
) {
    if element
        .attributes
        .iter()
        .any(|attribute| attribute.name.as_str().eq_ignore_ascii_case(name))
    {
        return;
    }
    write_html_attribute(output, name, value);
}

fn write_html_attribute(output: &mut String, name: &str, value: &str) {
    output.push(' ');
    output.push_str(name);
    if !value.is_empty() {
        output.push_str("=\"");
        escape_html_attribute(value, output);
        output.push('"');
    }
}

fn format_html_styles(styles: &[StyleDeclaration]) -> String {
    styles
        .iter()
        .map(|style| {
            if style.important {
                format!("{}: {} !important", style.property, style.value.as_str())
            } else {
                format!("{}: {}", style.property, style.value.as_str())
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn html_tag_for_element(element: &RenderElement) -> &str {
    if !element.source_tag.is_empty() {
        return element.source_tag.as_str();
    }

    match element.role {
        crate::plan::UiRole::Container => "div",
        crate::plan::UiRole::Inline => "span",
        crate::plan::UiRole::Paragraph => "p",
        crate::plan::UiRole::Button => "button",
        crate::plan::UiRole::TextInput => "input",
        crate::plan::UiRole::Select => "select",
        crate::plan::UiRole::Option => "option",
        crate::plan::UiRole::Link => "a",
        crate::plan::UiRole::Image => "img",
        crate::plan::UiRole::Heading(level) => match level {
            1 => "h1",
            2 => "h2",
            3 => "h3",
            4 => "h4",
            5 => "h5",
            6 => "h6",
            _ => "h6",
        },
        crate::plan::UiRole::List { ordered: false } => "ul",
        crate::plan::UiRole::List { ordered: true } => "ol",
        crate::plan::UiRole::ListItem => "li",
        crate::plan::UiRole::Form => "form",
        crate::plan::UiRole::Fieldset => "fieldset",
        crate::plan::UiRole::Legend => "legend",
        crate::plan::UiRole::Label => "label",
        crate::plan::UiRole::Unknown => "div",
    }
}

fn is_void_html_tag(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn write_indent(output: &mut String, depth: usize) {
    for _ in 0..depth {
        output.push_str("  ");
    }
}

fn escape_html_text(value: &str, output: &mut String) {
    for ch in value.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            _ => output.push(ch),
        }
    }
}

fn escape_html_attribute(value: &str, output: &mut String) {
    for ch in value.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '"' => output.push_str("&quot;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            _ => output.push(ch),
        }
    }
}

fn write_annotations(output: &mut String, annotations: &[RenderAnnotation], depth: usize) {
    let indent = "  ".repeat(depth);
    for annotation in annotations {
        writeln!(
            output,
            "{indent}annotation {} {:?}",
            annotation.kind.name(),
            annotation.value
        )
        .expect("writing to String cannot fail");
    }
}

fn write_nodes(output: &mut String, nodes: &[RenderNode], depth: usize) {
    for node in nodes {
        match node {
            RenderNode::Element(element) => write_element(output, element, depth),
            RenderNode::Text(text) => {
                let indent = "  ".repeat(depth);
                writeln!(output, "{indent}text {:?}", text.value)
                    .expect("writing to String cannot fail");
            }
            RenderNode::Raw(raw) => write_raw(output, raw, depth),
        }
    }
}

fn write_raw(output: &mut String, raw: &RenderRaw, depth: usize) {
    let indent = "  ".repeat(depth);
    writeln!(output, "{indent}raw {:?}", raw.html).expect("writing to String cannot fail");
}

fn write_element(output: &mut String, element: &RenderElement, depth: usize) {
    let indent = "  ".repeat(depth);
    let suffix = element_suffix(element);
    writeln!(
        output,
        "{indent}{} <{}>{suffix}",
        element.role.name(),
        element.source_tag
    )
    .expect("writing to String cannot fail");
    write_nodes(output, &element.children, depth + 1);
}

fn element_suffix(element: &RenderElement) -> String {
    let mut suffix = String::new();

    if !element.classes.is_empty() {
        suffix.push_str(" .");
        suffix.push_str(
            &element
                .classes
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<&str>>()
                .join("."),
        );
    }

    if !element.attributes.is_empty() {
        let attributes = element
            .attributes
            .iter()
            .map(|attribute| {
                if attribute.value.is_empty() {
                    attribute.name.to_string()
                } else {
                    format!("{}={:?}", attribute.name, attribute.value)
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        suffix.push_str(" [");
        suffix.push_str(&attributes);
        suffix.push(']');
    }

    if !element.styles.is_empty() {
        suffix.push_str(" {");
        suffix.push_str(&format_styles(&element.styles));
        suffix.push('}');
    }

    if !element.actions.is_empty() {
        suffix.push(' ');
        suffix.push_str(&format_actions(&element.actions));
    }

    suffix
}

fn format_styles(styles: &[StyleDeclaration]) -> String {
    styles
        .iter()
        .map(|style| {
            if style.important {
                format!("{}: {} !important", style.property, style.value)
            } else {
                format!("{}: {}", style.property, style.value)
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn format_actions(actions: &[ActionBinding]) -> String {
    actions
        .iter()
        .map(|action| {
            let target = action.action.as_deref().unwrap_or(&action.expression);
            if action.resolved {
                format!("@{}={target}", action.event)
            } else {
                format!("@{}={target}?", action.event)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
