use std::fmt;

use arcstr::ArcStr;

use crate::class::ClassIndex;
use crate::diagnostics::Compilation;
use crate::ir::HtmlDocument;
use crate::plan::RenderAnnotation;
use crate::resource::ResourceResolver;
use crate::source::SourceMap;

pub trait StyleProvider: Send + Sync {
    fn name(&self) -> &str;

    fn expand(&self, input: StyleProviderInput<'_>) -> Compilation<StyleProviderOutput>;
}

#[derive(Clone, Copy)]
pub struct StyleProviderInput<'a> {
    pub sources: &'a SourceMap,
    pub document: &'a HtmlDocument,
    pub classes: &'a ClassIndex,
    pub resources: &'a dyn ResourceResolver,
}

impl fmt::Debug for StyleProviderInput<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StyleProviderInput")
            .field("sources", &self.sources)
            .field("document", &self.document)
            .field("classes", &self.classes)
            .finish()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StyleProviderOutput {
    pub stylesheets: Vec<GeneratedStyleSource>,
    pub annotations: Vec<RenderAnnotation>,
}

impl StyleProviderOutput {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_stylesheet(mut self, stylesheet: GeneratedStyleSource) -> Self {
        self.stylesheets.push(stylesheet);
        self
    }

    #[must_use]
    pub fn with_annotation(mut self, annotation: RenderAnnotation) -> Self {
        self.annotations.push(annotation);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedStyleSource {
    pub name: String,
    pub contents: ArcStr,
}

impl GeneratedStyleSource {
    #[must_use]
    pub fn new(name: impl Into<String>, contents: impl Into<ArcStr>) -> Self {
        Self {
            name: name.into(),
            contents: contents.into(),
        }
    }
}
