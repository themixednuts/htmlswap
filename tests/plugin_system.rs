use std::sync::{Arc, Mutex};

use htmlswap::{
    Adapter, AdapterContext, ArcStr, Compilation, CompileAssets, Compiler, Diagnostic, Diagnostics,
    Frontend, GeneratedStyleSource, GpuiAdapter, GpuiAdapterOptions, RenderNode, ResourceKind,
    ResourceRequest, ResourceResolver, ResourceResolverChain, ResourceSource, RustFormatOptions,
    SourceDialect, StyleProperty, StyleProvider, StyleProviderInput, StyleProviderOutput,
    StyleValue, TailwindCli, TailwindEngine, TailwindError, TailwindInput, TailwindProvider,
};

#[derive(Debug)]
struct NamedDialect(&'static str);

impl SourceDialect for NamedDialect {
    fn name(&self) -> &str {
        self.0
    }
}

#[test]
fn frontends_clone_shared_dialects_without_erasing_order() {
    let frontend = Frontend::new()
        .with_dialect(NamedDialect("first"))
        .with_dialect(NamedDialect("second"));
    let cloned = frontend.clone();

    assert_eq!(
        cloned
            .dialects()
            .iter()
            .map(|dialect| dialect.name())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert!(Arc::ptr_eq(&frontend.dialects()[0], &cloned.dialects()[0]));
}

#[derive(Debug, Clone)]
struct MemoryResolver {
    name: &'static str,
    handles: bool,
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl ResourceResolver for MemoryResolver {
    fn name(&self) -> &str {
        self.name
    }

    fn can_resolve(&self, _request: &ResourceRequest) -> bool {
        self.handles
    }

    fn resolve(&self, request: &ResourceRequest) -> Compilation<Option<ResourceSource>> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(self.name);
        Compilation::clean(Some(ResourceSource::new(
            request.kind,
            Some(format!("memory:{}", self.name)),
            format!("/* {} */", self.name),
        )))
    }
}

#[test]
fn resolver_chain_uses_the_first_capable_resolver() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let chain = ResourceResolverChain::new()
        .with_resolver(MemoryResolver {
            name: "skip",
            handles: false,
            calls: calls.clone(),
        })
        .with_resolver(MemoryResolver {
            name: "first",
            handles: true,
            calls: calls.clone(),
        })
        .with_resolver(MemoryResolver {
            name: "second",
            handles: true,
            calls: calls.clone(),
        });
    let request = ResourceRequest::new(ResourceKind::Stylesheet, "memory:theme.css");
    let resolved = chain.resolve(&request);

    assert_eq!(
        *calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        ["first"]
    );
    assert_eq!(
        resolved
            .value
            .expect("first resolver should return a source")
            .name
            .as_deref(),
        Some("memory:first")
    );
}

#[derive(Debug, Clone)]
struct StaticStyles {
    name: &'static str,
    source_name: &'static str,
    css: &'static str,
    fail: bool,
}

impl StaticStyles {
    fn new(name: &'static str, source_name: &'static str, css: &'static str) -> Self {
        Self {
            name,
            source_name,
            css,
            fail: false,
        }
    }

    fn failing(mut self) -> Self {
        self.fail = true;
        self
    }
}

impl StyleProvider for StaticStyles {
    fn name(&self) -> &str {
        self.name
    }

    fn expand(&self, _input: StyleProviderInput<'_>) -> Compilation<StyleProviderOutput> {
        let output = StyleProviderOutput::new()
            .with_stylesheet(GeneratedStyleSource::new(self.source_name, self.css));
        let diagnostics = if self.fail {
            [Diagnostic::error("provider failed", None)]
                .into_iter()
                .collect()
        } else {
            Diagnostics::new()
        };
        Compilation::new(output, diagnostics)
    }
}

#[test]
fn style_providers_compose_in_registration_order() {
    let compiled = Compiler::new()
        .with_style_provider(StaticStyles::new(
            "base",
            "generated:base.css",
            ".card { color: red; display: flex; }",
        ))
        .with_style_provider(StaticStyles::new(
            "theme",
            "generated:theme.css",
            ".card { color: blue; }",
        ))
        .compile_fragment(r#"<div class="card"></div>"#, &CompileAssets::new());

    assert!(!compiled.diagnostics.has_errors());
    let RenderNode::Element(card) = &compiled.value.plan.nodes[0] else {
        panic!("expected card element");
    };
    assert!(card.styles.iter().any(|style| {
        style.property == StyleProperty::Display && style.value.as_str() == "flex"
    }));
    assert!(
        card.styles.iter().any(|style| {
            style.property == StyleProperty::Color && style.value.as_str() == "#00f"
        })
    );
    assert_eq!(
        compiled
            .value
            .sources
            .files()
            .iter()
            .filter_map(|source| source.name())
            .filter(|name| name.starts_with("generated:"))
            .collect::<Vec<_>>(),
        ["generated:base.css", "generated:theme.css"]
    );
}

#[test]
fn duplicate_provider_names_and_failed_partial_output_are_rejected() {
    let compiled = Compiler::new()
        .with_style_provider(
            StaticStyles::new("duplicate", "generated:failed.css", ".card { color: red; }")
                .failing(),
        )
        .with_style_provider(StaticStyles::new(
            "duplicate",
            "generated:duplicate.css",
            ".card { color: blue; }",
        ))
        .compile_fragment(r#"<div class="card"></div>"#, &CompileAssets::new());

    assert!(compiled.diagnostics.has_errors());
    assert!(
        compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("duplicate style provider"))
    );
    assert!(compiled.value.sources.files().iter().all(|source| {
        source
            .name()
            .is_none_or(|name| !name.starts_with("generated:"))
    }));
    let RenderNode::Element(card) = &compiled.value.plan.nodes[0] else {
        panic!("expected card element");
    };
    assert!(card.styles.is_empty());
}

#[test]
fn generated_sources_cannot_shadow_authored_sources() {
    let assets = CompileAssets::new()
        .with_stylesheet(Some("shared.css".to_owned()), ".card { color: red; }");
    let compiled = Compiler::new()
        .with_style_provider(StaticStyles::new(
            "conflict",
            "shared.css",
            ".card { color: blue; }",
        ))
        .compile_fragment(r#"<div class="card"></div>"#, &assets);

    assert!(compiled.diagnostics.has_errors());
    let RenderNode::Element(card) = &compiled.value.plan.nodes[0] else {
        panic!("expected card element");
    };
    assert!(
        card.styles.iter().any(|style| {
            style.property == StyleProperty::Color && style.value.as_str() == "red"
        })
    );
}

#[test]
fn jsx_rejects_style_providers_instead_of_silently_skipping_them() {
    let compiled = Compiler::try_with_options(
        htmlswap::CompilerOptions::new().with_source_frontend(htmlswap::SourceFrontendKind::Jsx),
    )
    .expect("JSX compiler options should be valid")
    .with_style_provider(StaticStyles::new(
        "theme",
        "generated:theme.css",
        ".card { color: blue; }",
    ))
    .compile_fragment(
        "export default () => <div className=\"card\" />;",
        &CompileAssets::new(),
    );

    assert!(compiled.diagnostics.has_errors());
    assert!(compiled.diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("style providers require an HTML-based source frontend")
    }));
    assert!(compiled.value.sources.files().iter().all(|source| {
        source
            .name()
            .is_none_or(|name| !name.starts_with("generated:"))
    }));
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedTailwindInput {
    stylesheet: String,
    candidates: Vec<String>,
}

#[derive(Debug, Clone)]
struct RecordingTailwind {
    calls: Arc<Mutex<Vec<RecordedTailwindInput>>>,
    css: ArcStr,
}

impl TailwindEngine for RecordingTailwind {
    fn name(&self) -> &str {
        "recording-tailwind"
    }

    fn compile(&self, input: TailwindInput<'_>) -> Result<ArcStr, TailwindError> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(RecordedTailwindInput {
                stylesheet: input.stylesheet.to_owned(),
                candidates: input.candidates.to_vec(),
            });
        Ok(self.css.clone())
    }
}

