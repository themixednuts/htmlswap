use std::collections::{BTreeMap, BTreeSet};

use crate::adapter::{RouteTarget, RouteTargetRef};
use crate::plan::{
    ActionBinding, RenderDynamicStyleBinding, RenderElement, RenderNode, RenderPlan,
    RenderPseudoElement, RenderScriptReference, RenderStatePlan, RenderStyleVariant,
    RenderThemePlan,
};
use crate::source::{SourceId, SourceMap, Span};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BundlePlan {
    pub units: Vec<OutputUnit>,
    pub edges: Vec<OutputDependency>,
}

impl BundlePlan {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_unit(&mut self, unit: OutputUnit) {
        self.units.push(unit);
    }

    pub fn push_edge(&mut self, edge: OutputDependency) {
        self.edges.push(edge);
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.units.is_empty() && self.edges.is_empty()
    }

    #[must_use]
    pub fn from_render_plan(plan: &RenderPlan, sources: &SourceMap) -> Self {
        Self::from_render_plan_with_config(plan, sources, &BundleConfig::default())
    }

    #[must_use]
    pub fn from_render_plan_with_config(
        plan: &RenderPlan,
        sources: &SourceMap,
        config: &BundleConfig,
    ) -> Self {
        OutputPlanBuilder::new(sources, config).build(plan)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OutputUnitId(u32);

impl OutputUnitId {
    #[must_use]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputUnit {
    pub id: OutputUnitId,
    pub kind: OutputUnitKind,
    pub boundary: OutputBoundary,
    pub source: OutputSource,
}

impl OutputUnit {
    #[must_use]
    pub fn new(
        id: OutputUnitId,
        kind: OutputUnitKind,
        boundary: OutputBoundary,
        source: OutputSource,
    ) -> Self {
        Self {
            id,
            kind,
            boundary,
            source,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OutputUnitKind {
    View,
    Component,
    Style,
    Script,
    Asset,
    State,
    Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputBoundary {
    Inline,
    SameModule,
    SeparateModule { path_hint: Option<String> },
    External { specifier: String },
}

impl OutputBoundary {
    #[must_use]
    pub fn from_hint(value: &str) -> Self {
        let value = value.trim();
        if value.eq_ignore_ascii_case("inline") {
            return Self::Inline;
        }
        if matches!(
            value.to_ascii_lowercase().as_str(),
            "same" | "same-module" | "same_module"
        ) {
            return Self::SameModule;
        }
        if let Some(specifier) = value.strip_prefix("external:") {
            return Self::External {
                specifier: specifier.trim().to_owned(),
            };
        }

        Self::SeparateModule {
            path_hint: (!value.is_empty()).then(|| value.to_owned()),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BundleConfig {
    pub boundaries: Vec<OutputBoundaryRule>,
}

impl BundleConfig {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_boundary(
        mut self,
        target: RouteTarget,
        kind: OutputUnitKind,
        boundary: OutputBoundary,
    ) -> Self {
        self.boundaries
            .push(OutputBoundaryRule::new(target, kind, boundary));
        self
    }

    fn boundary_for(
        &self,
        kind: OutputUnitKind,
        targets: &[RouteTargetRef<'_>],
    ) -> Option<OutputBoundary> {
        self.boundaries
            .iter()
            .rev()
            .find(|rule| {
                rule.kind == kind
                    && targets
                        .iter()
                        .any(|target| rule.target.matches_ref(*target))
            })
            .map(|rule| rule.boundary.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputBoundaryRule {
    pub target: RouteTarget,
    pub kind: OutputUnitKind,
    pub boundary: OutputBoundary,
}

impl OutputBoundaryRule {
    #[must_use]
    pub fn new(target: RouteTarget, kind: OutputUnitKind, boundary: OutputBoundary) -> Self {
        Self {
            target,
            kind,
            boundary,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutputSource {
    pub sources: Vec<SourceId>,
}

impl OutputSource {
    #[must_use]
    pub fn new(sources: impl Into<Vec<SourceId>>) -> Self {
        Self {
            sources: sources.into(),
        }
    }

    #[must_use]
    pub fn single(source: SourceId) -> Self {
        Self {
            sources: vec![source],
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    pub fn extend(&mut self, sources: impl IntoIterator<Item = SourceId>) {
        let mut merged = self.sources.iter().copied().collect::<BTreeSet<_>>();
        merged.extend(sources);
        self.sources = merged.into_iter().collect();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputDependency {
    pub from: OutputUnitId,
    pub to: OutputUnitId,
}

impl OutputDependency {
    #[must_use]
    pub const fn new(from: OutputUnitId, to: OutputUnitId) -> Self {
        Self { from, to }
    }
}

#[derive(Debug)]
struct OutputPlanBuilder<'a> {
    plan: BundlePlan,
    sources: &'a SourceMap,
    config: &'a BundleConfig,
    view: OutputUnitId,
    units: BTreeMap<OutputUnitKey, OutputUnitId>,
}

impl<'a> OutputPlanBuilder<'a> {
    fn new(sources: &'a SourceMap, config: &'a BundleConfig) -> Self {
        Self {
            plan: BundlePlan::new(),
            sources,
            config,
            view: OutputUnitId::new(0),
            units: BTreeMap::new(),
        }
    }

    fn build(mut self, render_plan: &RenderPlan) -> BundlePlan {
        self.push_view();
        self.collect_state(&render_plan.state);
        self.collect_theme(&render_plan.theme);
        for script in &render_plan.scripts {
            self.collect_script(script);
        }
        for node in &render_plan.nodes {
            self.collect_node(node);
        }
        self.plan
    }

    fn push_view(&mut self) {
        self.push_unit(
            OutputUnitKind::View,
            OutputBoundary::SameModule,
            OutputSource::new(
                self.sources
                    .files()
                    .iter()
                    .map(|source| source.id())
                    .collect::<Vec<_>>(),
            ),
        );
    }

    fn collect_state(&mut self, state: &RenderStatePlan) {
        if state.is_empty() {
            return;
        }

        let mut spans = Vec::new();
        spans.extend(state.bindings.iter().filter_map(|binding| binding.span));
        spans.extend(state.forms.iter().filter_map(|form| form.span));
        spans.extend(
            state
                .actions
                .iter()
                .flat_map(|action| [action.span, action.action_span].into_iter().flatten()),
        );

        let id = self.push_unit(
            OutputUnitKind::State,
            OutputBoundary::SameModule,
            source_from_spans(spans),
        );
        self.push_edge(self.view, id);
    }

    fn collect_theme(&mut self, theme: &RenderThemePlan) {
        if theme.is_empty() {
            return;
        }

        let spans = theme
            .tokens
            .iter()
            .flat_map(|token| token.values.iter().filter_map(|value| value.span));
        let id = self.push_unit(
            OutputUnitKind::Style,
            OutputBoundary::SameModule,
            source_from_spans(spans),
        );
        self.push_edge(self.view, id);
    }

    fn collect_script(&mut self, script: &RenderScriptReference) {
        let boundary = if script.resolved_source.is_some() {
            OutputBoundary::SeparateModule {
                path_hint: Some(script.src.to_string()),
            }
        } else if is_external_specifier(&script.src) {
            OutputBoundary::External {
                specifier: script.src.to_string(),
            }
        } else {
            OutputBoundary::SeparateModule {
                path_hint: Some(script.src.to_string()),
            }
        };
        let source = script
            .resolved_source
            .map_or_else(|| source_from_spans(script.span), OutputSource::single);
        let id = self.push_unit(OutputUnitKind::Script, boundary, source);
        self.push_edge(self.view, id);
    }

    fn collect_node(&mut self, node: &RenderNode) {
        match node {
            RenderNode::Element(element) => self.collect_element(element),
            RenderNode::Text(_) | RenderNode::Raw(_) => {}
        }
    }

    fn collect_element(&mut self, element: &RenderElement) {
        let targets = bundle_targets(element);
        if let Some(boundary) = self
            .config
            .boundary_for(OutputUnitKind::Component, &targets)
        {
            let id = self.push_unit(
                OutputUnitKind::Component,
                boundary,
                source_from_spans(element.span),
            );
            self.push_edge(self.view, id);
        }

        if element_has_styles(element) {
            let boundary = self
                .config
                .boundary_for(OutputUnitKind::Style, &targets)
                .unwrap_or(OutputBoundary::SameModule);
            let id = self.push_unit(OutputUnitKind::Style, boundary, style_source(element));
            self.push_edge(self.view, id);
        }

        if !element.actions.is_empty() {
            let boundary = self
                .config
                .boundary_for(OutputUnitKind::Action, &targets)
                .unwrap_or(OutputBoundary::SameModule);
            let id = self.push_unit(
                OutputUnitKind::Action,
                boundary,
                action_source(&element.actions),
            );
            self.push_edge(self.view, id);
        }

        for child in &element.children {
            self.collect_node(child);
        }
    }

    fn push_unit(
        &mut self,
        kind: OutputUnitKind,
        boundary: OutputBoundary,
        source: OutputSource,
    ) -> OutputUnitId {
        let key = OutputUnitKey::new(kind, &boundary);
        if let Some(id) = self.units.get(&key).copied() {
            if let Some(unit) = self.plan.units.get_mut(id.index()) {
                unit.source.extend(source.sources);
            }
            return id;
        }

        let id = OutputUnitId::new(self.plan.units.len() as u32);
        self.plan
            .push_unit(OutputUnit::new(id, kind, boundary, source));
        self.units.insert(key, id);
        id
    }

    fn push_edge(&mut self, from: OutputUnitId, to: OutputUnitId) {
        if from == to {
            return;
        }

        let edge = OutputDependency::new(from, to);
        if !self.plan.edges.contains(&edge) {
            self.plan.push_edge(edge);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct OutputUnitKey {
    kind: OutputUnitKind,
    boundary: OutputBoundaryKey,
}

impl OutputUnitKey {
    fn new(kind: OutputUnitKind, boundary: &OutputBoundary) -> Self {
        Self {
            kind,
            boundary: OutputBoundaryKey::from(boundary),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum OutputBoundaryKey {
    Inline,
    SameModule,
    SeparateModule(Option<String>),
    External(String),
}

impl From<&OutputBoundary> for OutputBoundaryKey {
    fn from(boundary: &OutputBoundary) -> Self {
        match boundary {
            OutputBoundary::Inline => Self::Inline,
            OutputBoundary::SameModule => Self::SameModule,
            OutputBoundary::SeparateModule { path_hint } => Self::SeparateModule(path_hint.clone()),
            OutputBoundary::External { specifier } => Self::External(specifier.clone()),
        }
    }
}

fn element_has_styles(element: &RenderElement) -> bool {
    !element.styles.is_empty()
        || !element.style_variants.is_empty()
        || !element.dynamic_styles.is_empty()
        || !element.pseudo_elements.is_empty()
}

fn style_source(element: &RenderElement) -> OutputSource {
    let mut spans = Vec::new();
    spans.extend(element.styles.iter().filter_map(|style| style.span));
    spans.extend(element.style_variants.iter().flat_map(style_variant_spans));
    spans.extend(element.dynamic_styles.iter().flat_map(dynamic_style_spans));
    spans.extend(
        element
            .pseudo_elements
            .iter()
            .flat_map(pseudo_element_spans),
    );
    if spans.is_empty() {
        source_from_spans(element.span)
    } else {
        source_from_spans(spans)
    }
}

fn dynamic_style_spans(style: &RenderDynamicStyleBinding) -> Vec<Span> {
    [style.span, style.expression.span]
        .into_iter()
        .flatten()
        .collect()
}

fn style_variant_spans(variant: &RenderStyleVariant) -> Vec<Span> {
    variant
        .span
        .into_iter()
        .chain(variant.declarations.iter().filter_map(|style| style.span))
        .collect()
}

fn pseudo_element_spans(pseudo: &RenderPseudoElement) -> Vec<Span> {
    pseudo
        .span
        .into_iter()
        .chain(pseudo.styles.iter().filter_map(|style| style.span))
        .collect()
}

fn action_source(actions: &[ActionBinding]) -> OutputSource {
    source_from_spans(actions.iter().flat_map(|action| {
        [
            action.span,
            action.action_span,
            action.template.as_ref().and_then(|template| template.span),
        ]
        .into_iter()
        .flatten()
    }))
}

fn bundle_targets(element: &RenderElement) -> Vec<RouteTargetRef<'_>> {
    let mut targets = Vec::new();
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
            .chain(
                element
                    .style_variants
                    .iter()
                    .flat_map(|variant| variant.declarations.iter()),
            )
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

fn source_from_spans(spans: impl IntoIterator<Item = Span>) -> OutputSource {
    OutputSource::new(
        spans
            .into_iter()
            .map(|span| span.source)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>(),
    )
}

fn is_external_specifier(value: &str) -> bool {
    matches!(
        value.split_once(':').map(|(scheme, _)| scheme),
        Some("http" | "https")
    )
}
