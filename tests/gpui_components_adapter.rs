mod common;

use common::emitted_child_signatures_in_stack_order;
use htmlswap::{
    Adapter, AdapterContext, CompileAssets, Compiler, DcComponentFragment, Frontend,
    GPUI_COMPONENT_CRATE_VERSION, GPUI_COMPONENTS_LAYER_ID, GPUI_CRATE_VERSION, GPUI_LAYER_ID,
    GpuiAdapterOptions, GpuiComponentsAdapter, GpuiComponentsAdapterOptions, Importer, RouteConfig,
    Severity, SourceMappingKind, TargetArtifact, inline_dc_component_imports,
};
use rustfmt_wrapper::config::{Config as RustfmtConfig, Edition, NewlineStyle};
use rustfmt_wrapper::rustfmt_config;

#[test]
fn gpui_components_adapter_composes_button_layer_over_gpui_base() {
    let assets = CompileAssets::new()
        .with_stylesheet(
            Some("app.css".to_owned()),
            "/* css note */\n.primary { display: flex; gap: 8px; color: red; }",
        )
        .with_script(Some("app.js".to_owned()), "// js note\nfunction save() {}");
    let compiled = Compiler::new().compile_fragment(
        r#"<!-- html note --><button id="save" class="primary" title="Save now" onclick="save()">Save</button>"#,
        &assets,
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit code");
    let code = output.code();
    let dependencies = output.dependencies();

    syn::parse_file(code).expect("GPUI components adapter should emit syntactically valid Rust");

    assert_eq!(
        dependencies
            .iter()
            .find(|dependency| dependency.package == "gpui")
            .expect("gpui dependency should be present")
            .version_req,
        GPUI_CRATE_VERSION
    );
    assert_eq!(
        dependencies
            .iter()
            .find(|dependency| dependency.package == "gpui-component")
            .expect("gpui-component dependency should be present")
            .version_req,
        GPUI_COMPONENT_CRATE_VERSION
    );

    assert!(code.contains("use gpui::{"));
    assert!(code.contains("AppContext as _"));
    assert!(code.contains("use gpui::prelude::FluentBuilder as _;"));
    assert!(code.contains("use gpui_component::button::ButtonVariants as _;"));
    assert!(code.contains("use gpui_component::Disableable as _;"));
    assert!(code.contains("use gpui_component::Selectable as _;"));
    assert!(code.contains("use gpui_component::Sizable as _;"));
    assert!(code.contains("\tfn render("));
    assert!(!code.contains("    fn render("));
    assert!(!code.contains("\r\n"));
    assert!(code.contains("// html-comment: html note"));
    assert!(code.contains("// css-comment: css note"));
    assert!(code.contains("// js-line-comment: js note"));
    assert!(code.contains("gpui_component::button::Button::new(\"save\")"));
    assert!(code.contains(".label(\"Save\")"));
    assert!(code.contains(".tooltip(\"Save now\")"));
    assert!(code.contains(".flex()"));
    assert!(code.contains(".gap(gpui::px(8.0))"));
    assert!(code.contains(".text_color(gpui::rgb(0xFF0000))"));
    assert!(code.contains("save();"));
    assert!(code.contains("// html classes: primary"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_maps_explicit_tab_components() {
    let compiled = Compiler::new().compile_fragment(
        r#"
        <div id="views" data-htmlswap-component="tabs" data-htmlswap-variant="underline" data-htmlswap-size="sm">
            <button data-htmlswap-component="tab" selected>Scene</button>
            <button data-htmlswap-component="tab" disabled>Game</button>
        </div>
        "#,
        &CompileAssets::new(),
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit tabs");
    let code = output.code();

    syn::parse_file(code).expect("GPUI components adapter should emit syntactically valid Rust");
    assert!(code.contains("use gpui_component::Selectable as _;"));
    assert!(code.contains("gpui_component::tab::TabBar::new(\"views\")"));
    assert!(code.contains(".underline()"));
    assert!(code.contains(".with_size(gpui_component::Size::Small)"));
    assert_eq!(code.matches("gpui_component::tab::Tab::new()").count(), 2);
    assert!(code.contains(".selected(true)"));
    assert!(code.contains(".disabled(true)"));
    assert!(code.contains(".child(\"Scene\")"));
    assert!(code.contains(".child(\"Game\")"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_maps_aria_tabs_without_guessing_div_rows() {
    let assets = CompileAssets::new().with_script(
        Some("tabs.js".to_owned()),
        "function openA() {}\nfunction openB() {}",
    );
    let compiled = Compiler::new().compile_fragment(
        r#"
        <div role="tablist">
            <button role="tab" aria-selected="true">Scene</button>
            <button role="tab">Game</button>
        </div>
        <div class="row"><div onclick="openA()">A</div><div onclick="openB()">B</div></div>
        "#,
        &assets,
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit ARIA tabs");
    let code = output.code();

    syn::parse_file(code).expect("GPUI components adapter should emit syntactically valid Rust");
    assert_eq!(code.matches("gpui_component::tab::TabBar::new(").count(), 1);
    assert_eq!(code.matches("gpui_component::tab::Tab::new()").count(), 2);
    assert!(code.contains(".selected(true)"));
    assert!(code.contains("openA();"));
    assert!(code.contains("openB();"));
    assert!(
        adapter_context
            .diagnostics()
            .as_slice()
            .iter()
            .all(|diagnostic| diagnostic.message.contains("accessibility"))
    );
}

#[test]
fn gpui_components_adapter_can_emit_source_stable_layout_debug_ids() {
    let compiled = Compiler::new().compile_fragment(
        r#"<button>Run</button><a href="/docs">Docs</a>"#,
        &CompileAssets::new(),
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut options = GpuiComponentsAdapterOptions::components();
    options.gpui.emit_debug_layout_ids = true;

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::new(options)
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI components adapter should emit syntactically valid Rust");

    assert!(code.contains("gpui_component::button::Button::new(\"htmlswap_debug_source_s0_0_"));
    assert!(code.contains("gpui_component::link::Link::new(\"htmlswap_debug_source_s0_20_"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_maps_button_source_semantics_to_component_methods() {
    let compiled = Compiler::new().compile_fragment(
        r#"
            <button
                id="delete"
                data-htmlswap-tone="danger"
                data-htmlswap-variant="outline"
                data-htmlswap-size="sm"
                data-htmlswap-density="compact"
            >Delete</button>
        "#,
        &CompileAssets::new(),
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit semantic button code");
    let code = output.code();

    syn::parse_file(code).expect("semantic GPUI components output should be valid Rust");

    assert!(code.contains("gpui_component::button::Button::new(\"delete\")"));
    assert!(code.contains(".danger()"));
    assert!(code.contains(".outline()"));
    assert!(code.contains(".with_size(gpui_component::Size::Small)"));
    assert!(code.contains(".compact()"));
    assert!(code.contains("// source tone: danger"));
    assert!(code.contains("// source variant: outline"));
    assert!(code.contains("// source size: sm"));
    assert!(code.contains("// source density: compact"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_maps_button_text_variant_and_interactive_states() {
    let compiled = Compiler::new().compile_fragment(
        r#"
            <button
                id="toggle-preview"
                data-htmlswap-variant="text"
                aria-pressed="true"
                aria-busy="true"
            >Preview</button>
        "#,
        &CompileAssets::new(),
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit text-variant button code");
    let code = output.code();

    syn::parse_file(code).expect("text-variant GPUI components output should be valid Rust");

    assert!(code.contains("gpui_component::button::Button::new(\"toggle-preview\")"));
    assert!(code.contains(".text()"));
    assert!(code.contains(".selected(true)"));
    assert!(code.contains(".loading(true)"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_warns_when_button_semantics_are_lossy() {
    let compiled = Compiler::new().compile_fragment(
        r#"
            <button
                id="soft"
                data-htmlswap-tone="brand-secondary"
                data-htmlswap-variant="soft"
                data-htmlswap-size="xl"
                data-htmlswap-density="spacious"
            >Soft</button>
        "#,
        &CompileAssets::new(),
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should still emit lossy semantic button code");
    let code = output.code();

    syn::parse_file(code).expect("lossy semantic GPUI components output should be valid Rust");
    assert!(code.contains(".with_size(gpui_component::Size::Large)"));
    assert!(code.contains("// source tone: brand-secondary"));
    assert!(code.contains("// source variant: soft"));
    assert!(code.contains("// source size: xl"));
    assert!(code.contains("// source density: spacious"));

    let diagnostics = adapter_context.diagnostics().as_slice();
    assert_eq!(diagnostics.len(), 4, "{diagnostics:?}");
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity == Severity::Warning)
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("tone `brand-secondary`"))
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("variant `soft`"))
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("size `xl`"))
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("density `spacious`"))
    );
}

#[test]
fn gpui_components_adapter_only_adds_component_dependency_when_used() {
    let compiled = Compiler::new()
        .compile_fragment(r#"<div class="shell">Plain</div>"#, &CompileAssets::new());
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit code");
    let code = output.code();
    let dependencies = output.dependencies();

    syn::parse_file(code).expect("GPUI components adapter should emit syntactically valid Rust");

    assert_eq!(dependencies.len(), 1);
    assert_eq!(dependencies[0].package, "gpui");
    assert!(!code.contains("gpui_component"));
    assert!(code.contains("pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_prefers_active_theme_for_css_tokens() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        r#"
            :root {
                --background: #ffffff;
                --foreground: #111111;
                --border: #dddddd;
                --radius: 8px;
            }
            .card {
                background-color: var(--background);
                color: var(--foreground);
                border: 1px solid var(--border);
                border-radius: var(--radius);
            }
        "#,
    );
    let compiled = Compiler::new().compile_fragment(r#"<div class="card">Panel</div>"#, &assets);
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit themed code");
    let code = output.code();
    let dependencies = output.dependencies();

    syn::parse_file(code).expect("themed GPUI components output should be valid Rust");

    assert!(dependencies.iter().any(|dependency| {
        dependency.package == "gpui-component"
            && dependency.version_req == GPUI_COMPONENT_CRATE_VERSION
    }));
    assert!(code.contains("use gpui_component::ActiveTheme as _;"));
    assert!(code.contains("let htmlswap_theme_background = _cx.theme().background;"));
    assert!(code.contains("let htmlswap_theme_foreground = _cx.theme().foreground;"));
    assert!(code.contains("let htmlswap_theme_border = _cx.theme().border;"));
    assert!(code.contains("let htmlswap_theme_radius = _cx.theme().radius;"));
    assert!(code.contains(".bg(htmlswap_theme_background)"));
    assert!(code.contains(".text_color(htmlswap_theme_foreground)"));
    assert!(code.contains(".border(gpui::px(1.0))"));
    assert!(code.contains(".border_color(htmlswap_theme_border)"));
    assert!(code.contains(".rounded(htmlswap_theme_radius)"));
    let file = output
        .artifact
        .files
        .first()
        .expect("GPUI components output should contain a file");
    let source_map = file
        .source_map
        .as_ref()
        .expect("themed output should keep a generated source map");
    assert!(source_map.mappings().iter().any(|mapping| {
        mapping.kind == SourceMappingKind::Style
            && file.contents[mapping.generated.start..mapping.generated.end]
                .contains("let htmlswap_theme_background")
            && compiled
                .value
                .sources
                .source_text(mapping.original)
                .is_some_and(|source| source.contains("--background"))
    }));
    assert!(source_map.mappings().iter().any(|mapping| {
        mapping.kind == SourceMappingKind::Style
            && file.contents[mapping.generated.start..mapping.generated.end]
                .contains(".bg(htmlswap_theme_background)")
            && compiled
                .value
                .sources
                .source_text(mapping.original)
                .is_some_and(|source| source.contains("background-color"))
    }));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_emits_rustfmt_idempotent_hard_tab_output() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"
        <section>
            <input value="{{ user.name }}">
            <input placeholder="Filter rows…" value="{{ filter }}">
        </section>
        "#,
            &CompileAssets::new(),
        );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit code");
    let code = output.code();
    let formatted_again = rustfmt_config(
        RustfmtConfig {
            edition: Some(Edition::Edition2024),
            hard_tabs: Some(true),
            tab_spaces: Some(4),
            max_width: Some(100),
            newline_style: Some(NewlineStyle::Unix),
            ..Default::default()
        },
        code.to_owned(),
    )
    .expect("rustfmt should format generated output");

    assert_eq!(formatted_again, code);
    let file = output
        .artifact
        .files
        .first()
        .expect("GPUI components output should contain a file");
    let source_map = file
        .source_map
        .as_ref()
        .expect("GPUI components output should keep a generated source map");
    assert!(source_map.mappings().iter().all(|mapping| {
        mapping.generated.start <= mapping.generated.end && mapping.generated.end <= code.len()
    }));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_uses_surface_theme_for_dark_neutral_background_tokens() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        r#"
            :root {
                --accent: #1b1e23;
                --blue: #3160a8;
            }
            .panel {
                background-color: var(--accent);
                color: var(--accent);
                border: 1px solid var(--accent);
            }
            .brand {
                background-color: var(--blue);
            }
        "#,
    );
    let compiled = Compiler::new().compile_fragment(
        r#"<section><div class="panel">Panel</div><div class="brand">Brand</div></section>"#,
        &assets,
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit themed code");
    let code = output.code();

    syn::parse_file(code).expect("themed GPUI components output should be valid Rust");

    assert!(code.contains("let htmlswap_theme_accent_background = _cx.theme().background;"));
    assert!(code.contains("let htmlswap_theme_accent = _cx.theme().accent;"));
    assert!(code.contains("let htmlswap_theme_blue = _cx.theme().blue;"));
    assert!(code.contains(".bg(htmlswap_theme_accent_background)"));
    assert!(code.contains(".text_color(htmlswap_theme_accent)"));
    assert!(code.contains(".bg(htmlswap_theme_blue)"));
    assert!(!code.contains("let htmlswap_theme_accent = _cx.theme().background;"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_maps_link_label_and_list_item_components() {
    let assets =
        CompileAssets::new().with_script(Some("app.js".to_owned()), "function openDocs() {}");
    let compiled = Compiler::new().compile_fragment(
        r#"<a id="docs" href="https://example.com" onclick="openDocs()">Docs</a><label>Name</label><li id="row" selected>Row</li>"#,
        &assets,
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit code");
    let code = output.code();
    let dependencies = output.dependencies();

    syn::parse_file(code).expect("GPUI components adapter should emit syntactically valid Rust");

    assert_eq!(dependencies.len(), 2);
    assert!(code.contains("gpui_component::link::Link::new(\"docs\")"));
    assert!(code.contains(".href(\"https://example.com\")"));
    assert!(code.contains("\"Docs\""));
    assert!(code.contains("openDocs();"));
    assert!(code.contains("gpui_component::label::Label::new(\"Name\")"));
    assert!(code.contains("gpui_component::list::ListItem::new(\"row\")"));
    assert!(code.contains(".selected(true)"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_preserves_component_and_base_child_order_with_stack_scan() {
    let compiled = Compiler::new().compile_fragment(
        r##"<div><a id="docs" href="#">Docs</a><span id="middle"></span><button id="save">Save</button></div>"##,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI components adapter should emit syntactically valid Rust");

    assert_eq!(
        emitted_child_signatures_in_stack_order(code),
        ["new:docs", "id:middle", "new:save"]
    );
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_maps_input_controls_with_self_contained_components() {
    let compiled = Compiler::new().compile_fragment(
        r#"
            <input id="agree" type="checkbox" checked disabled aria-label="Agree">
            <input id="choice" type="radio" checked aria-label="Choice">
            <input id="send" type="submit" value="Send">
        "#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI components adapter should emit syntactically valid Rust");

    assert!(code.contains("agree: bool"));
    assert!(code.contains("choice: bool"));
    assert!(code.contains("gpui_component::checkbox::Checkbox::new(\"agree\")"));
    assert!(code.contains(".checked(self.agree)"));
    assert!(code.contains(".disabled(true)"));
    assert!(code.contains(".label(\"Agree\")"));
    assert!(code.contains("gpui_component::radio::Radio::new(\"choice\")"));
    assert!(code.contains(".checked(self.choice)"));
    assert!(code.contains(".label(\"Choice\")"));
    assert!(code.contains("gpui_component::button::Button::new(\"send\")"));
    assert!(code.contains(".label(\"Send\")"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_maps_titlebar_to_native_component_controls() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        r#"
            :root {
                --accent: #3160a8;
                --cyan: #5ac0c0;
            }
            .titlebar {
                height: 36px;
                display: flex;
                align-items: center;
                justify-content: space-between;
                background: linear-gradient(135deg, var(--accent), var(--cyan));
            }
            .brand {
                display: flex;
                gap: 8px;
            }
        "#,
    );
    let compiled = Compiler::new().compile_fragment(
        r#"
            <div id="app-titlebar" class="titlebar" data-htmlswap-component="Title-Bar">
                <div class="brand"><span>Aether</span><span>Project Manager</span></div>
                <div class="window-controls" data-htmlswap-slot="Traffic-Lights">
                    <button aria-label="Minimize">remove</button>
                    <button aria-label="Maximize">crop_square</button>
                    <button aria-label="Close">close</button>
                </div>
            </div>
        "#,
        &assets,
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit native titlebar code");
    let code = output.code();
    let compact_code = code.split_whitespace().collect::<String>();

    syn::parse_file(code).expect("GPUI components titlebar output should be valid Rust");

    assert!(code.contains("gpui_component::TitleBar::new()"));
    assert!(code.contains("Aether"));
    assert!(code.contains("Project Manager"));
    assert!(
        code.contains(
            "source window controls replaced by gpui_component::TitleBar native controls"
        )
    );
    assert!(code.contains("let htmlswap_theme_accent = _cx.theme().accent;"));
    assert!(code.contains("let htmlswap_theme_cyan = _cx.theme().cyan;"));
    assert!(compact_code.contains(".bg(gpui::linear_gradient(135.0,gpui::linear_color_stop(htmlswap_theme_accent,0.0),gpui::linear_color_stop(htmlswap_theme_cyan,1.0),))"));
    assert!(!compact_code.contains(".child(\"crop_square\")"));
    assert!(!code.contains("Button::new"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_maps_sc_titlebar_class_conventions_to_component_hints() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        r#"
            .titlebar {
                display: flex;
            }
        "#,
    );
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"
            <div class="titlebar">
                <span>Aether</span>
                <div class="window-controls">
                    <button aria-label="Close">close</button>
                </div>
            </div>
        "#,
            &assets,
        );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("SC titlebar conventions should map to native titlebar code");
    let code = output.code();

    syn::parse_file(code).expect("SC titlebar output should be valid Rust");

    assert!(code.contains("gpui_component::TitleBar::new()"));
    assert!(code.contains("Aether"));
    assert!(code.contains("source window controls replaced"));
    assert!(!code.contains("Button::new"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_maps_sc_titlebar_comment_conventions_to_component_hints() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"
            <!-- titlebar -->
            <div>
                <span>Aether</span>
                <div style="margin-left:auto;display:flex;align-items:center;gap:2px;">
                    <button aria-label="Minimize window"><span>remove</span></button>
                    <button aria-label="Maximize window"><span>crop_square</span></button>
                    <button aria-label="Close window"><span>close</span></button>
                </div>
            </div>
        "#,
            &CompileAssets::new(),
        );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("SC titlebar comments should map to native titlebar code");
    let code = output.code();
    let compact_code = code.split_whitespace().collect::<String>();

    syn::parse_file(code).expect("SC comment titlebar output should be valid Rust");

    assert!(code.contains("gpui_component::TitleBar::new()"));
    assert!(code.contains("Aether"));
    assert!(code.contains("source window controls replaced"));
    assert!(!compact_code.contains(".child(\"remove\")"));
    assert!(!compact_code.contains(".child(\"crop_square\")"));
    assert!(!compact_code.contains(".child(\"close\")"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_maps_dc_app_window_menubar_to_titlebar() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"
            <x-dc>
                <div data-htmlswap-region="app.window" data-htmlswap-component="menubar">
                    <span>Aether</span>
                    <div onclick="open_menu()"><span>File</span></div>
                    <div data-htmlswap-slot="window-controls">
                        <button aria-label="Maximize window"><span>crop_square</span></button>
                        <button aria-label="Close window"><span>close</span></button>
                    </div>
                </div>
                <script data-dc-script>
                    function open_menu() {}
                </script>
            </x-dc>
        "#,
            &CompileAssets::new(),
        );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("DC app.window menubar should map to native titlebar code");
    let code = output.code();
    let compact_code = code.split_whitespace().collect::<String>();

    syn::parse_file(code).expect("DC app.window titlebar output should be valid Rust");

    assert!(code.contains("gpui_component::TitleBar::new()"));
    assert!(code.contains("source component: titlebar"));
    assert!(code.contains(".on_mouse_down(gpui::MouseButton::Left"));
    assert!(code.contains("_cx.stop_propagation();"));
    assert!(code.contains("open_menu();"));
    assert!(code.contains("source window controls replaced"));
    assert!(!compact_code.contains(".child(\"crop_square\")"));
    assert!(!compact_code.contains(".child(\"close\")"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_keeps_titlebar_action_groups_that_are_not_window_controls() {
    let compiled = Compiler::new().compile_fragment(
        r#"
            <div data-htmlswap-component="titlebar">
                <span>Unsaved File</span>
                <div style="margin-left:auto;display:flex;gap:4px;">
                    <button>Cancel</button>
                    <button>Close</button>
                </div>
            </div>
        "#,
        &CompileAssets::new(),
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("titlebar action groups should remain renderable children");
    let code = output.code();

    syn::parse_file(code).expect("titlebar action group output should be valid Rust");

    assert!(code.contains("gpui_component::TitleBar::new()"));
    assert!(code.contains("Cancel"));
    assert!(code.contains("Close"));
    assert!(!code.contains("source window controls replaced"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_does_not_treat_vanilla_titlebar_classes_as_semantics() {
    let compiled = Compiler::new().compile_fragment(
        r#"
            <div class="titlebar">
                <span>Aether</span>
                <div class="window-controls">
                    <button aria-label="Close">close</button>
                </div>
            </div>
        "#,
        &CompileAssets::new(),
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("vanilla class-only titlebar should still emit base GPUI");
    let code = output.code();

    syn::parse_file(code).expect("vanilla class-only titlebar output should be valid Rust");

    assert!(!code.contains("gpui_component::TitleBar::new()"));
    assert!(code.contains("gpui_component::button::Button::new"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_does_not_treat_vanilla_titlebar_comments_as_semantics() {
    let compiled = Compiler::new().compile_fragment(
        r#"
            <!-- titlebar -->
            <div>
                <span>Aether</span>
                <button aria-label="Close">close</button>
            </div>
        "#,
        &CompileAssets::new(),
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("vanilla titlebar comments should still emit base GPUI");
    let code = output.code();

    syn::parse_file(code).expect("vanilla comment titlebar output should be valid Rust");

    assert!(!code.contains("gpui_component::TitleBar::new()"));
    assert!(code.contains("gpui_component::button::Button::new"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_requires_sc_titlebar_comment_to_be_adjacent() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"
            <!-- titlebar -->
            not metadata for the next element
            <div>
                <span>Aether</span>
            </div>
        "#,
            &CompileAssets::new(),
        );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("non-adjacent SC titlebar comments should remain plain comments");
    let code = output.code();

    syn::parse_file(code).expect("non-adjacent SC comment output should be valid Rust");

    assert!(!code.contains("gpui_component::TitleBar::new()"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_warns_when_claimed_layer_falls_back_to_base() {
    let compiled = Compiler::new().compile_fragment(
        r#"<label id="name-label"><span>Name</span></label>"#,
        &CompileAssets::new(),
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("fallback should still emit base GPUI code");
    let code = output.code();

    syn::parse_file(code).expect("fallback output should be valid Rust");

    let diagnostics = adapter_context.diagnostics().as_slice();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].severity, Severity::Warning);
    assert!(
        diagnostics[0]
            .message
            .contains("route selected layer `gpui-components`"),
        "{diagnostics:?}"
    );
}

#[test]
fn gpui_components_adapter_maps_stateful_forms_and_text_inputs() {
    let compiled = Compiler::new().compile_fragment(
        r#"
            <form data-htmlswap-form-layout="horizontal" data-htmlswap-form-columns="2">
                <label for="name">Name</label>
                <input id="name" name="full_name" type="text" value="Ada" placeholder="Name" title="Full name" required>
                <textarea id="bio" rows="4" placeholder="Bio">About</textarea>
            </form>
        "#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit code");
    let code = output.code();
    let dependencies = output.dependencies();

    syn::parse_file(code).expect("GPUI components adapter should emit syntactically valid Rust");

    assert_eq!(dependencies.len(), 2);
    assert!(code.contains("name: gpui::Entity<gpui_component::input::InputState>"));
    assert!(code.contains("bio: gpui::Entity<gpui_component::input::InputState>"));
    assert!(code.contains("pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self"));
    assert!(code.contains("gpui_component::input::InputState::new(window, cx)"));
    assert!(code.contains(".placeholder(\"Name\")"));
    assert!(code.contains(".default_value(\"Ada\")"));
    assert!(code.contains("struct HtmlswapTooltipView"));
    assert!(code.contains("let htmlswap_tooltip = (\"Full name\").to_string();"));
    assert!(!code.contains("unmapped tooltip"));
    assert!(code.contains(".placeholder(\"Bio\")"));
    assert!(code.contains(".default_value(\"About\")"));
    assert!(code.contains(".multi_line(true)"));
    assert!(code.contains(".rows(4)"));
    assert!(code.contains("gpui_component::form::Form::horizontal()"));
    assert!(code.contains(".columns(2)"));
    assert!(code.contains("gpui_component::form::field()"));
    assert!(code.contains(".label(\"Name\")"));
    assert!(code.contains(".required(true)"));
    assert!(code.contains("gpui_component::input::Input::new(&self.name)"));
    assert!(code.contains("gpui_component::input::Input::new(&self.bio)"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_preserves_input_style_variants_that_component_cannot_emit() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        ".name:focus { border: 1px solid red; }",
    );
    let compiled = Compiler::new().compile_fragment(r#"<input class="name" value="Ada">"#, &assets);
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI components adapter should emit syntactically valid Rust");

    assert!(code.contains("gpui_component::input::Input::new(&self.input_0)"));
    assert!(!code.contains(".focus("));
    assert!(code.contains("// conditional CSS :focus .name:focus"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_emits_stateful_input_change_and_submit_actions() {
    let assets = CompileAssets::new().with_script(
        Some("app.js".to_owned()),
        "function save() {}\nfunction choose() {}",
    );
    let compiled = Compiler::new().compile_fragment(
        r#"
            <form onsubmit="save()">
                <label for="name">Name</label>
                <input id="name" name="full_name" value="Ada" oninput="save();">
                <select id="country" name="country" onchange="choose()">
                    <option value="us">United States</option>
                    <option value="ca" selected>Canada</option>
                </select>
                <input id="agree" type="checkbox" name="agree" checked onchange="save()">
            </form>
        "#,
        &assets,
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI components adapter should emit syntactically valid Rust");

    assert!(code.contains("name: gpui::Entity<gpui_component::input::InputState>"));
    assert!(code.contains("country: gpui::Entity<"));
    assert!(code.contains("agree: bool"));
    assert!(code.contains("_htmlswap_subscriptions: Vec<gpui::Subscription>"));
    assert!(!code.contains(".on_input("));
    assert!(code.contains("cx.subscribe_in("));
    assert!(code.contains("&this.name"));
    assert!(code.contains("gpui_component::input::InputEvent::Change"));
    assert!(code.contains("let _htmlswap_value = input.read(cx).value().to_string();"));
    assert!(!code.contains("// unmapped action: input=save();"));
    assert!(!code.contains("// action payload: element state `name`"));
    assert!(code.contains(".on_change(_cx.listener("));
    assert!(code.contains("this.country.update(_cx"));
    assert!(code.contains("state.set_selected(_event, _window, _cx);"));
    assert!(code.contains("this.agree = _event;"));
    assert!(code.contains("_cx.notify();"));
    assert!(code.contains(".checked(self.agree)"));
    assert!(code.contains(".on_submit(_cx.listener("));
    assert!(code.contains("std::collections::BTreeMap::new()"));
    assert!(code.contains("data.insert(\"full_name\""));
    assert!(code.contains("data.insert(\"country\""));
    assert!(code.contains("data.insert(\"agree\""));
    assert!(code.contains("save();"));
    assert!(code.contains("choose();"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_bridges_dc_source_owned_text_inputs() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"<x-dc>
                <input data-htmlswap-state-owner="source" value="{{ name }}" placeholder="{{ namePlaceholder }}" onInput="{{ onName }}">
                <input data-htmlswap-state-owner="source" value="{{ location }}" placeholder="{{ locationPlaceholder }}" onInput="{{ onLocation }}">
            </x-dc>"#,
            &CompileAssets::new(),
        );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI components adapter should emit syntactically valid Rust");

    assert_eq!(
        code.matches("gpui::Entity<gpui_component::input::InputState>")
            .count(),
        2
    );
    assert!(code.contains("_htmlswap_subscriptions: Vec<gpui::Subscription>"));
    assert!(code.contains("format!(\"{}\", this.name())"));
    assert!(code.contains("format!(\"{}\", this.location())"));
    assert!(code.contains("format!(\"{}\", this.name_placeholder())"));
    assert!(code.contains("format!(\"{}\", this.location_placeholder())"));
    assert!(code.contains("state.set_placeholder"));
    assert_eq!(code.matches("cx.subscribe_in(").count(), 2);
    assert!(code.contains("this.on_name(&_htmlswap_value, window, cx);"));
    assert!(code.contains("this.on_location(&_htmlswap_value, window, cx);"));
    assert!(!code.contains("source-owned and cannot be mutated"));
    assert!(!code.contains("unmapped action: input={{ onName }}"));
    assert!(!code.contains("{{ namePlaceholder }}"));
    assert!(!code.contains("{{ locationPlaceholder }}"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_bridges_loop_scoped_source_owned_text_inputs() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"<x-dc>
                <sc-for list="{{ props }}" as="p">
                    <input data-htmlswap-state-owner="source" value="{{ p.x }}" placeholder="{{ p.placeholder }}" onInput="{{ p.onX }}">
                </sc-for>
            </x-dc>"#,
            &CompileAssets::new(),
        );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut options = GpuiComponentsAdapterOptions::components();
    options.gpui.emit_debug_layout_ids = true;

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::new(options)
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI components adapter should emit syntactically valid Rust");

    assert!(code.contains("_htmlswap_component_text_inputs"));
    let compact_code = code.split_whitespace().collect::<String>();
    assert!(code.contains("_htmlswap_component_text_input_placeholders"));
    assert!(code.contains(".enumerate()"));
    assert!(code.contains("htmlswap_index_"));
    assert!(code.contains("format!(\"input_0:{}\""));
    assert!(code.contains("gpui::div().child("));
    assert!(code.contains("gpui_component::input::InputState::new(_window, cx)"));
    assert!(code.contains(".placeholder(htmlswap_input_placeholder.clone())"));
    assert!(code.contains("let htmlswap_input_placeholder"));
    assert!(code.contains("format!(\"{}\", p.placeholder"));
    assert!(code.contains("state.set_placeholder("));
    assert!(code.contains("htmlswap_input_placeholder.clone()"));
    assert!(code.contains("gpui_component::input::Input::new(&htmlswap_input)"));
    assert!(code.contains(".w_full()"));
    assert!(code.contains(".h_full()"));
    assert!(code.contains("htmlswap_debug_source_s0_"));
    assert!(code.contains("let htmlswap_capture_p = p.clone();"));
    assert!(compact_code.contains("htmlswap_capture_p.on_x(&_htmlswap_value,window,cx,);"));
    assert!(!code.contains("this.p().x"));
    assert!(!code.contains("{{ p.placeholder }}"));
    assert!(!code.contains("source-owned and cannot be mutated"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_maps_material_symbol_spans_to_icons() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"<x-dc>
                <style>
                    .ms {
                        font-family: 'Material Symbols Outlined';
                        font-weight: 400;
                        font-style: normal;
                        line-height: 1;
                        letter-spacing: normal;
                        text-transform: none;
                        white-space: nowrap;
                        direction: ltr;
                        -webkit-font-smoothing: antialiased;
                        font-variation-settings: 'opsz' 20,'wght' 400,'GRAD' 0,'FILL' 0;
                        vertical-align: middle;
                        user-select: none;
                    }
                </style>
                <span class="ms" style="font-size:16px;color:#fff;">search</span>
                <span class="ms" style="font-size:18px;color:{{ iconColor }};">{{ icon }}</span>
            </x-dc>"#,
            &CompileAssets::new(),
        );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit icon code");
    let code = output.code();
    let compact_code = code.split_whitespace().collect::<String>();

    syn::parse_file(code).expect("GPUI components icon output should be valid Rust");

    assert!(code.contains("struct HtmlswapMaterialSymbolIcon"));
    assert!(code.contains(r#"format!("icons/dc/{symbol}.svg")"#));
    assert!(code.contains("htmlswap_material_symbol_icon(\"search\")"));
    assert!(code.contains("htmlswap_material_symbol_icon(format!(\"{}\", self.icon()))"));
    assert!(
        output
            .artifact
            .files
            .iter()
            .any(|file| { file.path == "icons/dc/search.svg" && file.contents.contains("<svg") })
    );
    assert!(
        output
            .artifact
            .files
            .iter()
            .any(|file| { file.path == "icons/dc/help.svg" && file.contents.contains("<svg") })
    );
    assert!(code.contains(".text_size(gpui::px(16.0))"));
    assert!(code.contains(".text_color(gpui::rgb(0xFFFFFF))"));
    assert!(compact_code.contains(".child(htmlswap_material_symbol_icon(\"search\"))"));
    assert!(!compact_code.contains(".child(\"search\")"));
    assert!(!code.contains(".font_family(\"Material Symbols Outlined\")"));
    assert!(!code.contains("font-variation-settings"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_emits_static_parts_of_dc_dynamic_style_objects() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"
                <x-dc>
                    <div style="{{ item.rowStyle }}">Project</div>
                    <script data-dc-script>
                        class Component extends DCLogic {
                            renderVals() {
                                const item = {
                                    rowStyle: {
                                        display: 'flex',
                                        alignItems: 'center',
                                        height: '52px',
                                        padding: '0 12px',
                                        cursor: 'default',
                                        background: this.state.active ? '#111111' : '#222222'
                                    }
                                };
                                return { item };
                            }
                        }
                    </script>
                </x-dc>
            "#,
            &CompileAssets::new(),
        );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI components adapter should emit syntactically valid Rust");

    assert!(code.contains(".flex()"));
    assert!(code.contains(".items_center()"));
    assert!(code.contains(".h(gpui::px(52.0))"));
    assert!(code.contains(".pt(gpui::px(0.0))"));
    assert!(code.contains(".pr(gpui::px(12.0))"));
    assert!(code.contains(".pb(gpui::px(0.0))"));
    assert!(code.contains(".pl(gpui::px(12.0))"));
    assert!(code.contains(".cursor_default()"));
    assert!(code.contains("dynamic style: \"{{ item.rowStyle }}\" [item.rowStyle]"));
    assert!(!code.contains(".bg(gpui::rgb(0x111111))"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_emits_scoped_dc_generic_style_objects() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"
                <x-dc>
                    <sc-for list="{{ navItems }}" as="n">
                        <div style="{{ n.style }}">{{ n.label }}</div>
                    </sc-for>
                    <script data-dc-script>
                        class Component extends DCLogic {
                            renderVals() {
                                const navItems = navDefs.map(([id, label]) => {
                                    return {
                                        key: id,
                                        label,
                                        style: {
                                            display: 'flex',
                                            alignItems: 'center',
                                            gap: '10px',
                                            height: '36px',
                                            padding: '0 11px',
                                            borderRadius: '7px'
                                        }
                                    };
                                });
                                return { navItems };
                            }
                        }
                    </script>
                </x-dc>
            "#,
            &CompileAssets::new(),
        );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI components adapter should emit syntactically valid Rust");

    assert!(code.contains(".flex()"));
    assert!(code.contains(".items_center()"));
    assert!(code.contains(".gap(gpui::px(10.0))"));
    assert!(code.contains(".h(gpui::px(36.0))"));
    assert!(code.contains(".pr(gpui::px(11.0))"));
    assert!(code.contains(".pl(gpui::px(11.0))"));
    assert!(code.contains(".rounded(gpui::px(7.0))"));
    assert!(code.contains("dynamic style: \"{{ n.style }}\" [n.style]"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_preserves_inlined_dc_component_style_objects() {
    let compiler = Compiler::new().with_frontend(Frontend::dc());
    let assets = CompileAssets::new();
    let root = compiler.compile_fragment(
        r#"<x-dc><dc-import name="NavItem" hint-size="192px,36px"></dc-import></x-dc>"#,
        &assets,
    );
    assert!(
        root.diagnostics.is_empty(),
        "{:?}",
        root.diagnostics.as_slice()
    );

    let nav_item = compiler.compile_fragment(
        r#"
            <x-dc>
                <div style="{{ itemStyle }}">
                    <span>Recent Projects</span>
                </div>
                <script type="text/x-dc" data-dc-script>
                    class Component extends DCLogic {
                        renderVals() {
                            return {
                                itemStyle: {
                                    display: 'flex',
                                    alignItems: 'center',
                                    gap: '10px',
                                    height: '36px',
                                    padding: '0 11px',
                                    borderRadius: '7px',
                                    marginBottom: '2px'
                                }
                            };
                        }
                    }
                </script>
            </x-dc>
        "#,
        &assets,
    );
    assert!(
        nav_item.diagnostics.is_empty(),
        "{:?}",
        nav_item.diagnostics.as_slice()
    );

    let expanded = inline_dc_component_imports(
        root.value,
        [DcComponentFragment::new("NavItem", nav_item.value)],
    );
    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&expanded, &mut adapter_context)
        .expect("GPUI components adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI components adapter should emit syntactically valid Rust");

    assert!(code.contains(".flex()"), "{code}");
    assert!(code.contains(".items_center()"), "{code}");
    assert!(code.contains(".gap(gpui::px(10.0))"), "{code}");
    assert!(code.contains(".h(gpui::px(36.0))"), "{code}");
    assert!(code.contains(".pr(gpui::px(11.0))"), "{code}");
    assert!(code.contains(".pl(gpui::px(11.0))"), "{code}");
    assert!(code.contains(".rounded(gpui::px(7.0))"), "{code}");
    assert!(code.contains(".mb(gpui::px(2.0))"), "{code}");
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_emits_inlined_dc_component_dynamic_style_colors() {
    let compiler = Compiler::new().with_frontend(Frontend::dc());
    let assets = CompileAssets::new();
    let root = compiler.compile_fragment(
        r#"
            <x-dc>
                <sc-for list="{{ projects }}" as="p">
                    <dc-import name="ProjectRow" project="{{ p.raw }}"></dc-import>
                </sc-for>
            </x-dc>
        "#,
        &assets,
    );
    assert!(
        root.diagnostics.is_empty(),
        "{:?}",
        root.diagnostics.as_slice()
    );

    let row = compiler.compile_fragment(
        r#"
            <x-dc>
                <span style="{{ iconTileStyle }}">Icon</span>
                <script type="text/x-dc" data-dc-script>
                    class Component extends DCLogic {
                        renderVals() {
                            const pr = this.props.project || {};
                            return {
                                iconTileStyle: {
                                    width: '30px',
                                    height: '30px',
                                    background: pr.accent || '#4188e0',
                                    color: pr.foreground || '#ffffff'
                                }
                            };
                        }
                    }
                </script>
            </x-dc>
        "#,
        &assets,
    );
    assert!(
        row.diagnostics.is_empty(),
        "{:?}",
        row.diagnostics.as_slice()
    );

    let expanded = inline_dc_component_imports(
        root.value,
        [DcComponentFragment::new("ProjectRow", row.value)],
    );
    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&expanded, &mut adapter_context)
        .expect("GPUI components adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI components adapter should emit syntactically valid Rust");

    assert!(code.contains("fn htmlswap_dynamic_color"), "{code}");
    assert!(code.contains(".w(gpui::px(30.0))"), "{code}");
    assert!(code.contains(".h(gpui::px(30.0))"), "{code}");
    assert!(
        code.contains(".bg(htmlswap_dynamic_color(format!(\"{}\", p.raw.accent)))"),
        "{code}"
    );
    assert!(
        code.contains(".text_color(htmlswap_dynamic_color(format!(")
            && code.contains("p.raw.foreground"),
        "{code}"
    );
    assert!(code.contains("dynamic style: \"background:"), "{code}");
    assert!(code.contains("dynamic style: \"color:"), "{code}");
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_rewrites_inlined_dc_component_render_vals_to_import_props() {
    let compiler = Compiler::new().with_frontend(Frontend::dc());
    let assets = CompileAssets::new();
    let root = compiler.compile_fragment(
        r#"
            <x-dc>
                <sc-for list="{{ projects }}" as="p">
                    <dc-import name="ProjectRow" project="{{ p.raw }}" href="Editor.dc.html"></dc-import>
                </sc-for>
            </x-dc>
        "#,
        &assets,
    );
    assert!(
        root.diagnostics.is_empty(),
        "{:?}",
        root.diagnostics.as_slice()
    );

    let row = compiler.compile_fragment(
        r#"
            <x-dc>
                <a href="{{ href }}"><span>{{ name }}</span><span>{{ gems }}</span><span>{{ pinned }}</span></a>
                <script type="text/x-dc" data-dc-script>
                    class Component extends DCLogic {
                        renderVals() {
                            const pr = this.props.project || {};
                            return {
                                href: this.props.href || 'fallback.dc.html',
                                name: pr.name ?? 'Untitled',
                                gems: (pr.gems != null ? pr.gems : 0) + ' gems',
                                pinned: !!pr.pinned
                            };
                        }
                    }
                </script>
            </x-dc>
        "#,
        &assets,
    );
    assert!(
        row.diagnostics.is_empty(),
        "{:?}",
        row.diagnostics.as_slice()
    );

    let expanded = inline_dc_component_imports(
        root.value,
        [DcComponentFragment::new("ProjectRow", row.value)],
    );
    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&expanded, &mut adapter_context)
        .expect("GPUI components adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI components adapter should emit syntactically valid Rust");

    assert!(code.contains(".href(\"Editor.dc.html\")"), "{code}");
    assert!(code.contains("format!(\"{}\", p.raw.name)"), "{code}");
    assert!(code.contains("format!(\"{} gems\", p.raw.gems)"), "{code}");
    assert!(code.contains("format!(\"{}\", p.raw.pinned)"), "{code}");
    assert!(!code.contains("self.name()"), "{code}");
    assert!(!code.contains("self.gems()"), "{code}");
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_maps_selects_and_fieldsets() {
    let compiled = Compiler::new().compile_fragment(
        r#"
            <fieldset id="prefs">
                <legend>Preferences</legend>
                <button>Save</button>
            </fieldset>
            <select id="country" name="country" placeholder="Country" required>
                <option value="us">United States</option>
                <option value="ca" selected>Canada</option>
            </select>
        "#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit code");
    let code = output.code();
    let compact_code = code.split_whitespace().collect::<String>();

    syn::parse_file(code).expect("GPUI components adapter should emit syntactically valid Rust");

    assert!(code.contains("gpui_component::group_box::GroupBox::new()"));
    assert!(code.contains(".id(\"prefs\")"));
    assert!(code.contains(".title(gpui::div().child(\"Preferences\"))"));
    assert!(code.contains("gpui_component::button::Button::new("));
    assert!(compact_code.contains("country:gpui::Entity<gpui_component::select::SelectState<"));
    assert!(compact_code.contains("gpui_component::select::SearchableVec<String>"));
    assert!(code.contains("gpui_component::select::SelectState::new("));
    assert!(code.contains("String::from(\"United States\")"));
    assert!(code.contains("String::from(\"Canada\")"));
    assert!(code.contains("Some(gpui_component::IndexPath::new(1))"));
    assert!(code.contains("gpui_component::select::Select::new(&self.country)"));
    assert!(code.contains(".placeholder(\"Country\")"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_adapter_can_route_everything_to_gpui_base() {
    let compiled = Compiler::new().compile_fragment(
        r#"<button id="save" onclick="save()">Save</button>"#,
        &CompileAssets::new().with_script(Some("app.js".to_owned()), "function save() {}"),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::new(GpuiComponentsAdapterOptions {
        gpui: GpuiAdapterOptions::default(),
        routes: RouteConfig::new().with_base(GPUI_LAYER_ID),
    })
    .adapt(&compiled.value, &mut adapter_context)
    .expect("GPUI components adapter should emit code");
    let code = output.code();
    let dependencies = output.dependencies();

    syn::parse_file(code).expect("GPUI components adapter should emit syntactically valid Rust");

    assert_eq!(dependencies.len(), 1);
    assert_eq!(dependencies[0].package, "gpui");
    assert!(!code.contains("gpui_component"));
    assert!(!code.contains("Button::new"));
    assert!(code.contains("div()"));
    assert!(code.contains(".id(\"save\")"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_components_default_routes_expose_component_layer() {
    let options = GpuiComponentsAdapterOptions::default();

    assert_eq!(
        options
            .routes
            .layers
            .iter()
            .map(|layer| layer.id.as_str())
            .collect::<Vec<_>>(),
        [GPUI_LAYER_ID, GPUI_COMPONENTS_LAYER_ID]
    );
}

#[test]
fn gpui_components_adapter_exposes_html_import_api() {
    let compiled =
        Compiler::new().compile_fragment("<button>Imported</button>", &CompileAssets::new());
    let mut adapter_context = AdapterContext::new();
    let output = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI components adapter should emit code");
    assert!(adapter_context.diagnostics().is_empty());

    let input = TargetArtifact::from(output.artifact);
    let mut adapter_context = AdapterContext::new();
    let result = GpuiComponentsAdapter::default().import(&input, &mut adapter_context);

    assert!(
        result
            .expect("GPUI components importer should recover HTML")
            .html()
            .contains("Imported")
    );
}
