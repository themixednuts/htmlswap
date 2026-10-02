use std::collections::BTreeSet;
use std::fmt;
use std::hash::{Hash, Hasher};

use arcstr::ArcStr;
use compact_str::CompactString;

use crate::expr::{BindingPattern, Expr, TemplateString};
use crate::source::{SourceId, Span};
pub use crate::style::StyleDeclaration;
use crate::style::{StyleProperty, StyleValue};

#[derive(Debug, Clone)]
pub struct ComponentId {
    value: CompactString,
    canonical: CompactString,
}

impl ComponentId {
    #[must_use]
    pub fn new(value: impl AsRef<str>) -> Self {
        let value = value.as_ref().trim();
        Self {
            value: CompactString::from(value),
            canonical: canonical_source_identifier(value),
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }

    #[must_use]
    pub fn canonical_str(&self) -> &str {
        &self.canonical
    }

    #[must_use]
    pub fn is(&self, expected: &str) -> bool {
        self.canonical == canonical_source_identifier(expected)
    }

    #[must_use]
    pub fn canonical(value: &str) -> CompactString {
        canonical_source_identifier(value)
    }
}

impl PartialEq for ComponentId {
    fn eq(&self, other: &Self) -> bool {
        self.canonical == other.canonical
    }
}

impl Eq for ComponentId {}

impl PartialOrd for ComponentId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ComponentId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.canonical.cmp(&other.canonical)
    }
}

impl Hash for ComponentId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.canonical.hash(state);
    }
}

