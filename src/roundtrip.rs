use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::expr::{Expr, TemplateSegment, TemplateString};
use crate::plan::{
    ActionBinding, ActionPayload, RenderAccessibility, RenderActionHandlerEffect,
    RenderControlFlow, RenderElement, RenderFormControl, RenderNode, RenderPlan, RenderSemantics,
    RenderSourceIntent, RenderStateBinding, RenderStateKind, RenderStyleCondition,
    RenderStyleVariant, RenderTextInputState, UiRole,
};
use crate::style::{StyleDeclaration, StyleProperty};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoundTripOptions {
    pub max_mismatches: usize,
}

impl RoundTripOptions {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_mismatches: usize::MAX,
        }
    }

    #[must_use]
    pub const fn with_max_mismatches(mut self, max_mismatches: usize) -> Self {
        self.max_mismatches = max_mismatches;
        self
    }
}

impl Default for RoundTripOptions {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundTripComparison {
    mismatches: Vec<RoundTripMismatch>,
    omitted_mismatches: usize,
}

impl RoundTripComparison {
    #[must_use]
    pub fn new(mismatches: Vec<RoundTripMismatch>, omitted_mismatches: usize) -> Self {
        Self {
            mismatches,
            omitted_mismatches,
        }
    }

    #[must_use]
    pub fn is_match(&self) -> bool {
        self.mismatches.is_empty() && self.omitted_mismatches == 0
    }

    #[must_use]
    pub fn mismatches(&self) -> &[RoundTripMismatch] {
        &self.mismatches
    }

    #[must_use]
    pub const fn omitted_mismatches(&self) -> usize {
        self.omitted_mismatches
    }

    #[must_use]
    pub fn total_mismatches(&self) -> usize {
        self.mismatches.len() + self.omitted_mismatches
    }
}

impl fmt::Display for RoundTripComparison {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let total = self.total_mismatches();
        if total == 0 {
            return formatter.write_str("strict roundtrip tree and semantic features match");
        }