#[test]
fn tailwind_is_replaceable_and_enters_the_normal_css_pipeline() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let engine = RecordingTailwind {
        calls: calls.clone(),
        css: ArcStr::from(
            ".flex { display: flex; } .p-4 { padding: 1rem; } .sr-only { position: absolute; }",
        ),
    };
    let provider = TailwindProvider::new(engine)
        .with_stylesheet("@import \"tailwindcss\" source(none); @theme { --spacing: 0.25rem; }")
        .with_safelist(["sr-only", "flex"]);
    let compiled = Compiler::new()
        .with_style_provider(provider)
        .compile_fragment(
            r#"<main class="p-4 flex flex"></main>"#,
            &CompileAssets::new(),
        );

    assert!(!compiled.diagnostics.has_errors());
    assert_eq!(
        *calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        [RecordedTailwindInput {
            stylesheet: "@import \"tailwindcss\" source(none); @theme { --spacing: 0.25rem; }"
                .to_owned(),
            candidates: vec!["flex".to_owned(), "p-4".to_owned(), "sr-only".to_owned()],
        }]
    );
    let RenderNode::Element(element) = &compiled.value.plan.nodes[0] else {
        panic!("expected main element");
    };
    assert!(element.styles.iter().any(|style| {
        style.property == StyleProperty::Display
            && style.value == StyleValue::Keyword("flex".into())
    }));
    assert!(element.styles.iter().any(|style| {
        style.property == StyleProperty::Padding && style.value.as_str() == "1rem"
    }));
    assert!(
        compiled
            .value
            .sources
            .files()
            .iter()
            .any(|source| source.name() == Some("tailwindcss-v4:generated.css"))
    );
}

