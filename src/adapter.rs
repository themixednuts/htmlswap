use std::collections::{BTreeMap, BTreeSet};

use arcstr::ArcStr;
use compact_str::CompactString;

use crate::assets::CompileAssets;
use crate::compiler::CompiledFragment;
use crate::diagnostics::{Diagnostic, Diagnostics};
use crate::plan::{ComponentId, RegionId, UiRole};
use crate::source::GeneratedSourceMap;
use crate::style::StyleProperty;

#[derive(Debug, Default)]
pub struct AdapterContext {
    diagnostics: Diagnostics,
}

impl AdapterContext {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LayerId(CompactString);

impl LayerId {
    #[must_use]
    pub fn new(id: impl Into<CompactString>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for LayerId {
    fn from(id: &str) -> Self {
        Self::new(id)
    }
}

impl From<String> for LayerId {
    fn from(id: String) -> Self {
        Self::new(id)
    }
}

impl From<CompactString> for LayerId {
    fn from(id: CompactString) -> Self {
        Self::new(id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteTarget {
    Role(UiRole),
    Component(ComponentId),
    Region(RegionId),
    Class(CompactString),
    Tag(CompactString),
    Style(StyleProperty),
    Event(CompactString),
}

impl RouteTarget {
    #[must_use]
    pub fn component(component: impl AsRef<str>) -> Self {
        Self::Component(ComponentId::new(component))
    }

    #[must_use]
    pub fn region(region: impl AsRef<str>) -> Option<Self> {
        RegionId::parse(region).map(Self::Region)
    }

    #[must_use]
    pub fn tag(tag: impl Into<CompactString>) -> Self {
        Self::Tag(tag.into())
    }

    #[must_use]
    pub fn class(class: impl Into<CompactString>) -> Self {
        Self::Class(class.into())
    }

    #[must_use]
    pub fn event(event: impl Into<CompactString>) -> Self {
        Self::Event(event.into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteTargetRef<'a> {
    Role(&'a UiRole),
    Component(&'a ComponentId),
    Region(&'a RegionId),
    Class(&'a str),
    Tag(&'a str),
    Style(&'a StyleProperty),
    Event(&'a str),
}

impl RouteTarget {
    #[must_use]
    pub fn matches_ref(&self, target: RouteTargetRef<'_>) -> bool {
        match (self, target) {
            (Self::Role(left), RouteTargetRef::Role(right)) => left == right,
            (Self::Component(left), RouteTargetRef::Component(right)) => left == right,
            (Self::Region(left), RouteTargetRef::Region(right)) => left == right,
            (Self::Class(left), RouteTargetRef::Class(right)) => left == right,
            (Self::Tag(left), RouteTargetRef::Tag(right)) => left == right,
            (Self::Style(left), RouteTargetRef::Style(right)) => left == right,
            (Self::Event(left), RouteTargetRef::Event(right)) => left == right,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerClaim {
    All,
    Targets(Vec<RouteTarget>),
}

impl LayerClaim {
    #[must_use]
    pub fn matches(&self, target: &RouteTarget) -> bool {
        match self {
            Self::All => true,
            Self::Targets(targets) => targets.contains(target),
        }
    }

    #[must_use]
    pub fn matches_ref(&self, target: RouteTargetRef<'_>) -> bool {
        match self {
            Self::All => true,
            Self::Targets(targets) => targets.iter().any(|claim| claim.matches_ref(target)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerBinding {
    pub id: LayerId,
    pub claim: LayerClaim,
}

impl LayerBinding {
    #[must_use]
    pub fn new(id: impl Into<LayerId>, claim: LayerClaim) -> Self {
        Self {
            id: id.into(),
            claim,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteOverride {
    pub target: RouteTarget,
    pub layer: LayerId,
}

impl RouteOverride {
    #[must_use]
    pub fn new(target: RouteTarget, layer: impl Into<LayerId>) -> Self {
        Self {
            target,
            layer: layer.into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteConfig {
    pub layers: Vec<LayerBinding>,
    pub overrides: Vec<RouteOverride>,
}

impl RouteConfig {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_base(self, layer: impl Into<LayerId>) -> Self {
        self.with_layer(layer, LayerClaim::All)
    }

    #[must_use]
    pub fn with_layer(mut self, layer: impl Into<LayerId>, claim: LayerClaim) -> Self {
        self.layers.push(LayerBinding::new(layer, claim));
        self
    }

    #[must_use]
    pub fn with_override(mut self, target: RouteTarget, layer: impl Into<LayerId>) -> Self {
        self.overrides.push(RouteOverride::new(target, layer));
        self
    }

    #[must_use]
    pub fn override_for(&self, target: &RouteTarget) -> Option<&LayerId> {
        self.overrides
            .iter()
            .rev()
            .find(|route| &route.target == target)
            .map(|route| &route.layer)
    }

    #[must_use]
    pub fn override_for_ref(&self, target: RouteTargetRef<'_>) -> Option<&LayerId> {
        self.overrides
            .iter()
            .rev()
            .find(|route| route.target.matches_ref(target))
            .map(|route| &route.layer)
    }

    #[must_use]
    pub fn resolve(&self, target: &RouteTarget) -> Option<&LayerId> {
        self.override_for(target)
            .or_else(|| self.claiming_layer(target))
            .or_else(|| self.fallback_layer(target))
    }

    #[must_use]
    pub fn resolve_ref(&self, target: RouteTargetRef<'_>) -> Option<&LayerId> {
        self.override_for_ref(target)
            .or_else(|| self.claiming_layer_ref(target))
            .or_else(|| self.fallback_layer_ref(target))
    }

    #[must_use]
    pub fn claiming_layer(&self, target: &RouteTarget) -> Option<&LayerId> {
        self.layers
            .iter()
            .rev()
            .find(|layer| !matches!(layer.claim, LayerClaim::All) && layer.claim.matches(target))
            .map(|layer| &layer.id)
    }

    #[must_use]
    pub fn claiming_layer_ref(&self, target: RouteTargetRef<'_>) -> Option<&LayerId> {
        self.layers
            .iter()
            .rev()
            .find(|layer| {
                !matches!(layer.claim, LayerClaim::All) && layer.claim.matches_ref(target)
            })
            .map(|layer| &layer.id)
    }

    #[must_use]
    pub fn routed_layer_ref(&self, target: RouteTargetRef<'_>) -> Option<&LayerId> {
        self.override_for_ref(target)
            .or_else(|| self.claiming_layer_ref(target))
            .or_else(|| self.fallback_layer_ref(target))
    }

    #[must_use]
    pub fn resolve_first(&self, targets: &[RouteTarget]) -> Option<&LayerId> {
        for target in targets {
            if let Some(layer) = self.override_for(target) {
                return Some(layer);
            }
        }

        for target in targets {
            if let Some(layer) = self.claiming_layer(target) {
                return Some(layer);
            }
        }

        targets
            .iter()
            .find_map(|target| self.fallback_layer(target))
    }

    #[must_use]
    pub fn resolve_first_ref(&self, targets: &[RouteTargetRef<'_>]) -> Option<&LayerId> {
        for target in targets {
            if let Some(layer) = self.override_for_ref(*target) {
                return Some(layer);
            }
        }

        for target in targets {
            if let Some(layer) = self.claiming_layer_ref(*target) {
                return Some(layer);
            }
        }

        targets
            .iter()
            .find_map(|target| self.fallback_layer_ref(*target))
    }

    fn fallback_layer(&self, target: &RouteTarget) -> Option<&LayerId> {
        self.layers
            .iter()
            .rev()
            .find(|layer| layer.claim.matches(target))
            .map(|layer| &layer.id)
    }

    fn fallback_layer_ref(&self, target: RouteTargetRef<'_>) -> Option<&LayerId> {
        self.layers
            .iter()
            .rev()
            .find(|layer| layer.claim.matches_ref(target))
            .map(|layer| &layer.id)
    }

    pub fn validate(&self, cx: &mut AdapterContext) {
        let mut seen = BTreeSet::new();
        for layer in &self.layers {
            if !seen.insert(layer.id.as_str()) {
                cx.push(Diagnostic::warning(
                    format!(
                        "route config contains duplicate layer `{}`",
                        layer.id.as_str()
                    ),
                    None,
                ));
            }
        }

        for route in &self.overrides {
            if !seen.contains(route.layer.as_str()) {
                cx.push(Diagnostic::warning(
                    format!(
                        "route override for `{:?}` targets unknown layer `{}`",
                        route.target,
                        route.layer.as_str()
                    ),
                    None,
                ));
            }
        }

        if !self
            .layers
            .iter()
            .any(|layer| matches!(layer.claim, LayerClaim::All))
        {
            cx.push(Diagnostic::warning(
                "route config has no base layer; unmatched targets will not resolve",
                None,
            ));
        }
    }
}

pub trait Layer {
    fn id(&self) -> &LayerId;

    fn claim(&self) -> LayerClaim {
        LayerClaim::All
    }

    fn dependencies(&self) -> Vec<TargetDependency> {
        Vec::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetDependency {
    /// The name code refers to the crate by, when it differs from the
    /// package (`gpui = { package = "gpui-pre" }`).
    pub name: Option<CompactString>,
    pub package: CompactString,
    pub version_req: CompactString,
    pub source: DependencySource,
    pub default_features: bool,
    pub features: Vec<CompactString>,
}

impl TargetDependency {
    #[must_use]
    pub fn crates_io(
        package: impl Into<CompactString>,
        version_req: impl Into<CompactString>,
    ) -> Self {
        Self {
            name: None,
            package: package.into(),
            version_req: version_req.into(),
            source: DependencySource::CratesIo,
            default_features: true,
            features: Vec::new(),
        }
    }

    /// Refer to the package by another name, as Cargo's `package` key does.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<CompactString>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// The name code refers to the crate by.
    #[must_use]
    pub fn key(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.package)
    }

    /// The dependency as a `Cargo.toml` value.
    #[must_use]
    pub fn cargo_value(&self) -> String {
        match &self.name {
            None => format!("\"{}\"", self.version_req),
            Some(_) => format!(
                "{{ package = \"{}\", version = \"{}\" }}",
                self.package, self.version_req
            ),
        }
    }

    #[must_use]
    pub fn with_default_features(mut self, default_features: bool) -> Self {
        self.default_features = default_features;
        self
    }

    #[must_use]
    pub fn with_features(
        mut self,
        features: impl IntoIterator<Item = impl Into<CompactString>>,
    ) -> Self {
        self.features = features.into_iter().map(Into::into).collect();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencySource {
    CratesIo,
    Git { url: String, rev: Option<String> },
    Path { path: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
    pub path: String,
    pub contents: String,
    pub source_map: Option<GeneratedSourceMap>,
}

impl GeneratedFile {
    #[must_use]
    pub fn new(path: impl Into<String>, contents: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            contents: contents.into(),
            source_map: None,
        }
    }

    #[must_use]
    pub fn with_source_map(mut self, source_map: GeneratedSourceMap) -> Self {
        self.source_map = Some(source_map);
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdapterArtifact {
    pub files: Vec<GeneratedFile>,
    pub dependencies: Vec<TargetDependency>,
}

impl AdapterArtifact {
    #[must_use]
    pub fn new(files: Vec<GeneratedFile>, dependencies: Vec<TargetDependency>) -> Self {
        Self {
            files,
            dependencies,
        }
    }

    #[must_use]
    pub fn builder() -> ArtifactBuilder {
        ArtifactBuilder::new()
    }
}

#[derive(Debug, Default)]
pub struct ArtifactBuilder {
    files: BTreeMap<String, GeneratedFile>,
    dependencies: DependencySet,
}

impl ArtifactBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_file(&mut self, file: GeneratedFile, cx: &mut AdapterContext) {
        if let Some(existing) = self.files.get(&file.path) {
            if existing.contents != file.contents {
                cx.push(Diagnostic::warning(
                    format!(
                        "generated file `{}` has conflicting contents; keeping first version",
                        file.path
                    ),
                    None,
                ));
            }
            return;
        }

        self.files.insert(file.path.clone(), file);
    }

    pub fn add_dependency(&mut self, dependency: TargetDependency, cx: &mut AdapterContext) {
        self.dependencies.insert(dependency, cx);
    }

    pub fn add_artifact(&mut self, artifact: AdapterArtifact, cx: &mut AdapterContext) {
        for file in artifact.files {
            self.add_file(file, cx);
        }

        for dependency in artifact.dependencies {
            self.add_dependency(dependency, cx);
        }
    }

    #[must_use]
    pub fn finish(self) -> AdapterArtifact {
        AdapterArtifact::new(
            self.files.into_values().collect(),
            self.dependencies.into_vec(),
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DependencySet {
    dependencies: BTreeMap<CompactString, TargetDependency>,
}

impl DependencySet {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, dependency: TargetDependency, cx: &mut AdapterContext) {
        if let Some(existing) = self.dependencies.get_mut(dependency.key()) {
            if existing.version_req != dependency.version_req
                || existing.source != dependency.source
            {
                cx.push(Diagnostic::warning(
                    format!(
                        "dependency `{}` has conflicting requirements: `{}` and `{}`",
                        dependency.package, existing.version_req, dependency.version_req
                    ),
                    None,
                ));
                return;
            }

            existing.default_features &= dependency.default_features;
            let mut features = existing.features.iter().cloned().collect::<BTreeSet<_>>();
            features.extend(dependency.features);
            existing.features = features.into_iter().collect();
            return;
        }

        self.dependencies
            .insert(dependency.key().into(), dependency);
    }

    #[must_use]
    pub fn into_vec(self) -> Vec<TargetDependency> {
        self.dependencies.into_values().collect()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("{message}")]
    Message { message: String },
    #[error("unsupported adapter operation: {message}")]
    Unsupported { message: String },
}

impl AdapterError {
    #[must_use]
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message {
            message: message.into(),
        }
    }

    #[must_use]
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::Unsupported {
            message: message.into(),
        }
    }
}

pub trait Adapter {
    type Output;

    fn adapt(
        &self,
        fragment: &CompiledFragment,
        cx: &mut AdapterContext,
    ) -> Result<Self::Output, AdapterError>;
}

pub trait Importer {
    type Source;
    type Output;

    fn import(
        &self,
        source: &Self::Source,
        cx: &mut AdapterContext,
    ) -> Result<Self::Output, AdapterError>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TargetArtifact {
    pub files: Vec<TargetSourceFile>,
}

impl TargetArtifact {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_file(mut self, path: impl Into<String>, contents: impl Into<ArcStr>) -> Self {
        self.files.push(TargetSourceFile::new(path, contents));
        self
    }

    #[must_use]
    pub fn with_source_file(mut self, file: TargetSourceFile) -> Self {
        self.files.push(file);
        self
    }
}

impl From<AdapterArtifact> for TargetArtifact {
    fn from(artifact: AdapterArtifact) -> Self {
        Self {
            files: artifact.files.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetSourceFile {
    pub path: String,
    pub contents: ArcStr,
    pub source_map: Option<GeneratedSourceMap>,
}

impl TargetSourceFile {
    #[must_use]
    pub fn new(path: impl Into<String>, contents: impl Into<ArcStr>) -> Self {
        Self {
            path: path.into(),
            contents: contents.into(),
            source_map: None,
        }
    }

    #[must_use]
    pub fn with_source_map(mut self, source_map: GeneratedSourceMap) -> Self {
        self.source_map = Some(source_map);
        self
    }
}

impl From<GeneratedFile> for TargetSourceFile {
    fn from(file: GeneratedFile) -> Self {
        Self {
            path: file.path,
            contents: ArcStr::from(file.contents),
            source_map: file.source_map,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlArtifact {
    pub document: GeneratedFile,
    pub assets: CompileAssets,
}

impl HtmlArtifact {
    #[must_use]
    pub fn new(document: GeneratedFile, assets: CompileAssets) -> Self {
        Self { document, assets }
    }

    #[must_use]
    pub fn from_html(path: impl Into<String>, contents: impl Into<String>) -> Self {
        Self::new(GeneratedFile::new(path, contents), CompileAssets::new())
    }

    #[must_use]
    pub fn html(&self) -> &str {
        &self.document.contents
    }
}