impl fmt::Display for ComponentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<&str> for ComponentId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ComponentId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<CompactString> for ComponentId {
    fn from(value: CompactString) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlotId(CompactString);

impl SlotId {
    #[must_use]
    pub fn new(value: impl AsRef<str>) -> Self {
        Self(canonical_source_identifier(value.as_ref()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn is(&self, expected: &str) -> bool {
        self.0 == canonical_source_identifier(expected)
    }

    #[must_use]
    pub fn canonical(value: &str) -> CompactString {
        canonical_source_identifier(value)
    }
}

impl fmt::Display for SlotId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<&str> for SlotId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for SlotId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<CompactString> for SlotId {
    fn from(value: CompactString) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegionId(CompactString);

impl RegionId {
    #[must_use]
    pub fn parse(value: impl AsRef<str>) -> Option<Self> {
        let value = value.as_ref().trim().to_ascii_lowercase();
        is_valid_region_id(&value).then(|| Self(value.into()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn is_valid_source_value(value: &str) -> bool {
        let value = value.trim().to_ascii_lowercase();
        is_valid_region_id(&value)
    }
}

impl fmt::Display for RegionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

fn is_valid_region_id(value: &str) -> bool {
    if value.is_empty()
        || value.starts_with('.')
        || value.ends_with('.')
        || value.contains("..")
        || value.contains('/')
        || value.contains('\\')
        || value.contains(':')
        || matches!(
            value.rsplit_once('.').map(|(_, extension)| extension),
            Some("rs" | "js" | "jsx" | "ts" | "tsx" | "html" | "css" | "json" | "toml")
        )
    {
        return false;
    }

    value.split('.').all(|segment| {
        !segment.is_empty()
            && segment
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    })
}

fn canonical_source_identifier(value: &str) -> CompactString {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect::<String>()
        .into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderPlan {
    pub nodes: Vec<RenderNode>,
    pub root: RenderRoot,
    pub head: Vec<RenderHeadElement>,
    pub annotations: Vec<RenderAnnotation>,
    pub state: RenderStatePlan,
    pub scripts: Vec<RenderScriptReference>,
    pub source_logic: Vec<RenderSourceLogic>,
    pub theme: RenderThemePlan,
    /// `@keyframes` and view transitions.
    pub motion: RenderMotionPlan,
}

impl RenderPlan {
    #[must_use]
    pub fn new(nodes: Vec<RenderNode>) -> Self {
        let state = RenderStatePlan::from_nodes(&nodes);
        Self {
            nodes,
            root: RenderRoot::default(),
            head: Vec::new(),
            annotations: Vec::new(),
            state,
            scripts: Vec::new(),
            source_logic: Vec::new(),
            theme: RenderThemePlan::default(),
            motion: RenderMotionPlan::default(),
        }
    }

    #[must_use]
    pub fn with_motion(mut self, motion: RenderMotionPlan) -> Self {
        self.motion = motion;
        self
    }

    #[must_use]
    pub fn with_annotations(mut self, annotations: Vec<RenderAnnotation>) -> Self {
        self.annotations = annotations;
        self
    }

    #[must_use]
    pub fn with_root(mut self, root: RenderRoot) -> Self {
        self.root = root;
        self
    }

    #[must_use]
    pub fn with_head(mut self, head: Vec<RenderHeadElement>) -> Self {
        self.head = head;
        self
    }

    #[must_use]
    pub fn with_state(mut self, state: RenderStatePlan) -> Self {
        self.state = state;
        self
    }

    #[must_use]
    pub fn with_scripts(mut self, scripts: Vec<RenderScriptReference>) -> Self {
        self.scripts = scripts;
        self
    }

    #[must_use]
    pub fn with_source_logic(mut self, source_logic: Vec<RenderSourceLogic>) -> Self {
        self.source_logic = source_logic;
        self
    }

    #[must_use]
    pub fn with_theme(mut self, theme: RenderThemePlan) -> Self {
        self.theme = theme;
        self
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderRoot {
    pub styles: Vec<StyleDeclaration>,
    pub style_variants: Vec<RenderStyleVariant>,
    pub span: Option<Span>,
}

impl RenderRoot {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.styles.is_empty() && self.style_variants.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderHeadElement {
    pub html: String,
    pub span: Option<Span>,
}

// Keyframe offsets are finite by construction, so equality is total.
impl Eq for RenderMotionPlan {}
impl Eq for RenderKeyframes {}
impl Eq for RenderKeyframe {}

/// Motion defined by stylesheets: `@keyframes` and view transitions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RenderMotionPlan {
    /// `@keyframes` rules by name; a later rule replaces an earlier one.
    pub keyframes: Vec<RenderKeyframes>,
    /// `@view-transition` and the `::view-transition-*` rules.
    pub view_transition: RenderViewTransitionPlan,
}

impl RenderMotionPlan {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keyframes.is_empty() && self.view_transition.is_empty()
    }

    /// The `@keyframes` rule with this name.
    #[must_use]
    pub fn keyframes(&self, name: &str) -> Option<&RenderKeyframes> {
        self.keyframes
            .iter()
            .rev()
            .find(|keyframes| keyframes.name == name)
    }

    pub(crate) fn extend(&mut self, other: Self) {
        for keyframes in other.keyframes {
            self.keyframes
                .retain(|existing| existing.name != keyframes.name);
            self.keyframes.push(keyframes);
        }
        self.view_transition.extend(other.view_transition);
    }
}

/// One `@keyframes` rule.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderKeyframes {
    pub name: CompactString,
    /// Keyframes sorted by offset. A selector list such as `0%, 100%` yields
    /// one keyframe per offset.
    pub frames: Vec<RenderKeyframe>,
    /// Media and supports conditions the rule was nested in.
    pub conditions: Vec<RenderStyleCondition>,
    pub span: Option<Span>,
}

/// One keyframe of a `@keyframes` rule.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderKeyframe {
    /// Offset in `0..=1`.
    pub offset: f32,
    pub declarations: Vec<StyleDeclaration>,
}

/// Where an animated property sits between two keyframes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyframeInterval<'a> {
    /// The value at the start of the interval, or `None` for the element's
    /// own value (no keyframe sets the property at or before this point).
    pub from: Option<&'a StyleDeclaration>,
    /// The value at the end of the interval, or `None` for the element's own value.
    pub to: Option<&'a StyleDeclaration>,
    /// Progress through the interval in `0..=1`, before easing.
    pub progress: f32,
    /// The keyframe that starts the interval, whose `animation-timing-function`
    /// (if any) eases it.
    pub start: Option<&'a RenderKeyframe>,
}

impl RenderKeyframes {
    /// Locate `property` (a CSS name) at `progress` in `0..=1`, following the
    /// CSS rule that keyframes which do not set a property are skipped for it.
    #[must_use]
    pub fn interval(&self, property: &str, progress: f32) -> Option<KeyframeInterval<'_>> {
        let sets = |frame: &&RenderKeyframe| {
            frame
                .declarations
                .iter()
                .any(|declaration| declaration.property.as_str() == property)
        };
        if !self.frames.iter().any(|frame| sets(&frame)) {
            return None;
        }
        fn value<'a>(frame: &'a RenderKeyframe, property: &str) -> Option<&'a StyleDeclaration> {
            frame
                .declarations
                .iter()
                .rev()
                .find(|declaration| declaration.property.as_str() == property)
        }
        let before = self
            .frames
            .iter()
            .rev()
            .find(|frame| sets(frame) && frame.offset <= progress);
        let after = self
            .frames
            .iter()
            .filter(sets)
            .find(|frame| frame.offset > progress);
        let start_offset = before.map_or(0.0, |frame| frame.offset);
        let end_offset = after.map_or(1.0, |frame| frame.offset);
        let span = end_offset - start_offset;
        Some(KeyframeInterval {
            from: before.and_then(|frame| value(frame, property)),
            to: after.and_then(|frame| value(frame, property)),
            progress: if span > 0.0 {
                ((progress - start_offset) / span).clamp(0.0, 1.0)
            } else {
                1.0
            },
            start: before,
        })
    }
}

/// `@view-transition` and the rules that style view-transition pseudo-elements.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderViewTransitionPlan {
    /// `navigation: auto` opts document replacements into a view transition.
    pub navigation: bool,
    /// `types` named by `@view-transition`.
    pub types: Vec<CompactString>,
    /// `::view-transition-*` rules in source order.
    pub rules: Vec<RenderViewTransitionRule>,
    pub span: Option<Span>,
}

impl RenderViewTransitionPlan {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.navigation && self.types.is_empty() && self.rules.is_empty()
    }

    pub(crate) fn extend(&mut self, other: Self) {
        if other.span.is_some() {
            self.navigation = other.navigation;
            self.types = other.types;
            self.span = other.span;
        }
        let offset = self.rules.len();
        self.rules.extend(other.rules.into_iter().map(|mut rule| {
            rule.source_order += offset;
            rule
        }));
    }

    /// Declarations that apply to one pseudo-element of a transition, in
    /// cascade order (lowest priority first). `name` is the captured
    /// element's `view-transition-name` (`root` for the document), `classes`
    /// its `view-transition-class` list, `active_types` the transition's
    /// types, and `condition_matches` evaluates media and supports conditions.
    #[must_use]
    pub fn declarations_for<'a>(
        &'a self,
        part: ViewTransitionPart,
        name: &str,
        classes: &[CompactString],
        active_types: &[CompactString],
        condition_matches: impl Fn(&RenderStyleCondition) -> bool,
    ) -> Vec<&'a StyleDeclaration> {
        let mut matched: Vec<&RenderViewTransitionRule> = self
            .rules
            .iter()
            .filter(|rule| rule.part == part)
            .filter(|rule| rule.name.matches(name))
            .filter(|rule| rule.classes.iter().all(|class| classes.contains(class)))
            .filter(|rule| {
                rule.types.is_empty() || rule.types.iter().any(|kind| active_types.contains(kind))
            })
            .filter(|rule| rule.conditions.iter().all(&condition_matches))
            .collect();
        matched.sort_by_key(|rule| (rule.specificity, rule.source_order));
        let normal = matched
            .iter()
            .flat_map(|rule| rule.declarations.iter())
            .filter(|declaration| !declaration.important);
        let important = matched
            .iter()
            .flat_map(|rule| rule.declarations.iter())
            .filter(|declaration| declaration.important);
        normal.chain(important).collect()
    }
}