#[test]
fn tailwind_v4_tokens_calc_lengths_and_hover_guards_lower_to_gpui() {
    let engine = RecordingTailwind {
        calls: Arc::new(Mutex::new(Vec::new())),
        css: ArcStr::from(
            r#"
                :root {
                    --spacing: .25rem;
                    --color-red-500: oklch(63.7% .237 25.331);
                    --color-blue-500: oklch(62.3% .214 259.815);
                }
                .flex { display: flex; }
                .gap-4 { gap: calc(var(--spacing) * 4); }
                .p-4 { padding: calc(var(--spacing) * 4); }
                .text-red-500 { color: var(--color-red-500); }
                @media (hover: hover) {
                    .hover\:text-blue-500:hover { color: var(--color-blue-500); }
                }
            "#,
        ),
    };
    let compiled = Compiler::new()
        .with_style_provider(TailwindProvider::new(engine))
        .compile_fragment(
            r#"<main class="flex gap-4 p-4 text-red-500 hover:text-blue-500"></main>"#,
            &CompileAssets::new(),
        );
    let mut cx = AdapterContext::new();
    let output = GpuiAdapter::new(GpuiAdapterOptions {
        format: RustFormatOptions {
            enabled: false,
            ..RustFormatOptions::default()
        },
        ..GpuiAdapterOptions::default()
    })
    .adapt(&compiled.value, &mut cx)
    .expect("Tailwind-flavored CSS should lower to GPUI");
    let code = &output.artifact.files[0].contents;

    assert!(!compiled.diagnostics.has_errors());
    assert!(cx.diagnostics().is_empty());
    assert!(code.contains(".flex()"), "{code}");
    assert!(code.contains(".gap(gpui::rems(1.0))"), "{code}");
    assert!(code.contains(".p(gpui::rems(1.0))"), "{code}");
    assert!(code.contains(".text_color(gpui::rgb("), "{code}");
    assert!(code.contains(".hover(|this|"), "{code}");
}

#[test]
fn missing_tailwind_cli_is_a_diagnostic_not_a_panic() {
    let compiled = Compiler::new()
        .with_style_provider(TailwindProvider::new(TailwindCli::new(
            "htmlswap-definitely-missing-tailwind-cli",
        )))
        .compile_fragment(r#"<main class="flex"></main>"#, &CompileAssets::new());

    assert!(compiled.diagnostics.has_errors());
    assert!(compiled.diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("tailwind style provider failed")
            && diagnostic.message.contains("spawn Tailwind CLI")
    }));
    assert!(
        compiled
            .value
            .sources
            .files()
            .iter()
            .all(|source| { source.name() != Some("tailwindcss-v4:generated.css") })
    );
}