        writeln!(
            formatter,
            "{total} strict roundtrip tree/semantic mismatch(es)"
        )?;
        for mismatch in &self.mismatches {
            writeln!(formatter, "  - {mismatch}")?;
        }
        if self.omitted_mismatches > 0 {
            writeln!(
                formatter,
                "  - ... {} more mismatch(es) omitted",
                self.omitted_mismatches
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundTripMismatch {
    pub feature_kind: RoundTripFeatureKind,
    pub kind: RoundTripMismatchKind,
    pub feature: String,
    pub expected_count: usize,
    pub actual_count: usize,
}

impl fmt::Display for RoundTripMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            RoundTripMismatchKind::Missing => write!(
                formatter,
                "missing {} `{}` (expected {}, actual 0)",
                self.feature_kind.as_str(),
                self.feature,
                self.expected_count
            ),
            RoundTripMismatchKind::Extra => write!(
                formatter,
                "extra {} `{}` (expected 0, actual {})",
                self.feature_kind.as_str(),
                self.feature,
                self.actual_count
            ),
            RoundTripMismatchKind::Count => write!(
                formatter,
                "{} count differs for `{}` (expected {}, actual {})",
                self.feature_kind.as_str(),
                self.feature,
                self.expected_count,
                self.actual_count
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundTripMismatchKind {
    Missing,
    Extra,
    Count,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RoundTripFeatureKind {
    Element,
    Text,
    Raw,
    Attribute,
    Class,
    Style,
    StyleVariant,
    DynamicStyle,
    PseudoElement,
    Action,
    ControlFlow,
    State,
    FormControl,
    Accessibility,
    SourceIntent,
    Semantics,
    Region,
}

impl RoundTripFeatureKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Element => "element",
            Self::Text => "text",
            Self::Raw => "raw html",
            Self::Attribute => "attribute",
            Self::Class => "class",
            Self::Style => "style",
            Self::StyleVariant => "style variant",
            Self::DynamicStyle => "dynamic style",
            Self::PseudoElement => "pseudo element",
            Self::Action => "action",
            Self::ControlFlow => "control flow",
            Self::State => "state",
            Self::FormControl => "form control",
            Self::Accessibility => "accessibility",
            Self::SourceIntent => "source intent",
            Self::Semantics => "semantics",
            Self::Region => "region",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Feature {
    kind: RoundTripFeatureKind,
    value: String,
}

impl Feature {
    fn new(kind: RoundTripFeatureKind, value: impl Into<String>) -> Self {
        Self {
            kind,
            value: value.into(),
        }
    }
}

type FeatureCounts = BTreeMap<Feature, usize>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextTransform {
    None,
    Uppercase,
    Lowercase,
}

#[must_use]
pub fn compare_roundtrip_plans(
    expected: &RenderPlan,
    actual: &RenderPlan,
    options: RoundTripOptions,
) -> RoundTripComparison {
    let expected = collect_features(expected);
    let actual = collect_features(actual);
    compare_feature_counts(&expected, &actual, options.max_mismatches)
}

fn compare_feature_counts(
    expected: &FeatureCounts,
    actual: &FeatureCounts,
    max_mismatches: usize,
) -> RoundTripComparison {
    let mut keys = BTreeSet::new();
    keys.extend(expected.keys().cloned());
    keys.extend(actual.keys().cloned());

    let mut mismatches = Vec::new();
    let mut omitted = 0usize;
    for feature in keys {
        let expected_count = expected.get(&feature).copied().unwrap_or_default();
        let actual_count = actual.get(&feature).copied().unwrap_or_default();
        if expected_count == actual_count {
            continue;
        }

        let kind = if expected_count == 0 {
            RoundTripMismatchKind::Extra
        } else if actual_count == 0 {
            RoundTripMismatchKind::Missing
        } else {
            RoundTripMismatchKind::Count
        };

        let mismatch = RoundTripMismatch {
            feature_kind: feature.kind,
            kind,
            feature: feature.value,
            expected_count,
            actual_count,
        };
        if mismatches.len() < max_mismatches {
            mismatches.push(mismatch);
        } else {
            omitted += 1;
        }
    }

    RoundTripComparison::new(mismatches, omitted)
}

fn collect_features(plan: &RenderPlan) -> FeatureCounts {
    let mut features = FeatureCounts::new();
    for node in visible_root_nodes(&plan.nodes) {
        collect_node_features(node, "$", &mut features, TextTransform::None);
    }
    features
}

fn visible_root_nodes(nodes: &[RenderNode]) -> &[RenderNode] {
    let mut current = nodes;
    loop {
        let [RenderNode::Element(element)] = current else {
            return current;
        };
        if !is_transparent_document_wrapper(element) {
            return current;
        }
        current = &element.children;
    }
}

fn is_transparent_document_wrapper(element: &RenderElement) -> bool {
    if element.children.len() != 1
        || !element.actions.is_empty()
        || element.control_flow.is_some()
        || element.state.is_some()
        || element.form_control.is_some()
        || element.accessibility.is_some()
        || element.source_intent.is_some()
        || element.semantics.is_some()
        || element.region.is_some()
        || !element.dynamic_styles.is_empty()
        || !element.style_variants.is_empty()
        || !element.pseudo_elements.is_empty()
    {
        return false;
    }

    let tag = html_tag(element);
    if matches!(tag, "html" | "body" | "x-dc" | "helmet") {
        return true;
    }

    tag == "div"
        && element.classes.is_empty()
        && element.attributes.iter().all(|attribute| {
            matches!(
                attribute.name.as_str(),
                "id" | "data-htmlswap-generated-root" | "data-htmlswap-document-wrapper"
            ) && is_generated_attribute_value(attribute.value.as_str())
        })
        && element.styles.iter().all(|style| {
            matches!(
                style.property.as_str(),
                "margin" | "padding" | "width" | "height" | "min-width" | "min-height"
            )
        })
}

fn is_document_envelope_tag(tag: &str) -> bool {
    matches!(
        tag,
        "html" | "head" | "body" | "x-dc" | "helmet" | "style" | "script" | "link" | "meta"
    )
}

fn collect_node_features(
    node: &RenderNode,
    path: &str,
    features: &mut FeatureCounts,
    text_transform: TextTransform,
) {
    match node {
        RenderNode::Element(element) => {
            collect_element_features(element, path, features, text_transform);
        }
        RenderNode::Text(text) => {
            let value = text.template.as_ref().map_or_else(
                || normalize_text(&transform_text(&text.value, text_transform)),
                |template| normalize_text(&template_signature(template, text_transform)),
            );
            if value.is_empty() {
                return;
            }
            insert(
                features,
                RoundTripFeatureKind::Text,
                format!("{path}:literal={value}"),
            );
            if let Some(template) = &text.template {
                insert(
                    features,
                    RoundTripFeatureKind::Text,
                    format!(
                        "{path}:template={}",
                        template_signature(template, text_transform)
                    ),
                );
            }
        }
        RenderNode::Raw(raw) => {
            let html = normalize_text(&raw.html);
            if !html.is_empty() {
                insert(
                    features,
                    RoundTripFeatureKind::Raw,
                    format!("{path}:{}", preview(&html, 160)),
                );
            }
        }
    }
}

fn collect_element_features(
    element: &RenderElement,
    path: &str,
    features: &mut FeatureCounts,
    inherited_text_transform: TextTransform,
) {
    let tag = html_tag(element);
    let text_transform = text_transform_for_element(element, inherited_text_transform);
    insert(
        features,
        RoundTripFeatureKind::Element,
        format!("{path}:tag={tag}:role={}", role_signature(&element.role)),
    );

    collect_attributes(element, path, features);
    collect_styles(element, path, features);
    collect_actions(element, path, features);
    collect_state(element, path, features);
    collect_form_control(element, path, features);
    collect_accessibility(element, path, features);
    collect_source_intent(element, path, features);
    collect_semantics(element, path, features);
    collect_control_flow(element.control_flow.as_deref(), path, features);

    for (index, child) in element.children.iter().enumerate() {
        collect_node_features(
            child,
            &format!("{path}/{tag}[{index}]"),
            features,
            text_transform,
        );
    }
}

fn collect_attributes(element: &RenderElement, path: &str, features: &mut FeatureCounts) {
    for class in &element.classes {
        insert(
            features,
            RoundTripFeatureKind::Class,
            format!("{path}:{}", class.trim()),
        );
    }

    for attribute in &element.attributes {
        let name = attribute.name.trim().to_ascii_lowercase();
        if should_ignore_attribute(&name, attribute.value.as_str()) {
            continue;
        }
        let value = attribute
            .template
            .as_ref()
            .map(|template| template_signature(template, TextTransform::None))
            .unwrap_or_else(|| normalize_attribute_value(attribute.value.as_str()));
        insert(
            features,
            RoundTripFeatureKind::Attribute,
            format!("{path}:{name}={value}"),
        );
    }
}

fn collect_styles(element: &RenderElement, path: &str, features: &mut FeatureCounts) {
    for signature in style_feature_signatures_for_element(element) {
        insert(
            features,
            RoundTripFeatureKind::Style,
            format!("{path}:{signature}"),
        );
    }

    for variant in &element.style_variants {
        insert(
            features,
            RoundTripFeatureKind::StyleVariant,
            format!("{path}:{}", style_variant_signature(variant)),
        );
    }

    for binding in &element.dynamic_styles {
        let state = binding.state.as_deref().unwrap_or("");
        insert(
            features,
            RoundTripFeatureKind::DynamicStyle,
            format!(
                "{path}:state={state}:expr={}",
                template_signature(&binding.expression, TextTransform::None)
            ),
        );
    }

    if !is_document_envelope_tag(html_tag(element)) {
        for pseudo in &element.pseudo_elements {
            insert(
                features,
                RoundTripFeatureKind::PseudoElement,
                format!(
                    "{path}:{}:{}:{}",
                    pseudo.kind,
                    pseudo.selector,
                    styles_signature(&pseudo.styles)
                ),
            );
        }
    }
}

fn collect_actions(element: &RenderElement, path: &str, features: &mut FeatureCounts) {
    for action in &element.actions {
        insert(
            features,
            RoundTripFeatureKind::Action,
            format!("{path}:{}", action_signature(action)),
        );
    }
}

fn collect_control_flow(
    control_flow: Option<&RenderControlFlow>,
    path: &str,
    features: &mut FeatureCounts,
) {
    let Some(control_flow) = control_flow else {
        return;
    };
    let expression = control_flow
        .expression
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    let binding = control_flow
        .binding
        .as_ref()
        .map(|binding| binding.name.as_str())
        .unwrap_or("");
    let placeholder = control_flow.placeholder.as_deref().unwrap_or("");
    insert(
        features,
        RoundTripFeatureKind::ControlFlow,
        format!(
            "{path}:kind={:?}:host={:?}:expr={expression}:binding={binding}:placeholder={placeholder}",
            control_flow.kind, control_flow.host
        ),
    );
}

fn collect_state(element: &RenderElement, path: &str, features: &mut FeatureCounts) {
    let Some(state) = element.state.as_deref() else {
        return;
    };
    insert(
        features,
        RoundTripFeatureKind::State,
        format!("{path}:{}", state_signature(state)),
    );
}

fn collect_form_control(element: &RenderElement, path: &str, features: &mut FeatureCounts) {
    let Some(control) = element.form_control.as_deref() else {
        return;
    };
    insert(
        features,
        RoundTripFeatureKind::FormControl,
        format!("{path}:{}", form_control_signature(control)),
    );
}

fn collect_accessibility(element: &RenderElement, path: &str, features: &mut FeatureCounts) {
    let Some(accessibility) = element.accessibility.as_deref() else {
        return;
    };
    let signature = accessibility_signature(accessibility);
    if !signature.is_empty() {
        insert(
            features,
            RoundTripFeatureKind::Accessibility,
            format!("{path}:{signature}"),
        );
    }
}

fn collect_source_intent(element: &RenderElement, path: &str, features: &mut FeatureCounts) {
    let Some(intent) = element.source_intent.as_deref() else {
        return;
    };
    let signature = source_intent_signature(intent);
    if !signature.is_empty() {
        insert(
            features,
            RoundTripFeatureKind::SourceIntent,
            format!("{path}:{signature}"),
        );
    }
}

fn collect_semantics(element: &RenderElement, path: &str, features: &mut FeatureCounts) {
    let Some(semantics) = element.semantics.as_deref() else {
        return;
    };
    let signature = semantics_signature(semantics);
    if !signature.is_empty() {
        insert(
            features,
            RoundTripFeatureKind::Semantics,
            format!("{path}:{signature}"),
        );
    }
}

fn action_signature(action: &ActionBinding) -> String {
    let event = canonical_event(action.event.as_str());
    let action_name = action
        .action
        .as_deref()
        .unwrap_or(action.expression.as_str())
        .trim();
    let template = action
        .template
        .as_ref()
        .map(|template| template_signature(template, TextTransform::None))
        .unwrap_or_default();
    let invocations = action
        .handler
        .invocations
        .iter()
        .map(|invocation| {
            let arguments = invocation
                .arguments
                .iter()
                .map(|argument| format!("{argument:?}"))
                .collect::<Vec<_>>()
                .join(",");
            format!("{}({arguments})", invocation.action)
        })
        .collect::<Vec<_>>()
        .join("|");
    let effects = action
        .handler
        .effects
        .iter()
        .map(action_effect_signature)
        .collect::<Vec<_>>()
        .join("|");
    format!(
        "event={event}:action={}:template={template}:payload={}:invocations={invocations}:effects={effects}",
        normalize_binding(action_name),
        payload_signature(&action.payload)
    )
}

fn action_effect_signature(effect: &RenderActionHandlerEffect) -> &'static str {
    match effect {
        RenderActionHandlerEffect::PreventDefault { .. } => "prevent-default",
        RenderActionHandlerEffect::StopPropagation { .. } => "stop-propagation",
    }
}

fn payload_signature(payload: &ActionPayload) -> String {
    match payload {
        ActionPayload::None => "none".to_owned(),
        ActionPayload::ElementState { state_id } => {
            format!("element-state:{}", normalized_state_id(state_id))
        }
        ActionPayload::FormData { controls } => format!(
            "form-data:{}",
            controls
                .iter()
                .map(|field| {
                    format!(
                        "{}:{}",
                        field.name,
                        field
                            .state_id
                            .as_deref()
                            .map(normalized_state_id)
                            .unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

fn state_signature(state: &RenderStateBinding) -> String {
    match &state.kind {
        RenderStateKind::TextInput(text) => format!(
            "owner={:?}:text-input:{}",
            state.owner,
            text_input_signature(text)
        ),
        RenderStateKind::Choice(choice) => format!(
            "owner={:?}:choice:multiple={}:disabled={}:selected={:?}:options={}",
            state.owner,
            choice.multiple,
            choice.disabled,
            choice.selected_index,
            choice
                .options
                .iter()
                .map(|option| format!("{}={}:{}", option.label, option.value, option.disabled))
                .collect::<Vec<_>>()
                .join(",")
        ),
        RenderStateKind::Toggle(toggle) => format!(
            "owner={:?}:toggle:checked={}:disabled={}",
            state.owner, toggle.checked, toggle.disabled
        ),
    }
}

fn text_input_signature(text: &RenderTextInputState) -> String {
    format!(
        "type={}:value={}:template={}:placeholder={}:placeholder_template={}:multiline={}:rows={:?}:password={}:disabled={}:readonly={}",
        text.input_type,
        text.initial_value.as_deref().unwrap_or(""),
        text.initial_template
            .as_ref()
            .map(|template| template_signature(template, TextTransform::None))
            .unwrap_or_default(),
        text.placeholder.as_deref().unwrap_or(""),
        text.placeholder_template
            .as_ref()
            .map(|template| template_signature(template, TextTransform::None))
            .unwrap_or_default(),
        text.multiline,
        text.rows,
        text.password,
        text.disabled,
        text.readonly
    )
}

fn form_control_signature(control: &RenderFormControl) -> String {
    let id = control
        .id
        .as_deref()
        .filter(|id| !is_generated_attribute_value(id))
        .unwrap_or("");
    let options = control
        .options
        .iter()
        .map(|option| format!("{}={}:{}", option.label, option.value, option.disabled))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "id={id}:type={:?}:name={}:value={}:group={}:label={}:options={options}:required={}:disabled={}:readonly={}:validation={:?}",
        control.control_type,
        control.name.as_deref().unwrap_or(""),
        control.value.as_deref().unwrap_or(""),
        control.group.as_deref().unwrap_or(""),
        control.label.as_deref().unwrap_or(""),
        control.required,
        control.disabled,
        control.readonly,
        control.validation
    )
}

fn accessibility_signature(accessibility: &RenderAccessibility) -> String {
    let aria = accessibility
        .aria
        .iter()
        .map(|attribute| format!("{}={}", attribute.name, attribute.value))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "role={}:label={}:labelled-by={}:described-by={}:tab-index={:?}:autofocus={}:hidden={}:aria={aria}",
        accessibility.role.as_deref().unwrap_or(""),
        accessibility.label.as_deref().unwrap_or(""),
        accessibility.labelled_by.as_deref().unwrap_or(""),
        accessibility.described_by.as_deref().unwrap_or(""),
        accessibility.tab_index,
        accessibility.autofocus,
        accessibility.hidden
    )
}

fn source_intent_signature(intent: &RenderSourceIntent) -> String {
    let props = intent
        .props
        .iter()
        .map(|prop| {
            let value = prop
                .template
                .as_ref()
                .map(|template| template_signature(template, TextTransform::None))
                .unwrap_or_else(|| normalize_attribute_value(prop.value.as_str()));
            format!("{}={value}", prop.name.to_ascii_lowercase())
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "key={}:state={}:component={}:source={}:slot={}:child-strategy={}:props={props}",
        intent.key.as_deref().unwrap_or(""),
        intent.state_id.as_deref().unwrap_or(""),
        intent
            .component
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        intent.component_source.as_deref().unwrap_or(""),
        intent
            .slot
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        intent.child_strategy.as_deref().unwrap_or("")
    )
}

fn semantics_signature(semantics: &RenderSemantics) -> String {
    let extras = semantics
        .extras
        .iter()
        .map(|extra| format!("{}={}", extra.axis, extra.value))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "variant={}:tone={}:size={}:density={}:extras={extras}",
        semantics
            .variant
            .as_ref()
            .map(|value| value.as_str())
            .unwrap_or(""),
        semantics
            .tone
            .as_ref()
            .map(|value| value.as_str())
            .unwrap_or(""),
        semantics
            .size
            .as_ref()
            .map(|value| value.as_str())
            .unwrap_or(""),
        semantics
            .density
            .as_ref()
            .map(|value| value.as_str())
            .unwrap_or("")
    )
}

fn style_variant_signature(variant: &RenderStyleVariant) -> String {
    format!(
        "selector={}:conditions={}:styles={}",
        variant.selector,
        style_conditions_signature(&variant.conditions),
        styles_signature(&variant.declarations)
    )
}

fn style_conditions_signature(conditions: &[RenderStyleCondition]) -> String {
    conditions
        .iter()
        .map(|condition| match condition {
            RenderStyleCondition::PseudoClass(value) => format!("pseudo-class:{value}"),
            RenderStyleCondition::PseudoElement(value) => format!("pseudo-element:{value}"),
            RenderStyleCondition::Media(value) => format!("media:{value}"),
            RenderStyleCondition::Supports(value) => format!("supports:{value}"),
            RenderStyleCondition::Container(value) => format!("container:{value}"),
            RenderStyleCondition::StartingStyle => "starting-style".to_owned(),
            RenderStyleCondition::ActiveViewTransitionType(types) => {
                format!("active-view-transition-type:{}", types.join(","))
            }
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn styles_signature(styles: &[StyleDeclaration]) -> String {
    style_feature_signatures(styles).join(";")
}

#[derive(Debug, Clone)]
struct CanonicalStyleEntry {
    property: String,
    value: String,
    important: bool,
}

impl CanonicalStyleEntry {
    fn new(property: impl Into<String>, value: impl Into<String>, important: bool) -> Self {
        Self {
            property: property.into(),
            value: value.into(),
            important,
        }
    }

    fn signature(&self) -> String {
        let important = if self.important { " !important" } else { "" };
        format!("{}={}{}", self.property, self.value, important)
    }
}

fn style_feature_signatures(styles: &[StyleDeclaration]) -> Vec<String> {
    canonical_style_signatures(styles.iter())
}

fn style_feature_signatures_for_element(element: &RenderElement) -> Vec<String> {
    canonical_style_signatures(
        element
            .styles
            .iter()
            .filter(|style| !is_material_symbol_presentation_style(element, style)),
    )
}

fn canonical_style_signatures<'a>(
    styles: impl Iterator<Item = &'a StyleDeclaration>,
) -> Vec<String> {
    let mut entries = styles.flat_map(canonical_style_entries).collect::<Vec<_>>();
    scope_border_colors_to_visible_sides(&mut entries);
    entries.into_iter().map(|entry| entry.signature()).collect()
}

fn is_material_symbol_presentation_style(
    element: &RenderElement,
    style: &StyleDeclaration,
) -> bool {
    is_material_symbol_element(element)
        && matches!(
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

fn is_material_symbol_element(element: &RenderElement) -> bool {
    element.classes.iter().any(|class| {
        matches!(
            class.as_str(),
            "ms" | "material-icons" | "material-symbols-outlined"
        )
    })
}

fn canonical_style_entries(style: &StyleDeclaration) -> Vec<CanonicalStyleEntry> {
    let property = canonical_style_property(style);
    let value = normalize_style_value_for_property(property, style.value.as_str());
    if should_ignore_style(property, &value) {
        return Vec::new();
    }

    let entries = match property {
        "padding" | "margin" => expand_box_shorthand(property, &value, style.important)
            .unwrap_or_else(|| vec![CanonicalStyleEntry::new(property, value, style.important)]),
        "inset" => expand_box_shorthand_with(&value, style.important, str::to_owned)
            .unwrap_or_else(|| vec![CanonicalStyleEntry::new(property, value, style.important)]),
        "flex" => expand_flex_shorthand(&value, style.important)
            .unwrap_or_else(|| vec![CanonicalStyleEntry::new(property, value, style.important)]),
        "border" | "border-top" | "border-right" | "border-bottom" | "border-left" => {
            expand_border_shorthand(property, &value, style.important)
                .unwrap_or_else(|| vec![CanonicalStyleEntry::new(property, value, style.important)])
        }
        "border-width" => expand_box_shorthand_with(&value, style.important, |side| {
            format!("border-{side}-width")
        })
        .unwrap_or_else(|| vec![CanonicalStyleEntry::new(property, value, style.important)]),
        "border-style" => expand_box_shorthand_with(&value, style.important, |side| {
            format!("border-{side}-style")
        })
        .unwrap_or_else(|| vec![CanonicalStyleEntry::new(property, value, style.important)]),
        "border-color" => expand_box_shorthand_with(&value, style.important, |side| {
            format!("border-{side}-color")
        })
        .unwrap_or_else(|| vec![CanonicalStyleEntry::new(property, value, style.important)]),
        _ => vec![CanonicalStyleEntry::new(property, value, style.important)],
    };

    entries
        .into_iter()
        .filter(|entry| !should_ignore_style(&entry.property, &entry.value))
        .collect()
}

fn should_ignore_style(property: &str, value: &str) -> bool {
    if property.starts_with("border-")
        && property.ends_with("-width")
        && matches!(value, "0" | "0px")
    {
        return true;
    }
    if property.starts_with("border-") && property.ends_with("-color") && value == "#0000" {
        return true;
    }
    if property.starts_with("border-") && property.ends_with("-style") && value == "solid" {
        return true;
    }

    matches!(property, "text-transform" | "-webkit-font-smoothing")
        || matches!(
            (property, value),
            ("box-sizing", "border-box")
                | ("background", "none")
                | ("background-color", "none")
                | ("background-size", "auto")
                | ("border", "none")
                | ("border-width", "0")
                | ("border-width", "0px")
                | ("border-color", "#0000")
                | ("border-style", "solid")
                | ("color", "inherit")
                | ("cursor", "default")
                | ("direction", "ltr")
                | ("align-items", "normal")
                | ("align-items", "stretch")
                | ("align-self", "auto")
                | ("align-self", "normal")
                | ("align-self", "stretch")
                | ("justify-content", "normal")
                | ("justify-content", "stretch")
                | ("flex-shrink", "1")
                | ("font-style", "normal")
                | ("font-variation-settings", "normal")
                | ("letter-spacing", "0")
                | ("letter-spacing", "0px")
                | ("letter-spacing", "normal")
                | ("line-height", "normal")
                | ("line-height", "1")
                | ("margin", "0")
                | ("margin-top", "0")
                | ("margin-right", "0")
                | ("margin-bottom", "0")
                | ("margin-left", "0")
                | ("outline", "none")
                | ("pointer-events", "auto")
                | ("padding", "0")
                | ("padding-top", "0")
                | ("padding-right", "0")
                | ("padding-bottom", "0")
                | ("padding-left", "0")
                | ("user-select", "none")
                | ("vertical-align", "baseline")
                | ("vertical-align", "middle")
        )
}

fn expand_box_shorthand(
    property: &str,
    value: &str,
    important: bool,
) -> Option<Vec<CanonicalStyleEntry>> {
    expand_box_shorthand_with(value, important, |side| format!("{property}-{side}"))
}

fn expand_box_shorthand_with(
    value: &str,
    important: bool,
    property_for_side: impl Fn(&str) -> String,
) -> Option<Vec<CanonicalStyleEntry>> {
    let values = css_space_tokens(value);
    let [top, right, bottom, left] = match values.as_slice() {
        [] => return Some(Vec::new()),
        [all] => [all, all, all, all],
        [vertical, horizontal] => [vertical, horizontal, vertical, horizontal],
        [top, horizontal, bottom] => [top, horizontal, bottom, horizontal],
        [top, right, bottom, left] => [top, right, bottom, left],
        _ => return None,
    };

    Some(vec![
        CanonicalStyleEntry::new(property_for_side("top"), top.clone(), important),
        CanonicalStyleEntry::new(property_for_side("right"), right.clone(), important),
        CanonicalStyleEntry::new(property_for_side("bottom"), bottom.clone(), important),
        CanonicalStyleEntry::new(property_for_side("left"), left.clone(), important),
    ])
}

fn expand_border_shorthand(
    property: &str,
    value: &str,
    important: bool,
) -> Option<Vec<CanonicalStyleEntry>> {
    if matches!(value, "none" | "0" | "0px") {
        return Some(Vec::new());
    }

    let mut width = None;
    let mut style = None;
    let mut color = None;
    for token in css_space_tokens(value) {
        let normalized = normalize_style_value(&token);
        let lower = normalized.to_ascii_lowercase();
        if is_border_style(&lower) {
            style = Some(lower);
        } else if looks_like_color_value(&normalized) || lower.starts_with("var(") {
            color = Some(normalized);
        } else if is_border_width(&lower) {
            width = Some(normalized);
        } else {
            return None;
        }
    }

    let mut entries = Vec::new();
    for side in border_sides(property)? {
        if let Some(width) = &width {
            entries.push(CanonicalStyleEntry::new(
                format!("border-{side}-width"),
                width.clone(),
                important,
            ));
        }
        if let Some(color) = &color {
            entries.push(CanonicalStyleEntry::new(
                format!("border-{side}-color"),
                color.clone(),
                important,
            ));
        }
        if let Some(style) = &style
            && style != "solid"
        {
            entries.push(CanonicalStyleEntry::new(
                format!("border-{side}-style"),
                style.clone(),
                important,
            ));
        }
    }
    Some(entries)
}

fn expand_flex_shorthand(value: &str, important: bool) -> Option<Vec<CanonicalStyleEntry>> {
    let normalized = normalize_text(value).to_ascii_lowercase();
    let (grow, shrink, basis) = match normalized.as_str() {
        "none" | "0" => ("0", "0", None),
        "auto" => ("1", "1", Some("auto")),
        "initial" => ("0", "1", Some("auto")),
        "1" => ("1", "1", Some("0")),
        _ => {
            let parts = normalized.split_whitespace().collect::<Vec<_>>();
            match parts.as_slice() {
                [basis] => ("1", "1", Some(*basis)),
                [grow, shrink, basis] => (*grow, *shrink, Some(*basis)),
                _ => return None,
            }
        }
    };

    let mut entries = vec![
        CanonicalStyleEntry::new("flex-grow", grow, important),
        CanonicalStyleEntry::new("flex-shrink", shrink, important),
    ];
    if let Some(basis) = basis {
        entries.push(CanonicalStyleEntry::new(
            "flex-basis",
            normalize_style_value(basis),
            important,
        ));
    }
    Some(entries)
}

fn border_sides(property: &str) -> Option<&'static [&'static str]> {
    match property {
        "border" => Some(&["top", "right", "bottom", "left"]),
        "border-top" => Some(&["top"]),
        "border-right" => Some(&["right"]),
        "border-bottom" => Some(&["bottom"]),
        "border-left" => Some(&["left"]),
        _ => None,
    }
}

fn is_border_style(value: &str) -> bool {
    matches!(
        value,
        "none"
            | "hidden"
            | "dotted"
            | "dashed"
            | "solid"
            | "double"
            | "groove"
            | "ridge"
            | "inset"
            | "outset"
    )
}

fn is_border_width(value: &str) -> bool {
    matches!(value, "thin" | "medium" | "thick")
        || value == "0"
        || value.ends_with("px")
        || value.ends_with("rem")
        || value.ends_with("em")
        || value.ends_with('%')
}

fn scope_border_colors_to_visible_sides(entries: &mut Vec<CanonicalStyleEntry>) {
    let visible_sides = entries
        .iter()
        .filter_map(|entry| {
            border_side_with_suffix(&entry.property, "width")
                .map(str::to_owned)
                .filter(|_| !matches!(entry.value.as_str(), "0" | "0px"))
        })
        .collect::<BTreeSet<_>>();
    if visible_sides.is_empty() {
        return;
    }

    entries.retain(|entry| {
        border_side_with_suffix(&entry.property, "color")
            .is_none_or(|side| visible_sides.contains(side))
            && border_side_with_suffix(&entry.property, "style")
                .is_none_or(|side| visible_sides.contains(side))
    });
}

fn border_side_with_suffix<'a>(property: &'a str, suffix: &str) -> Option<&'a str> {
    let property = property.strip_prefix("border-")?;
    let suffix = format!("-{suffix}");
    property.strip_suffix(suffix.as_str())
}

fn css_space_tokens(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut quote = None;

    for ch in value.chars() {
        if let Some(active_quote) = quote {
            current.push(ch);
            if ch == active_quote {
                quote = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' => {
                quote = Some(ch);
                current.push(ch);
            }
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ch if ch.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    tokens.push(current);
                    current = String::new();
                }
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn canonical_style_property(style: &StyleDeclaration) -> &str {
    if style.property.as_str() == "background" && looks_like_color_value(style.value.as_str()) {
        "background-color"
    } else {
        style.property.as_str()
    }
}

fn looks_like_color_value(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value.starts_with('#')
        || value.starts_with("rgb(")
        || value.starts_with("rgba(")
        || value.starts_with("hsl(")
        || value.starts_with("hsla(")
        || value.starts_with("oklch(")
        || value.starts_with("oklab(")
        || value.starts_with("lab(")
        || value.starts_with("lch(")
        || matches!(
            value.as_str(),
            "black" | "white" | "red" | "green" | "blue" | "transparent" | "currentcolor"
        )
}

fn normalize_style_value_for_property(property: &str, value: &str) -> String {
    let value = normalize_style_value(value);
    match property {
        "aspect-ratio" => normalize_aspect_ratio(&value),
        "background" | "background-image" => normalize_css_function_commas(&value),
        "box-shadow" => normalize_box_shadow(&value),
        "font-family" => normalize_font_family(&value),
        "font-weight" => normalize_font_weight(&value),
        "grid-template-columns" => normalize_grid_template_columns(&value),
        "height" => normalize_viewport_height(&value),
        "line-height" => normalize_line_height(&value),
        "left" | "right" | "top" | "bottom" => normalize_calc_position(&value),
        "max-height" => normalize_viewport_height(&value),
        "max-width" => normalize_viewport_width(&value),
        "min-height" => normalize_viewport_height(&value),
        "min-width" => normalize_viewport_width(&value),
        "overflow" | "overflow-x" | "overflow-y" if value == "auto" => "scroll".to_owned(),
        "border-radius" if matches!(value.as_str(), "50%" | "9999px") => "round".to_owned(),
        "transform" => normalize_transform(&value),
        "width" => normalize_viewport_width(&value),
        _ => value,
    }
}

fn normalize_grid_template_columns(value: &str) -> String {
    let value = normalize_css_function_commas(value);
    if let Some((count, track)) = repeat_grid_columns(&value) {
        return std::iter::repeat_n(track, count)
            .collect::<Vec<_>>()
            .join(" ");
    }
    value
}

fn repeat_grid_columns(value: &str) -> Option<(usize, String)> {
    let inner = value.strip_prefix("repeat(")?.strip_suffix(')')?;
    let (count, track) = inner.split_once(',')?;
    let count = count.trim().parse::<usize>().ok()?;
    (count > 0).then(|| (count, track.trim().to_owned()))
}

fn normalize_transform(value: &str) -> String {
    normalize_css_function_commas(value)
}

fn normalize_css_function_commas(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut previous_was_comma = false;
    for ch in value.chars() {
        if ch == ',' {
            output.push(',');
            previous_was_comma = true;
        } else if previous_was_comma && ch.is_whitespace() {
            continue;
        } else {
            output.push(ch);
            previous_was_comma = false;
        }
    }
    output
}

fn normalize_viewport_width(value: &str) -> String {
    normalize_viewport_dimension(value, ["vw", "dvw", "svw", "lvw"])
}

fn normalize_viewport_height(value: &str) -> String {
    normalize_viewport_dimension(value, ["vh", "dvh", "svh", "lvh"])
}

fn normalize_viewport_dimension(value: &str, units: [&str; 4]) -> String {
    for unit in units {
        if let Some(number) = value.strip_suffix(unit)
            && number.trim().parse::<f64>().is_ok()
        {
            return format!("{}%", trim_numeric(number.trim()));
        }
    }
    value.to_owned()
}

fn normalize_calc_position(value: &str) -> String {
    let value = normalize_css_function_commas(value);
    let Some(inner) = value
        .strip_prefix("calc(")
        .and_then(|value| value.strip_suffix(')'))
    else {
        return value;
    };
    let Some((left, right)) = inner.split_once(" + ") else {
        return value;
    };
    if right.trim().ends_with("* 0") {
        left.trim().to_owned()
    } else {
        value
    }
}

fn normalize_aspect_ratio(value: &str) -> String {
    let value = value.trim();
    if let Some((width, height)) = value.split_once('/')
        && let (Ok(width), Ok(height)) = (width.trim().parse::<f64>(), height.trim().parse::<f64>())
        && height != 0.0
    {
        return trim_numeric(&(width / height).to_string());
    }
    trim_numeric(value)
}

fn normalize_line_height(value: &str) -> String {
    if let Some(percent) = value.strip_suffix('%')
        && let Ok(percent) = percent.trim().parse::<f64>()
    {
        return trim_numeric(&(percent / 100.0).to_string());
    }
    trim_numeric(value)
}

fn normalize_box_shadow(value: &str) -> String {
    normalize_rgba_functions(value)
}

fn normalize_rgba_functions(value: &str) -> String {
    let mut output = String::new();
    let mut rest = value;
    while let Some(start) = rest.find("rgba(") {
        output.push_str(&rest[..start]);
        let after_start = &rest[start..];
        let Some(end) = after_start.find(')') else {
            output.push_str(after_start);
            return output;
        };
        let function = &after_start[..=end];
        if let Some(hex) = rgba_function_to_hex(function) {
            output.push_str(&hex);
        } else {
            output.push_str(function);
        }
        rest = &after_start[end + 1..];
    }
    output.push_str(rest);
    output
}

fn rgba_function_to_hex(value: &str) -> Option<String> {
    let args = value.strip_prefix("rgba(")?.strip_suffix(')')?;
    let parts = args.split(',').map(str::trim).collect::<Vec<_>>();
    let [red, green, blue, alpha] = parts.as_slice() else {
        return None;
    };
    let red = red.parse::<u8>().ok()?;
    let green = green.parse::<u8>().ok()?;
    let blue = blue.parse::<u8>().ok()?;
    let alpha = alpha.parse::<f64>().ok()?;
    let alpha = if alpha <= 1.0 { alpha * 255.0 } else { alpha };
    Some(format!(
        "#{red:02x}{green:02x}{blue:02x}{:02x}",
        alpha.clamp(0.0, 255.0).round() as u8
    ))
}

fn trim_numeric(value: &str) -> String {
    let value = value.trim();
    if !value.contains('.') {
        return value.to_owned();
    }
    let trimmed = value.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() {
        "0".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn normalize_font_family(value: &str) -> String {
    first_css_list_item(value)
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_owned()
}

fn first_css_list_item(value: &str) -> &str {
    let mut depth = 0usize;
    let mut quote = None;
    for (index, ch) in value.char_indices() {
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' => quote = Some(ch),
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => return &value[..index],
            _ => {}
        }
    }
    value
}

fn normalize_font_weight(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "thin" => "100".to_owned(),
        "extralight" | "extra-light" => "200".to_owned(),
        "light" => "300".to_owned(),
        "normal" | "regular" => "400".to_owned(),
        "medium" => "500".to_owned(),
        "semibold" | "semi-bold" | "demibold" | "demi-bold" => "600".to_owned(),
        "bold" => "700".to_owned(),
        "extrabold" | "extra-bold" => "800".to_owned(),
        "black" | "heavy" => "900".to_owned(),
        _ => value.to_owned(),
    }
}

fn text_transform_for_element(
    element: &RenderElement,
    inherited_text_transform: TextTransform,
) -> TextTransform {
    let mut text_transform = inherited_text_transform;
    for style in &element.styles {
        if style.property.as_str() != "text-transform" {
            continue;
        }
        match style.value.as_str().trim().to_ascii_lowercase().as_str() {
            "none" => text_transform = TextTransform::None,
            "uppercase" => text_transform = TextTransform::Uppercase,
            "lowercase" => text_transform = TextTransform::Lowercase,
            _ => {}
        }
    }
    text_transform
}

fn transform_text(value: &str, text_transform: TextTransform) -> String {
    match text_transform {
        TextTransform::None => value.to_owned(),
        TextTransform::Uppercase => value.to_uppercase(),
        TextTransform::Lowercase => value.to_lowercase(),
    }
}

fn template_signature(template: &TemplateString, text_transform: TextTransform) -> String {
    template
        .segments
        .iter()
        .map(|segment| match segment {
            TemplateSegment::Literal(value) => {
                normalize_text(&transform_text(value, text_transform))
            }
            TemplateSegment::Expression(expression) => {
                format!("{{{{ {} }}}}", normalize_expression(expression))
            }
        })
        .collect::<Vec<_>>()
        .join("")
}

fn normalize_expression(expression: &Expr) -> String {
    normalize_binding(&expression.to_string())
}

fn normalize_binding(value: &str) -> String {
    let value = value
        .trim()
        .trim_start_matches("{{")
        .trim_end_matches("}}")
        .trim()
        .trim_end_matches("()")
        .trim();

    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.'))
    {
        return value
            .split('.')
            .map(|segment| {
                segment
                    .chars()
                    .filter(|ch| ch.is_ascii_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join(".");
    }

    value.to_owned()
}

fn normalize_attribute_value(value: &str) -> String {
    normalize_text(value)
}

fn normalize_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_style_value(value: &str) -> String {
    let value = normalize_text(value);
    if value.eq_ignore_ascii_case("transparent") {
        "#0000".to_owned()
    } else if value.starts_with('#') && value.chars().skip(1).all(|ch| ch.is_ascii_hexdigit()) {
        value.to_ascii_lowercase()
    } else {
        value
    }
}

fn canonical_event(event: &str) -> String {
    let event = event
        .trim()
        .trim_start_matches("on")
        .to_ascii_lowercase()
        .replace('-', "");
    match event.as_str() {
        "dblclick" | "doubleclick" => "doubleclick".to_owned(),
        _ => event,
    }
}

fn role_signature(role: &UiRole) -> String {
    match role {
        UiRole::Heading(level) => format!("heading-{level}"),
        UiRole::List { ordered } => {
            if *ordered {
                "ordered-list".to_owned()
            } else {
                "list".to_owned()
            }
        }
        _ => role.name().to_owned(),
    }
}

fn html_tag(element: &RenderElement) -> &str {
    if !element.source_tag.is_empty() {
        element.source_tag.as_str()
    } else {
        match element.role {
            UiRole::Container => "div",
            UiRole::Inline => "span",
            UiRole::Paragraph => "p",
            UiRole::Button => "button",
            UiRole::TextInput => "input",
            UiRole::Select => "select",
            UiRole::Option => "option",
            UiRole::Link => "a",
            UiRole::Image => "img",
            UiRole::Heading(1) => "h1",
            UiRole::Heading(2) => "h2",
            UiRole::Heading(3) => "h3",
            UiRole::Heading(4) => "h4",
            UiRole::Heading(5) => "h5",
            UiRole::Heading(_) => "h6",
            UiRole::List { ordered: false } => "ul",
            UiRole::List { ordered: true } => "ol",
            UiRole::ListItem => "li",
            UiRole::Form => "form",
            UiRole::Fieldset => "fieldset",
            UiRole::Legend => "legend",
            UiRole::Label => "label",
            UiRole::Unknown => "div",
        }
    }
}

fn should_ignore_attribute(name: &str, value: &str) -> bool {
    matches!(name, "class" | "style")
        || (name == "id" && is_generated_attribute_value(value))
        || matches!(
            name,
            "data-htmlswap-if"
                | "data-htmlswap-icon-system"
                | "data-htmlswap-generated-root"
                | "data-htmlswap-document-wrapper"
        )
}

fn is_generated_attribute_value(value: &str) -> bool {
    value.is_empty()
        || value.starts_with("htmlswap_interactive_")
        || value.starts_with("htmlswap_link_")
        || value.starts_with("htmlswap_generated_")
}

fn normalized_state_id(value: &str) -> String {
    if is_generated_attribute_value(value) || looks_like_generated_state_id(value) {
        "<generated>".to_owned()
    } else {
        value.to_owned()
    }
}

fn looks_like_generated_state_id(value: &str) -> bool {
    let Some((prefix, suffix)) = value.rsplit_once('_') else {
        return false;
    };
    matches!(
        prefix,
        "input" | "text_input" | "select" | "checkbox" | "radio" | "state"
    ) && suffix.chars().all(|character| character.is_ascii_digit())
}

fn preview(value: &str, max_chars: usize) -> String {
    let mut preview = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        preview.push_str("...");
    }
    preview
}

fn insert(features: &mut FeatureCounts, kind: RoundTripFeatureKind, value: impl Into<String>) {
    *features.entry(Feature::new(kind, value)).or_insert(0) += 1;
}