/// A view-transition pseudo-element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewTransitionPart {
    /// `::view-transition`, the overlay root.
    Overlay,
    /// `::view-transition-group(name)`, which animates position and size.
    Group,
    /// `::view-transition-image-pair(name)`.
    ImagePair,
    /// `::view-transition-old(name)`, the outgoing capture.
    Old,
    /// `::view-transition-new(name)`, the incoming capture.
    New,
}

/// The name argument of a view-transition pseudo-element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewTransitionName {
    /// `*`, or no name (classes only).
    Any,
    /// A `view-transition-name`; `root` names the document.
    Named(CompactString),
}

impl ViewTransitionName {
    #[must_use]
    pub fn matches(&self, name: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Named(named) => named == name,
        }
    }
}

/// One `::view-transition-*` rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderViewTransitionRule {
    pub part: ViewTransitionPart,
    pub name: ViewTransitionName,
    /// `view-transition-class` names the captured element must all have.
    pub classes: Vec<CompactString>,
    /// `:active-view-transition-type(...)` on the originating element; the
    /// rule applies when the transition has any of them. Empty applies always.
    pub types: Vec<CompactString>,
    /// Media and supports conditions.
    pub conditions: Vec<RenderStyleCondition>,
    pub declarations: Vec<StyleDeclaration>,
    pub specificity: u32,
    pub source_order: usize,
    pub selector: CompactString,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderThemePlan {
    pub tokens: Vec<RenderThemeToken>,
}

impl RenderThemePlan {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    #[must_use]
    pub fn token(&self, name: &str) -> Option<&RenderThemeToken> {
        self.tokens.iter().find(|token| token.name == name)
    }

    pub fn push_token_value(
        &mut self,
        name: impl Into<CompactString>,
        scope: RenderThemeScope,
        value: StyleValue,
        span: Option<Span>,
    ) {
        let name = name.into();
        let kind = ThemeTokenKind::from_name_and_value(&name, &value);
        let value = RenderThemeTokenValue { scope, value, span };

        if let Some(token) = self.tokens.iter_mut().find(|token| token.name == name) {
            token.kind = token.kind.merge(kind);
            token.values.push(value);
            return;
        }

        self.tokens.push(RenderThemeToken {
            name,
            kind,
            values: vec![value],
            referenced: false,
        });
    }

    pub fn mark_referenced(&mut self, name: &str) {
        if let Some(token) = self.tokens.iter_mut().find(|token| token.name == name) {
            token.referenced = true;
            return;
        }

        self.tokens.push(RenderThemeToken {
            name: CompactString::from(name),
            kind: ThemeTokenKind::Other,
            values: Vec::new(),
            referenced: true,
        });
    }

    pub fn collect_from_nodes(&mut self, nodes: &[RenderNode]) {
        for node in nodes {
            self.collect_from_node(node);
        }
    }

    pub fn collect_from_root(&mut self, root: &RenderRoot) {
        for style in &root.styles {
            self.collect_theme_references(style);
        }
        for variant in &root.style_variants {
            for declaration in &variant.declarations {
                self.collect_theme_references(declaration);
            }
        }
    }

    fn collect_from_node(&mut self, node: &RenderNode) {
        let RenderNode::Element(element) = node else {
            return;
        };

        self.collect_from_element(element);
    }

