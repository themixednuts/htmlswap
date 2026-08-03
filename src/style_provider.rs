use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use arcstr::ArcStr;

use crate::class::ClassIndex;
use crate::diagnostics::{Compilation, Diagnostic, Diagnostics};
use crate::ir::HtmlDocument;
use crate::plan::RenderAnnotation;
use crate::resource::ResourceResolver;
use crate::source::SourceMap;

/// Generates target-neutral CSS before the normal stylesheet pipeline runs.
///
/// Providers are invoked in registration order. Implementations should be
/// deterministic for the same input and return diagnostics instead of
/// panicking for environmental or input failures.
pub trait StyleProvider: Send + Sync {
    fn name(&self) -> &str;

    fn expand(&self, input: StyleProviderInput<'_>) -> Compilation<StyleProviderOutput>;
}

/// An ordered, cloneable collection of style providers.
///
/// The pipeline rejects unnamed providers, skips a provider's partial output
/// when it reports an error, and prevents generated source names from silently
/// shadowing each other.
#[derive(Clone, Default)]
pub struct StyleProviderPipeline {
    providers: Vec<Arc<dyn StyleProvider>>,
}

impl StyleProviderPipeline {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_provider(mut self, provider: impl StyleProvider + 'static) -> Self {
        self.push(provider);
        self
    }

    #[must_use]
    pub fn with_provider_arc(mut self, provider: Arc<dyn StyleProvider>) -> Self {
        self.push_arc(provider);
        self
    }

    pub fn push(&mut self, provider: impl StyleProvider + 'static) {
        self.push_arc(Arc::new(provider));
    }

    pub fn push_arc(&mut self, provider: Arc<dyn StyleProvider>) {
        self.providers.push(provider);
    }

    #[must_use]
    pub fn providers(&self) -> &[Arc<dyn StyleProvider>] {
        &self.providers
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    #[must_use]
    pub fn expand(&self, input: StyleProviderInput<'_>) -> Compilation<StyleProviderOutput> {
        let mut diagnostics = Diagnostics::new();
        let mut output = StyleProviderOutput::new();
        let mut provider_names = BTreeMap::<String, usize>::new();
        let mut stylesheet_names = BTreeMap::<String, ArcStr>::new();

        for (index, provider) in self.providers.iter().enumerate() {
            let name = provider.name().trim();
            if name.is_empty() {
                diagnostics.push(Diagnostic::error(
                    format!("style provider at index {index} has an empty name"),
                    None,
                ));
                continue;
            }
            if let Some(first_index) = provider_names.get(name) {
                diagnostics.push(Diagnostic::error(
                    format!(
                        "duplicate style provider `{name}` at indexes {first_index} and {index}"
                    ),
                    None,
                ));
                continue;
            }
            provider_names.insert(name.to_owned(), index);

            let expanded = provider.expand(input);
            let failed = expanded.diagnostics.has_errors();
            diagnostics.extend(expanded.diagnostics);
            if failed {
                continue;
            }

            for stylesheet in expanded.value.stylesheets {
                let source_name = stylesheet.name.trim();
                if source_name.is_empty() {
                    diagnostics.push(Diagnostic::error(
                        format!(
                            "style provider `{name}` generated a stylesheet with an empty name"
                        ),
                        None,
                    ));
                    continue;
                }

                if let Some(existing) = stylesheet_names.get(source_name) {
                    if existing != &stylesheet.contents {
                        diagnostics.push(Diagnostic::error(
                            format!(
                                "style providers generated conflicting contents for source `{source_name}`"
                            ),
                            None,
                        ));
                    }
                    continue;
                }

                stylesheet_names.insert(source_name.to_owned(), stylesheet.contents.clone());
                output.stylesheets.push(stylesheet);
            }
            output.annotations.extend(expanded.value.annotations);
        }

        Compilation::new(output, diagnostics)
    }
}

impl fmt::Debug for StyleProviderPipeline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StyleProviderPipeline")
            .field(
                "providers",
                &self
                    .providers
                    .iter()
                    .map(|provider| provider.name())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
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

    pub fn extend(&mut self, mut other: Self) {
        self.stylesheets.append(&mut other.stylesheets);
        self.annotations.append(&mut other.annotations);
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