    fn collect_from_element(&mut self, element: &RenderElement) {
        for style in &element.styles {
            self.collect_from_declaration(
                style,
                RenderThemeScope::InlineElement {
                    element_span: element.span,
                },
            );
        }

        for variant in &element.style_variants {
            for declaration in &variant.declarations {
                self.collect_theme_references(declaration);
            }
        }

        for pseudo in &element.pseudo_elements {
            for declaration in &pseudo.styles {
                self.collect_theme_references(declaration);
            }
        }

        for child in &element.children {
            self.collect_from_node(child);
        }
    }

    fn collect_from_declaration(&mut self, style: &StyleDeclaration, scope: RenderThemeScope) {
        if let StyleProperty::Custom(name) = &style.property {
            self.push_token_value(
                name.clone(),
                scope,
                StyleValue::from_theme_raw(style.value.as_str(), style.span),
                style.span,
            );
        }

        self.collect_theme_references(style);
    }

    fn collect_theme_references(&mut self, style: &StyleDeclaration) {
        for token in style.value.tokens_with_span(style.span) {
            self.mark_referenced(&token.name);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderThemeToken {
    pub name: CompactString,
    pub kind: ThemeTokenKind,
    pub values: Vec<RenderThemeTokenValue>,
    pub referenced: bool,
}

impl RenderThemeToken {
    #[must_use]
    pub fn latest_root_value(&self) -> Option<&RenderThemeTokenValue> {
        self.values
            .iter()
            .rev()
            .find(|value| value.scope == RenderThemeScope::Root)
    }

    #[must_use]
    pub fn latest_unconditional_value(&self) -> Option<&RenderThemeTokenValue> {
        self.values.iter().rev().find(|value| {
            matches!(
                value.scope,
                RenderThemeScope::Root
                    | RenderThemeScope::Selector(_)
                    | RenderThemeScope::InlineElement { .. }
            )
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderThemeTokenValue {
    pub scope: RenderThemeScope,
    pub value: StyleValue,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderThemeScope {
    Root,
    Selector(CompactString),
    Conditional {
        selector: Option<CompactString>,
        conditions: Vec<RenderStyleCondition>,
    },
    InlineElement {
        element_span: Option<Span>,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThemeTokenKind {
    Color,
    Length,
    Number,
    FontFamily,
    Shadow,
    #[default]
    Other,
}

impl ThemeTokenKind {
    #[must_use]
    pub fn from_name_and_value(name: &str, value: &StyleValue) -> Self {
        let normalized = normalize_theme_token_name(name);
        if normalized.contains("font_family") || normalized == "font" {
            return Self::FontFamily;
        }
        if normalized.contains("shadow") {
            return Self::Shadow;
        }
        if normalized.contains("opacity")
            || normalized.contains("index")
            || normalized.contains("scale")
        {
            return Self::Number;
        }
        if normalized.contains("radius")
            || normalized.contains("space")
            || normalized.contains("gap")
            || normalized.contains("padding")
            || normalized.contains("margin")
            || normalized.ends_with("width")
            || normalized.ends_with("height")
            || normalized.ends_with("size")
        {
            return Self::Length;
        }
        if normalized.contains("color")
            || normalized.contains("background")
            || normalized.contains("foreground")
            || normalized.contains("border")
            || normalized.contains("ring")
            || matches!(
                normalized.as_str(),
                "primary"
                    | "secondary"
                    | "accent"
                    | "muted"
                    | "destructive"
                    | "danger"
                    | "success"
                    | "warning"
                    | "info"
                    | "link"
                    | "input"
                    | "popover"
                    | "selection"
            )
        {
            return Self::Color;
        }

        match value {
            StyleValue::Color(_) => Self::Color,
            StyleValue::Length(_) => Self::Length,
            _ => Self::Other,
        }
    }

    const fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Other, other) => other,
            (current, Self::Other) => current,
            (current, _) => current,
        }
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderScriptReference {
    pub src: CompactString,
    pub kind: RenderScriptKind,
    pub async_script: bool,
    pub defer: bool,
    pub span: Option<Span>,
    pub resolved_source: Option<SourceId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderScriptKind {
    Classic,
    Module,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderSourceLogic {
    pub dialect: CompactString,
    pub script_type: Option<CompactString>,
    pub body: ArcStr,
    pub data_props: Option<CompactString>,
    pub component: Option<RenderSourceComponentLogic>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderSourceComponentLogic {
    pub items: Vec<RenderSourceLogicItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderSourceLogicItem {
    Local(RenderSourceLocal),
    State(RenderSourceState),
    Derived(RenderSourceDerived),
    Ref(RenderSourceRef),
    Callback(RenderSourceCallback),
    Mount(RenderSourceMount),
    Effect(RenderSourceEffect),
    Snippet(RenderSourceSnippet),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderSourceLocal {
    pub body: ArcStr,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderSourceState {
    pub name: CompactString,
    pub setter: CompactString,
    pub initial: CompactString,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderSourceDerived {
    pub name: CompactString,
    pub body: CompactString,
    pub by: bool,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderSourceRef {
    pub name: CompactString,
    pub initial: CompactString,
    pub dom: bool,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderSourceCallback {
    pub name: CompactString,
    pub body: CompactString,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderSourceMount {
    pub body: CompactString,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderSourceEffect {
    pub dependencies: Vec<CompactString>,
    pub body: CompactString,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderSourceSnippet {
    pub name: CompactString,
    pub params: Vec<CompactString>,
    pub nodes: Vec<RenderNode>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderNode {
    Element(Box<RenderElement>),
    Text(RenderText),
    Raw(RenderRaw),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderElement {
    pub role: UiRole,
    pub source_tag: CompactString,
    pub attributes: Vec<RenderAttribute>,
    pub classes: Vec<CompactString>,
    /// Inline declarations as authored on the source element. `styles` contains
    /// the resolved cascade; source-generating adapters use this provenance to
    /// preserve declarations that a stylesheet `!important` rule suppresses.
    pub source_inline_styles: Vec<StyleDeclaration>,
    pub styles: Vec<StyleDeclaration>,
    pub stylesheet_rules: Vec<RenderStylesheetRule>,
    pub stylesheet_declarations: Vec<StyleDeclaration>,
    pub style_variants: Vec<RenderStyleVariant>,
    pub dynamic_styles: Vec<RenderDynamicStyleBinding>,
    pub pseudo_elements: Vec<RenderPseudoElement>,
    pub actions: Vec<ActionBinding>,
    pub state: Option<Box<RenderStateBinding>>,
    pub form_control: Option<Box<RenderFormControl>>,
    pub accessibility: Option<Box<RenderAccessibility>>,
    pub control_flow: Option<Box<RenderControlFlow>>,
    pub source_intent: Option<Box<RenderSourceIntent>>,
    pub semantics: Option<Box<RenderSemantics>>,
    pub region: Option<RegionId>,
    pub children: Vec<RenderNode>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderAttribute {
    pub name: CompactString,
    pub value: CompactString,
    pub template: Option<TemplateString>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderDynamicStyleBinding {
    pub state: Option<CompactString>,
    pub expression: TemplateString,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderControlFlow {
    pub kind: RenderControlFlowKind,
    pub host: RenderControlFlowHost,
    pub expression: Option<Expr>,
    pub binding: Option<BindingPattern>,
    pub index_binding: Option<BindingPattern>,
    pub key: Option<Expr>,
    pub locals: Vec<RenderLoopLocal>,
    pub placeholder: Option<CompactString>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderLoopLocal {
    pub name: CompactString,
    pub value: Expr,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderControlFlowHost {
    Wrapper,
    Element,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderControlFlowKind {
    For,
    If,
    ElseIf,
    Else,
    Switch,
    Case,
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderText {
    pub value: String,
    pub template: Option<TemplateString>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderRaw {
    pub html: String,
    pub span: Option<Span>,
}

/// Source-only metadata that adapters may emit as target-language comments or
/// consume for code-generation decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderAnnotation {
    pub kind: RenderAnnotationKind,
    pub value: String,
    pub span: Option<Span>,
}

impl RenderAnnotation {
    #[must_use]
    pub fn new(kind: RenderAnnotationKind, value: impl Into<String>, span: Option<Span>) -> Self {
        Self {
            kind,
            value: value.into(),
            span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderAnnotationKind {
    HtmlComment,
    CssComment,
    JavaScriptLineComment,
    JavaScriptBlockComment,
    SourceMetadata,
}

impl RenderAnnotationKind {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::HtmlComment => "html-comment",
            Self::CssComment => "css-comment",
            Self::JavaScriptLineComment => "js-line-comment",
            Self::JavaScriptBlockComment => "js-block-comment",
            Self::SourceMetadata => "source-metadata",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionBinding {
    pub event: CompactString,
    pub expression: String,
    pub template: Option<TemplateString>,
    pub action: Option<CompactString>,
    pub resolved: bool,
    pub handler: RenderActionHandler,
    pub payload: ActionPayload,
    pub span: Option<Span>,
    pub action_span: Option<Span>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderActionHandler {
    pub invocations: Vec<RenderActionInvocation>,
    pub effects: Vec<RenderActionHandlerEffect>,
}

impl RenderActionHandler {
    #[must_use]
    pub fn primary_invocation(&self) -> Option<&RenderActionInvocation> {
        self.invocations.first()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.invocations.is_empty() && self.effects.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderActionInvocation {
    pub action: CompactString,
    pub arguments: Vec<RenderActionArgument>,
    pub resolved: bool,
    pub span: Option<Span>,
    pub action_span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderActionArgument {
    Event,
    Element,
    ElementValue,
    ElementChecked,
    Literal(CompactString),
    Unknown(CompactString),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderActionHandlerEffect {
    PreventDefault { span: Option<Span> },
    StopPropagation { span: Option<Span> },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ActionPayload {
    #[default]
    None,
    ElementState {
        state_id: CompactString,
    },
    FormData {
        controls: Vec<RenderFormDataField>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderFormDataField {
    pub name: CompactString,
    pub state_id: Option<CompactString>,
    pub value: Option<CompactString>,
    pub control_type: RenderFormControlType,
}

/// Canonical target-neutral state graph for adapters that need owned target
/// state, entity initialization, or form-level state aggregation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderStatePlan {
    pub bindings: Vec<RenderStateBinding>,
    pub forms: Vec<RenderFormBinding>,
    pub actions: Vec<RenderActionPlan>,
}

impl RenderStatePlan {
    #[must_use]
    pub fn from_nodes(nodes: &[RenderNode]) -> Self {
        let mut builder = RenderStatePlanBuilder::default();
        for node in nodes {
            builder.collect_node(node);
        }
        Self {
            bindings: builder.bindings,
            forms: builder.forms,
            actions: builder.actions,
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty() && self.forms.is_empty() && self.actions.is_empty()
    }

    #[must_use]
    pub fn binding(&self, id: &str) -> Option<&RenderStateBinding> {
        self.bindings.iter().find(|binding| binding.id == id)
    }

    #[must_use]
    pub fn form(&self, id: &str) -> Option<&RenderFormBinding> {
        self.forms.iter().find(|form| form.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderFormBinding {
    pub id: CompactString,
    pub controls: Vec<RenderFormControlBinding>,
    pub validation: RenderValidation,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderFormControlBinding {
    pub state_id: Option<CompactString>,
    pub control: RenderFormControl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderActionPlan {
    pub event: CompactString,
    pub action: Option<CompactString>,
    pub effects: Vec<RenderActionEffect>,
    pub span: Option<Span>,
    pub action_span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderActionEffect {
    PreventDefault,
    StopPropagation,
    Invoke {
        action: CompactString,
    },
    UpdateState {
        state_id: CompactString,
        source: RenderStateValueSource,
    },
    SubmitForm(RenderFormSubmit),
    ValidateForm {
        form_id: CompactString,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderFormSubmit {
    pub form_id: Option<CompactString>,
    pub controls: Vec<RenderFormDataField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderStateValueSource {
    EventValue,
    EventChecked,
    FormField { name: CompactString },
    Static(CompactString),
}

/// Target-neutral state allocated for elements whose runtime value is owned by
/// the generated target UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderStateBinding {
    pub id: CompactString,
    pub owner: RenderStateOwner,
    pub kind: RenderStateKind,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RenderStateOwner {
    #[default]
    Target,
    Source,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderStateKind {
    TextInput(RenderTextInputState),
    Choice(RenderChoiceState),
    Toggle(RenderToggleState),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderTextInputState {
    pub input_type: CompactString,
    pub initial_value: Option<CompactString>,
    pub initial_template: Option<TemplateString>,
    pub placeholder: Option<CompactString>,
    pub placeholder_template: Option<TemplateString>,
    pub multiline: bool,
    pub rows: Option<u16>,
    pub password: bool,
    pub disabled: bool,
    pub readonly: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderChoiceState {
    pub options: Vec<RenderChoiceOption>,
    pub selected_index: Option<usize>,
    pub multiple: bool,
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderChoiceOption {
    pub label: CompactString,
    pub value: CompactString,
    pub disabled: bool,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderToggleState {
    pub checked: bool,
    pub disabled: bool,
}

/// Form metadata that lets adapters map HTML controls into target-specific
/// field abstractions without re-parsing sibling labels and attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderFormControl {
    pub id: Option<CompactString>,
    pub control_type: RenderFormControlType,
    pub name: Option<CompactString>,
    pub value: Option<CompactString>,
    pub group: Option<CompactString>,
    pub label: Option<CompactString>,
    pub options: Vec<RenderChoiceOption>,
    pub required: bool,
    pub disabled: bool,
    pub readonly: bool,
    pub validation: RenderValidation,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderFormControlType {
    Text,
    TextArea,
    Select,
    Checkbox,
    Radio,
    Button,
    Submit,
    Reset,
    Fieldset,
    Unknown(CompactString),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderValidation {
    pub required: bool,
    pub disabled: bool,
    pub readonly: bool,
    pub constraints: Vec<RenderValidationConstraint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderValidationConstraint {
    MinLength(u32),
    MaxLength(u32),
    Min(CompactString),
    Max(CompactString),
    Step(CompactString),
    Pattern(CompactString),
    InputType(CompactString),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderPseudoElement {
    pub kind: CompactString,
    pub selector: CompactString,
    pub conditions: Vec<RenderStyleCondition>,
    pub styles: Vec<StyleDeclaration>,
    pub children: Vec<RenderNode>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderAccessibility {
    pub role: Option<CompactString>,
    pub label: Option<CompactString>,
    pub labelled_by: Option<CompactString>,
    pub described_by: Option<CompactString>,
    pub tab_index: Option<isize>,
    pub autofocus: bool,
    pub hidden: bool,
    pub aria: Vec<RenderAttribute>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RenderSemantics {
    pub variant: Option<RenderVariant>,
    pub tone: Option<RenderTone>,
    pub size: Option<RenderSize>,
    pub density: Option<RenderDensity>,
    pub extras: Vec<RenderSemanticExtra>,
}

impl RenderSemantics {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.variant.is_none()
            && self.tone.is_none()
            && self.size.is_none()
            && self.density.is_none()
            && self.extras.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderSemanticExtra {
    pub axis: CompactString,
    pub value: CompactString,
    pub span: Option<Span>,
}

macro_rules! semantic_enum {
    (
        $name:ident {
            $($variant:ident => $canonical:literal [$($alias:literal),*]),+ $(,)?
        }
    ) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum $name {
            $($variant,)+
            Custom(CompactString),
        }

        impl $name {
            #[must_use]
            pub fn parse(value: &str) -> Self {
                let value = normalized_semantic_token(value);
                match value.as_str() {
                    $($canonical $(| $alias)* => Self::$variant,)+
                    _ => Self::Custom(value.into()),
                }
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$variant => $canonical,)+
                    Self::Custom(value) => value.as_str(),
                }
            }
        }
    };
}

semantic_enum!(RenderVariant {
    Solid => "solid" ["filled", "fill"],
    Outline => "outline" ["outlined"],
    Ghost => "ghost" [],
    Soft => "soft" ["subtle"],
    Link => "link" [],
    Text => "text" ["plain"],
});

semantic_enum!(RenderTone {
    Neutral => "neutral" ["default"],
    Accent => "accent" ["primary", "brand"],
    Success => "success" ["positive"],
    Warning => "warning" ["warn", "caution"],
    Danger => "danger" ["destructive", "error", "negative"],
    Info => "info" ["informative"],
});

semantic_enum!(RenderSize {
    Xs => "xs" ["extra-small", "x-small"],
    Sm => "sm" ["small"],
    Md => "md" ["medium", "default"],
    Lg => "lg" ["large"],
    Xl => "xl" ["extra-large", "x-large"],
});

semantic_enum!(RenderDensity {
    Compact => "compact" ["dense"],
    Comfortable => "comfortable" ["default"],
    Spacious => "spacious" ["loose"],
});

fn normalized_semantic_token(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('_', "-")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderSourceIntent {
    pub key: Option<CompactString>,
    pub state_id: Option<CompactString>,
    pub component: Option<ComponentId>,
    pub component_source: Option<CompactString>,
    pub slot: Option<SlotId>,
    pub child_strategy: Option<CompactString>,
    pub props: Vec<RenderSourceProp>,
}

impl RenderSourceIntent {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.key.is_none()
            && self.state_id.is_none()
            && self.component.is_none()
            && self.component_source.is_none()
            && self.slot.is_none()
            && self.child_strategy.is_none()
            && self.props.is_empty()
    }
}

#[derive(Default)]
struct RenderStatePlanBuilder {
    bindings: Vec<RenderStateBinding>,
    forms: Vec<RenderFormBinding>,
    actions: Vec<RenderActionPlan>,
    used_state_ids: BTreeSet<CompactString>,
    used_form_ids: BTreeSet<CompactString>,
    next_form: usize,
}

impl RenderStatePlanBuilder {
    fn collect_node(&mut self, node: &RenderNode) {
        let RenderNode::Element(element) = node else {
            return;
        };

        self.collect_element(element);
    }

    fn collect_element(&mut self, element: &RenderElement) {
        if let Some(state) = &element.state
            && self.used_state_ids.insert(state.id.clone())
        {
            self.bindings.push(state.as_ref().clone());
        }

        let form_id = if element.role == UiRole::Form || element.source_tag == "form" {
            let id = self.form_id_for_element(element);
            let controls = form_control_bindings(&element.children);
            let validation = validation_for_form_controls(&controls);
            self.forms.push(RenderFormBinding {
                id: id.clone(),
                controls,
                validation,
                span: element.span,
            });
            Some(id)
        } else {
            None
        };

        self.collect_action_plans(element, form_id.as_ref());

        for child in &element.children {
            self.collect_node(child);
        }
    }

    fn form_id_for_element(&mut self, element: &RenderElement) -> CompactString {
        let seed = attribute_value(element, "data-htmlswap-form")
            .or_else(|| attribute_value(element, "id"))
            .or_else(|| attribute_value(element, "name"))
            .map(str::to_owned)
            .unwrap_or_else(|| {
                let id = format!("form_{}", self.next_form);
                self.next_form += 1;
                id
            });
        let base = sanitize_plan_id(&seed, "form");

        if self.used_form_ids.insert(base.clone()) {
            return base;
        }

        let mut suffix = 2;
        loop {
            let candidate = CompactString::from(format!("{base}_{suffix}"));
            if self.used_form_ids.insert(candidate.clone()) {
                return candidate;
            }
            suffix += 1;
        }
    }

    fn collect_action_plans(&mut self, element: &RenderElement, form_id: Option<&CompactString>) {
        for action in &element.actions {
            let mut effects = Vec::new();
            for effect in &action.handler.effects {
                match effect {
                    RenderActionHandlerEffect::PreventDefault { .. } => {
                        effects.push(RenderActionEffect::PreventDefault);
                    }
                    RenderActionHandlerEffect::StopPropagation { .. } => {
                        effects.push(RenderActionEffect::StopPropagation);
                    }
                }
            }

            match &action.payload {
                ActionPayload::None => {}
                ActionPayload::ElementState { state_id } => {
                    effects.push(RenderActionEffect::UpdateState {
                        state_id: state_id.clone(),
                        source: state_value_source_for_element(element),
                    });
                }
                ActionPayload::FormData { controls } => {
                    if let Some(form_id) = form_id {
                        effects.push(RenderActionEffect::ValidateForm {
                            form_id: form_id.clone(),
                        });
                    }
                    effects.push(RenderActionEffect::SubmitForm(RenderFormSubmit {
                        form_id: form_id.cloned(),
                        controls: controls.clone(),
                    }));
                }
            }

            if action.resolved
                && let Some(action_name) = &action.action
            {
                effects.push(RenderActionEffect::Invoke {
                    action: action_name.clone(),
                });
            }

            if !effects.is_empty() {
                self.actions.push(RenderActionPlan {
                    event: action.event.clone(),
                    action: action.action.clone(),
                    effects,
                    span: action.span,
                    action_span: action.action_span,
                });
            }
        }
    }
}

fn form_control_bindings(nodes: &[RenderNode]) -> Vec<RenderFormControlBinding> {
    let mut controls = Vec::new();
    collect_form_control_bindings(nodes, &mut controls);
    controls
}

fn validation_for_form_controls(controls: &[RenderFormControlBinding]) -> RenderValidation {
    let mut validation = RenderValidation::default();
    for control in controls {
        validation.required |= control.control.validation.required;
        validation.disabled |= control.control.validation.disabled;
        validation.readonly |= control.control.validation.readonly;
        validation
            .constraints
            .extend(control.control.validation.constraints.iter().cloned());
    }
    validation
}

fn state_value_source_for_element(element: &RenderElement) -> RenderStateValueSource {
    if let Some(control) = &element.form_control {
        match control.control_type {
            RenderFormControlType::Checkbox | RenderFormControlType::Radio => {
                return RenderStateValueSource::EventChecked;
            }
            _ => {}
        }
    }

    RenderStateValueSource::EventValue
}

fn collect_form_control_bindings(
    nodes: &[RenderNode],
    controls: &mut Vec<RenderFormControlBinding>,
) {
    for node in nodes {
        let RenderNode::Element(element) = node else {
            continue;
        };

        if element.role == UiRole::Form || element.source_tag == "form" {
            continue;
        }

        if let Some(control) = &element.form_control {
            controls.push(RenderFormControlBinding {
                state_id: element.state.as_ref().map(|state| state.id.clone()),
                control: control.as_ref().clone(),
            });
        }

        collect_form_control_bindings(&element.children, controls);
    }
}

fn attribute_value<'a>(element: &'a RenderElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.as_str().eq_ignore_ascii_case(name))
        .map(|attribute| attribute.value.as_str())
}

fn sanitize_plan_id(value: &str, fallback_prefix: &str) -> CompactString {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderSourceProp {
    pub name: CompactString,
    pub value: CompactString,
    pub template: Option<TemplateString>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderStyleVariant {
    pub conditions: Vec<RenderStyleCondition>,
    pub selector: CompactString,
    pub declarations: Vec<StyleDeclaration>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderStylesheetRule {
    pub source_order: usize,
    pub selector: CompactString,
    pub conditions: Vec<RenderStyleCondition>,
    pub declarations: Vec<StyleDeclaration>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderStyleCondition {
    PseudoClass(CompactString),
    PseudoElement(CompactString),
    Media(CompactString),
    Supports(CompactString),
    Container(CompactString),
    /// Inside `@starting-style`: the values an element starts from the first
    /// time it is styled, so transitions can animate its entry.
    StartingStyle,
    /// `:active-view-transition-type(...)`: matches while a view transition
    /// with any of these types is active.
    ActiveViewTransitionType(Vec<CompactString>),
    /// A state pseudo-class written on an ancestor compound, or negated:
    /// `.card:hover .title` styles `.title` while its ancestor `ancestor`
    /// levels up is hovered; `:not(:hover)` styles an element while it is
    /// not. A state on the subject itself, not negated, is
    /// [`RenderStyleCondition::PseudoClass`].
    ElementState {
        pseudo: CompactString,
        /// Levels above the styled element: 1 is its parent.
        ancestor: u16,
        negated: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiRole {
    Container,
    Inline,
    Paragraph,
    Button,
    TextInput,
    Select,
    Option,
    Link,
    Image,
    Heading(u8),
    List { ordered: bool },
    ListItem,
    Form,
    Fieldset,
    Legend,
    Label,
    Unknown,
}

impl UiRole {
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Container => "container",
            Self::Inline => "inline",
            Self::Paragraph => "paragraph",
            Self::Button => "button",
            Self::TextInput => "text-input",
            Self::Select => "select",
            Self::Option => "option",
            Self::Link => "link",
            Self::Image => "image",
            Self::Heading(_) => "heading",
            Self::List { ordered: false } => "list",
            Self::List { ordered: true } => "ordered-list",
            Self::ListItem => "list-item",
            Self::Form => "form",
            Self::Fieldset => "fieldset",
            Self::Legend => "legend",
            Self::Label => "label",
            Self::Unknown => "unknown",
        }
    }
}
