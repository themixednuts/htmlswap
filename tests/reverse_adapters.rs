use htmlswap::{
    Adapter, AdapterContext, CompileAssets, Compiler, Frontend, GpuiAdapter, GpuiComponentsAdapter,
    Importer, RoundTripOptions, TargetArtifact, compare_roundtrip_plans,
};

#[test]
fn gpui_importer_round_trips_basic_structure_to_html() {
    let compiled = Compiler::new().compile_fragment(
        r#"<section id="panel" style="display: flex; gap: 8px; color: #ff0000"><button id="save" onclick="save()">Save</button><img src="logo.png" alt="Logo"></section>"#,
        &CompileAssets::new().with_script(Some("app.js".to_owned()), "function save() {}"),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());

    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(recovered.contains(r#"<section id="panel" style="#));
    assert!(recovered.contains("display: flex"));
    assert!(recovered.contains("gap: 8px"));
    assert!(recovered.contains("color: #FF0000"));
    assert!(recovered.contains(r#"<button id="save" onclick="{{ save }}">"#));
    assert!(recovered.contains("Save"));
    assert!(recovered.contains(r#"<img src="logo.png" alt="Logo">"#));
}

#[test]
fn gpui_importer_restores_source_head_comments() {
    let source = r#"
        impl gpui::Render for View {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl gpui::IntoElement {
                // source head: <link href="/icons.css" rel="stylesheet">
                // source head: <style>.ms { font-family: 'Material Symbols Outlined'; }</style>
                gpui::div().child("Hi")
            }
        }
    "#;

    let artifact = TargetArtifact::new().with_file("view.rs", source);
    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&artifact, &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered.contains(r#"<link href="/icons.css" rel="stylesheet">"#),
        "{recovered}"
    );
    assert!(
        recovered.contains(r#"<style>.ms { font-family: 'Material Symbols Outlined'; }</style>"#),
        "{recovered}"
    );
}

#[test]
fn gpui_importer_restores_only_root_source_head_group() {
    let source = r#"
        impl gpui::Render for View {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl gpui::IntoElement {
                // source head: <meta charset="utf-8">
                // source head: <meta name="viewport" content="width=device-width, initial-scale=1">
                // source head: <style>.root { color: red; }</style>
                // source head: <link rel="stylesheet" href="/root-after-style.css">
                // source head: <meta charset="utf-8">
                // source head: <meta name="viewport" content="width=device-width, initial-scale=1">
                // source head: <style>.component { color: blue; }</style>
                gpui::div().child("Hi")
            }
        }
    "#;

    let artifact = TargetArtifact::new().with_file("view.rs", source);
    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&artifact, &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered.contains(r#"<style>.root { color: red; }</style>"#),
        "{recovered}"
    );
    assert!(
        recovered.contains(r#"<link rel="stylesheet" href="/root-after-style.css">"#),
        "{recovered}"
    );
    assert!(
        !recovered.contains(r#"<style>.component { color: blue; }</style>"#),
        "{recovered}"
    );
}

#[test]
fn gpui_importer_restores_preserved_white_space_pre() {
    let compiler = Compiler::new();
    let compiled = compiler.compile_fragment(
        r#"<pre style="white-space: pre">Line</pre>"#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());
    assert!(
        gpui.code().contains("preserved CSS: white-space: pre"),
        "{}",
        gpui.code()
    );

    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();
    let actual = compiler.compile_fragment(recovered, &CompileAssets::new());
    assert!(actual.diagnostics.is_empty());

    let comparison = compare_roundtrip_plans(
        &compiled.value.plan,
        &actual.value.plan,
        RoundTripOptions::new(),
    );
    assert!(comparison.is_match(), "{comparison}\n{recovered}");
}

#[test]
fn gpui_importer_removes_transform_margin_fallback_when_source_transform_is_preserved() {
    let source = r#"<div style="position: absolute; left: 50%; top: 50%; width: 346px; height: 20px; transform: translate(-50%, -50%)"></div>"#;
    let compiler = Compiler::new();
    let compiled = compiler.compile_fragment(source, &CompileAssets::new());
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());
    let code = gpui.code();
    assert!(code.contains("preserved CSS: transform: translate(-50%, -50%)"));
    assert!(code.contains(".ml(gpui::px(-173.0))"), "{code}");
    assert!(code.contains(".mt(gpui::px(-10.0))"), "{code}");

    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();
    assert!(
        recovered.contains("transform: translate(-50%, -50%)"),
        "{recovered}"
    );
    assert!(!recovered.contains("margin-left: -173px"), "{recovered}");
    assert!(!recovered.contains("margin-top: -10px"), "{recovered}");

    let actual = compiler.compile_fragment(recovered, &CompileAssets::new());
    assert!(actual.diagnostics.is_empty());
    let comparison = compare_roundtrip_plans(
        &compiled.value.plan,
        &actual.value.plan,
        RoundTripOptions::new(),
    );
    assert!(comparison.is_match(), "{comparison}\n{recovered}");
}

#[test]
fn gpui_importer_removes_calc_position_margin_fallback_when_source_position_is_preserved() {
    let source =
        r#"<div style="position: absolute; left: calc(50% + 22px); top: calc(50% - 5px)"></div>"#;
    let compiler = Compiler::new();
    let compiled = compiler.compile_fragment(source, &CompileAssets::new());
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());
    let code = gpui.code();
    assert!(
        code.contains("preserved CSS: left: calc(50% + 22px)"),
        "{code}"
    );
    assert!(
        code.contains("preserved CSS: top: calc(50% - 5px)"),
        "{code}"
    );
    assert!(code.contains(".ml(gpui::px(22.0))"), "{code}");
    assert!(code.contains(".mt(gpui::px(-5.0))"), "{code}");

    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();
    assert!(recovered.contains("left: calc(50% + 22px)"), "{recovered}");
    assert!(recovered.contains("top: calc(50% - 5px)"), "{recovered}");
    assert!(!recovered.contains("margin-left: 22px"), "{recovered}");
    assert!(!recovered.contains("margin-top: -5px"), "{recovered}");

    let actual = compiler.compile_fragment(recovered, &CompileAssets::new());
    assert!(actual.diagnostics.is_empty());
    let comparison = compare_roundtrip_plans(
        &compiled.value.plan,
        &actual.value.plan,
        RoundTripOptions::new(),
    );
    assert!(comparison.is_match(), "{comparison}\n{recovered}");
}

#[test]
fn gpui_importer_removes_inset_shadow_border_fallback_when_source_shadow_is_preserved() {
    let source = r#"<div style="box-shadow: inset 0 -2px #d6a23b"></div>"#;
    let compiler = Compiler::new();
    let compiled = compiler.compile_fragment(source, &CompileAssets::new());
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());
    let code = gpui.code();
    assert!(
        code.contains("preserved CSS: box-shadow: inset 0 -2px #d6a23b"),
        "{code}"
    );
    assert!(code.contains(".border_b(gpui::px(2.0))"), "{code}");
    assert!(
        code.contains(".border_color(gpui::rgb(0xD6A23B))"),
        "{code}"
    );

    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();
    assert!(
        recovered.contains("box-shadow: inset 0 -2px #d6a23b"),
        "{recovered}"
    );
    assert!(!recovered.contains("border-bottom-width"), "{recovered}");
    assert!(!recovered.contains("border-color"), "{recovered}");

    let actual = compiler.compile_fragment(recovered, &CompileAssets::new());
    assert!(actual.diagnostics.is_empty());
    let comparison = compare_roundtrip_plans(
        &compiled.value.plan,
        &actual.value.plan,
        RoundTripOptions::new(),
    );
    assert!(comparison.is_match(), "{comparison}\n{recovered}");
}

#[test]
fn gpui_components_importer_recovers_dynamic_source_input_actions_from_hints() {
    let source = r#"
        <x-dc>
            <sc-for list="{{ points }}" as="p">
                <input value="{{ p.x }}" onInput="{{ p.onX }}" style="width:20px" />
            </sc-for>
        </x-dc>
    "#;
    let compiler = Compiler::new().with_frontend(Frontend::dc());
    let compiled = compiler.compile_fragment(source, &CompileAssets::new());
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI components adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());
    let code = gpui.code();
    assert!(
        code.contains("template action: input=\"{{ p.onX }}\""),
        "{code}"
    );
    let compact_code = code.split_whitespace().collect::<String>();
    assert!(
        compact_code.contains("htmlswap_capture_p.on_x(&_htmlswap_value,window,cx,);"),
        "{code}"
    );

    let mut import_context = AdapterContext::new();
    let html = GpuiComponentsAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI components importer should recover HTML");
    let recovered = html.html();
    assert!(
        recovered.contains(r#"oninput="{{ p.onX }}""#),
        "{recovered}"
    );

    let actual = compiler.compile_fragment(recovered, &CompileAssets::new());
    assert!(actual.diagnostics.is_empty());
    let comparison = compare_roundtrip_plans(
        &compiled.value.plan,
        &actual.value.plan,
        RoundTripOptions::new(),
    );
    assert!(comparison.is_match(), "{comparison}\n{recovered}");
}

#[test]
fn strict_roundtrip_ignores_material_symbol_font_presentation_css() {
    let compiler = Compiler::new();
    let expected =
        compiler.compile_fragment(r#"<span class="ms">folder</span>"#, &CompileAssets::new());
    assert!(expected.diagnostics.is_empty());
    let actual = compiler.compile_fragment(
        r#"<span class="ms" style="font-family: 'Material Symbols Outlined'; font-weight: 400; font-style: normal; font-variation-settings: 'opsz' 20, 'wght' 400, 'GRAD' 0, 'FILL' 0; line-height: 1; letter-spacing: normal; text-transform: none; white-space: nowrap; direction: ltr; vertical-align: middle; user-select: none; -webkit-font-smoothing: antialiased">folder</span>"#,
        &CompileAssets::new(),
    );
    assert!(actual.diagnostics.is_empty());

    let comparison = compare_roundtrip_plans(
        &expected.value.plan,
        &actual.value.plan,
        RoundTripOptions::new(),
    );
    assert!(comparison.is_match(), "{comparison}");
}

#[test]
fn gpui_importer_restores_source_order_after_z_index_paint_order() {
    let source = r#"<div><div id="menu" style="position:absolute;z-index:80"></div><div id="backdrop" style="position:fixed;z-index:79"></div><div id="base"></div></div>"#;
    let compiler = Compiler::new();
    let compiled = compiler.compile_fragment(source, &CompileAssets::new());
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());
    let code = gpui.code().to_owned();
    assert!(
        code.find(r#".id("base")"#) < code.find(r#".id("backdrop")"#)
            && code.find(r#".id("backdrop")"#) < code.find(r#".id("menu")"#),
        "{code}"
    );
    assert!(code.contains("htmlswap source-order: 0"), "{code}");
    assert!(
        code.contains("htmlswap paint-order: 2 z-index: 80"),
        "{code}"
    );

    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();
    assert!(
        !recovered.contains("data-htmlswap-source-order"),
        "{recovered}"
    );

    let actual = compiler.compile_fragment(recovered, &CompileAssets::new());
    assert!(actual.diagnostics.is_empty());
    let comparison = compare_roundtrip_plans(
        &compiled.value.plan,
        &actual.value.plan,
        RoundTripOptions::new(),
    );
    assert!(comparison.is_match(), "{comparison}\n{recovered}");
}

#[test]
fn gpui_importer_restores_source_order_for_z_indexed_control_flow_wrapper() {
    let source = r#"<div><div id="menu" style="position:absolute;z-index:90"></div><sc-if value="{{ open }}"><div id="backdrop" style="position:fixed;z-index:85"></div></sc-if></div>"#;
    let compiler = Compiler::new().with_frontend(Frontend::dc());
    let compiled = compiler.compile_fragment(source, &CompileAssets::new());
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());
    let code = gpui.code().to_owned();
    assert!(
        code.find(r#".id("backdrop")"#) < code.find(r#".id("menu")"#),
        "{code}"
    );
    assert!(
        code.contains("htmlswap control-flow source-order: 1"),
        "{code}"
    );

    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();
    assert!(
        !recovered.contains("data-htmlswap-source-order"),
        "{recovered}"
    );

    let actual = compiler.compile_fragment(recovered, &CompileAssets::new());
    assert!(actual.diagnostics.is_empty());
    let comparison = compare_roundtrip_plans(
        &compiled.value.plan,
        &actual.value.plan,
        RoundTripOptions::new(),
    );
    assert!(comparison.is_match(), "{comparison}\n{recovered}");
}

#[test]
fn gpui_importer_recovers_generated_title_tooltips() {
    let compiled = Compiler::new().compile_fragment(
        r#"<div title="Open project">Open</div>"#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());

    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(recovered.contains(r#"title="Open project""#), "{recovered}");
}

#[test]
fn gpui_importer_recovers_dynamic_title_tooltips() {
    let compiled = Compiler::new().with_frontend(Frontend::dc()).compile_fragment(
        r#"<sc-for list="{{ tools }}" as="tool"><div title="{{ tool.title }}">{{ tool.label }}</div></sc-for>"#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());

    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered.contains(r#"title="{{ tool.title }}""#),
        "{recovered}"
    );
}

#[test]
fn gpui_importer_recovers_preserved_dynamic_style_bindings() {
    let compiled = Compiler::new().with_frontend(Frontend::dc()).compile_fragment(
        r#"<x-dc><div style="{{ n.style }}" style-hover="{{ n.hoverStyle }}">Item</div></x-dc>"#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());

    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered.contains(r#"style="{{ n.style }}""#),
        "{recovered}"
    );
    assert!(
        recovered.contains(r#"style-hover="{{ n.hoverStyle }}""#),
        "{recovered}"
    );
}

#[test]
fn gpui_importer_preserves_dynamic_style_metadata_without_dropping_static_styles() {
    let source = r#"
        struct View;

        impl gpui::Render for View {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl gpui::IntoElement {
                // dynamic style: "{{ n.style }}" [n.style]
                gpui::div().bg(gpui::rgb(0x15171B))
            }
        }
    "#;

    let artifact = TargetArtifact::new().with_file("view.rs", source);
    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&artifact, &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered.contains("background-color: #15171B"),
        "{recovered}"
    );
    assert!(
        recovered.contains(r#"data-htmlswap-style="{{ n.style }}""#),
        "{recovered}"
    );
}

#[test]
fn gpui_importer_recovers_raw_svg_placeholders() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"<svg data-htmlswap-raw style="position:absolute;left:0;top:0"><sc-for list="{{ lines }}" as="line"><line x1="{{ line.x1 }}" y1="0" /></sc-for></svg>"#,
            &CompileAssets::new(),
        );
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());

    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(recovered.contains("<svg data-htmlswap-raw"), "{recovered}");
    assert!(
        recovered.contains(r#"<sc-for list="{{ lines }}" as="line">"#),
        "{recovered}"
    );
    assert!(
        recovered.contains(r#"<line x1="{{ line.x1 }}" y1="0""#),
        "{recovered}"
    );
}

#[test]
fn gpui_components_importer_composes_component_recognizers_with_base_gpui() {
    let compiled = Compiler::new().compile_fragment(
        r#"<div style="display: flex; gap: 8px"><button id="save" data-htmlswap-tone="accent" data-htmlswap-variant="outline">Save</button><a id="docs" href="/docs">Docs</a></div>"#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI components adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());

    let mut import_context = AdapterContext::new();
    let html = GpuiComponentsAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI components importer should recover HTML");
    let recovered = html.html();

    assert!(recovered.contains(r#"<button id="save""#));
    assert!(recovered.contains(r#"data-htmlswap-tone="accent""#));
    assert!(recovered.contains(r#"data-htmlswap-variant="outline""#));
    assert!(recovered.contains("Save"));
    assert!(recovered.contains(r#"<a id="docs" href="/docs">"#));
    assert!(recovered.contains("Docs"));
    assert!(recovered.contains("display: flex"));
}

#[test]
fn gpui_components_importer_recovers_component_title_tooltips() {
    let compiled = Compiler::new().compile_fragment(
        r#"<button title="Save now">Save</button>"#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI components adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());

    let mut import_context = AdapterContext::new();
    let html = GpuiComponentsAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI components importer should recover HTML");
    let recovered = html.html();

    assert!(recovered.contains(r#"title="Save now""#), "{recovered}");
}

#[test]
fn gpui_importer_collapses_generated_base_text_input_wrapper() {
    let compiled = Compiler::new().compile_fragment(
        r#"<input id="project-name" value="Aether" placeholder="Project name">"#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());

    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered
            .contains(r#"<input id="project-name" value="Aether" placeholder="Project name">"#),
        "{recovered}"
    );
}

#[test]
fn gpui_importer_recovers_stateful_text_input_titles() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"<x-dc>
                <input id="project-name" data-htmlswap-state-owner="source" value="{{ name }}" placeholder="Project name" title="Rename project" onInput="{{ onName }}">
            </x-dc>"#,
            &CompileAssets::new(),
        );
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());

    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered.contains(r#"title="Rename project""#),
        "{recovered}"
    );
    assert!(
        recovered.contains(r#"oninput="{{ onName }}""#),
        "{recovered}"
    );
}

#[test]
fn gpui_importer_recovers_multi_side_layout_helpers() {
    let source = r#"
        struct View;

        impl gpui::Render for View {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl gpui::IntoElement {
                gpui::div()
                    .size_full()
                    .px(gpui::px(8.0))
                    .py(gpui::px(4.0))
                    .mx(gpui::px(2.0))
                    .my(gpui::px(6.0))
                    .border_t(gpui::px(1.0))
                    .border_r(gpui::px(2.0))
                    .border_b(gpui::px(3.0))
                    .border_l(gpui::px(4.0))
            }
        }
    "#;

    let artifact = TargetArtifact::new().with_file("view.rs", source);
    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&artifact, &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    for expected in [
        "width: 100%",
        "height: 100%",
        "padding-left: 8px",
        "padding-right: 8px",
        "padding-top: 4px",
        "padding-bottom: 4px",
        "margin-left: 2px",
        "margin-right: 2px",
        "margin-top: 6px",
        "margin-bottom: 6px",
        "border-top-width: 1px",
        "border-right-width: 2px",
        "border-bottom-width: 3px",
        "border-left-width: 4px",
    ] {
        assert!(recovered.contains(expected), "{recovered}");
    }
}

#[test]
fn gpui_importer_recovers_richer_generated_styles() {
    let source = r#"
        struct View;

        impl gpui::Render for View {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl gpui::IntoElement {
                gpui::div()
                    .content_start()
                    .overflow_y_scroll()
                    .flex_grow()
                    .flex_shrink_0()
                    .flex_basis(gpui::px(212.0))
                    .line_height(gpui::relative(1.4))
                    .cursor_pointer()
                    .border(gpui::px(1.0))
                    .border_dashed()
                    .border_color(gpui::rgb(0x2C313A))
                    .bg(gpui::linear_gradient(
                        135.0,
                        gpui::linear_color_stop(gpui::rgb(0x3160A8), 0.0),
                        gpui::linear_color_stop(gpui::rgb(0x5AC0C0), 1.0),
                    ))
                    .shadow(std::vec![gpui::BoxShadow {
                        color: gpui::rgba(0x00000080),
                        offset: gpui::point(gpui::px(0.0), gpui::px(12.0)),
                        blur_radius: gpui::px(30.0),
                        spread_radius: gpui::px(0.0),
                    }])
                    .map(|mut this| { this.style().aspect_ratio = Some(1.0); this })
            }
        }
    "#;

    let artifact = TargetArtifact::new().with_file("view.rs", source);
    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&artifact, &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    for expected in [
        "align-content: start",
        "overflow-y: scroll",
        "flex-grow: 1",
        "flex-shrink: 0",
        "flex-basis: 212px",
        "line-height: 140%",
        "cursor: pointer",
        "border-width: 1px",
        "border-top-style: dashed",
        "border-right-style: dashed",
        "border-bottom-style: dashed",
        "border-left-style: dashed",
        "border-color: #2C313A",
        "background: linear-gradient(135deg, #3160A8, #5AC0C0)",
        "box-shadow: 0 12px 30px #00000080",
        "aspect-ratio: 1",
    ] {
        assert!(recovered.contains(expected), "{recovered}");
    }
}

#[test]
fn gpui_importer_recovers_stock_display_grid_text_and_inset_helpers() {
    let source = r#"
        struct View;

        impl gpui::Render for View {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl gpui::IntoElement {
                gpui::div()
                    .block()
                    .grid()
                    .grid_cols(3)
                    .flex_col_reverse()
                    .flex_wrap_reverse()
                    .items_baseline()
                    .justify_around()
                    .inset(gpui::px(0.0))
                    .whitespace_nowrap()
                    .underline()
                    .line_through()
            }
        }
    "#;

    let artifact = TargetArtifact::new().with_file("view.rs", source);
    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&artifact, &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    for expected in [
        "display: block",
        "display: grid",
        "grid-template-columns: repeat(3, 1fr)",
        "flex-direction: column-reverse",
        "flex-wrap: wrap-reverse",
        "align-items: baseline",
        "justify-content: space-around",
        "inset: 0px",
        "white-space: nowrap",
        "text-decoration: underline",
        "text-decoration: line-through",
    ] {
        assert!(recovered.contains(expected), "{recovered}");
    }
}

#[test]
fn gpui_importer_uses_preserved_css_comments_after_generated_methods() {
    let source = r#"
        struct View;

        impl gpui::Render for View {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl gpui::IntoElement {
                // unmapped CSS: display: inline-flex
                // unmapped CSS: letter-spacing: 0.02em
                // conditional CSS :hover [style-hover] {filter: brightness(1.1)} (one or more declarations are not mapped to GPUI style methods)
                gpui::div()
                    .flex()
            }
        }
    "#;

    let artifact = TargetArtifact::new().with_file("view.rs", source);
    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&artifact, &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(recovered.contains("display: inline-flex"), "{recovered}");
    assert!(recovered.contains("letter-spacing: .02em"), "{recovered}");
    assert!(
        recovered.contains(r#"style-hover="filter: brightness(1.1)""#),
        "{recovered}"
    );
    assert!(!recovered.contains("display: flex"), "{recovered}");
}

#[test]
fn gpui_importer_preserved_dynamic_style_replaces_generated_property_fallbacks() {
    let source = r#"
        struct View;

        impl gpui::Render for View {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl gpui::IntoElement {
                // dynamic style: "font-size:24px;color:{{ iconColor }};" [iconColor]
                gpui::div()
                    .text_size(gpui::px(24.0))
                    .text_color(htmlswap_dynamic_color(format!("{}", self.icon_color())))
            }
        }
    "#;

    let artifact = TargetArtifact::new().with_file("view.rs", source);
    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&artifact, &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered.contains(r#"style="font-size:24px;color:{{ iconColor }};""#),
        "{recovered}"
    );
    assert!(
        !recovered.contains(r#"style="font-size: 24px""#),
        "{recovered}"
    );
}

#[test]
fn gpui_importer_preserved_background_replaces_generated_fallbacks() {
    let source = r#"
        struct View;

        impl gpui::Render for View {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl gpui::IntoElement {
                // preserved CSS: background: radial-gradient(circle at 34% 28%, #c9ccd2 0%, #24272c 100%)
                gpui::div()
                    .bg(gpui::rgb(0xC9CCD2))
            }
        }
    "#;

    let artifact = TargetArtifact::new().with_file("view.rs", source);
    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&artifact, &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered
            .contains("background: radial-gradient(circle at 34% 28%, #c9ccd2 0%, #24272c 100%)"),
        "{recovered}"
    );
    assert!(!recovered.contains("background-color"), "{recovered}");
}

#[test]
fn gpui_importer_recovers_negative_length_arguments() {
    let source = r#"
        struct View;

        impl gpui::Render for View {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl gpui::IntoElement {
                gpui::div()
                    .ml(gpui::px(-5.0))
                    .mt(gpui::px(-4.0))
            }
        }
    "#;

    let artifact = TargetArtifact::new().with_file("view.rs", source);
    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&artifact, &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(recovered.contains("margin-left: -5px"), "{recovered}");
    assert!(recovered.contains("margin-top: -4px"), "{recovered}");
}

#[test]
fn gpui_importer_ignores_debug_source_html_comments() {
    let source = r#"
        struct View;

        impl gpui::Render for View {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl gpui::IntoElement {
                {
                    // source html: <section data-note="literal gpui::div() in source">
                    gpui::div().child("Debug")
                }
            }
        }
    "#;

    let artifact = TargetArtifact::new().with_file("view.rs", source);
    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&artifact, &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert_eq!(recovered.matches("<div>").count(), 1, "{recovered}");
    assert!(recovered.contains("Debug"), "{recovered}");
}

#[test]
fn gpui_importer_unwraps_generated_loop_if_wrapper() {
    let source = r#"
        struct View;

        impl gpui::Render for View {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl gpui::IntoElement {
                gpui::div()
                    // source control-flow: for, expr=items, item=item, placeholder=hint-placeholder-count="2"
                    .children(
                        self.items().iter().map(|item| {
                            let item = item.clone();
                            gpui::div()
                                // source control-flow: if, expr=item.visible, placeholder=hint-placeholder-val="{{ true }}"
                                .when(item.visible, |this| {
                                    this.child({
                                        // source tag: div
                                        gpui::div().child(format!("{}", item.label))
                                    })
                                })
                        })
                    )
            }
        }
    "#;

    let artifact = TargetArtifact::new().with_file("view.rs", source);
    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&artifact, &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered.contains(r#"<sc-for list="{{ items }}" as="item" hint-placeholder-count="2">"#),
        "{recovered}"
    );
    assert!(
        recovered
            .contains(r#"<sc-if value="{{ item.visible }}" hint-placeholder-val="{{ true }}">"#),
        "{recovered}"
    );
    assert!(!recovered.contains("<sc-for list=\"{{ items }}\" as=\"item\" hint-placeholder-count=\"2\">\n    <div>\n      <sc-if"), "{recovered}");
}

#[test]
fn gpui_components_importer_collapses_material_symbol_icon_wrapper() {
    let source = r#"
        struct View;

        impl gpui::Render for View {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl gpui::IntoElement {
                {
                    // source tag: span
                    // source classes: ms
                    gpui::div()
                        .font_family("Material Symbols Outlined")
                        .child(htmlswap_material_symbol_icon("search"))
                }
            }
        }
    "#;

    let artifact = TargetArtifact::new().with_file("view.rs", source);
    let mut import_context = AdapterContext::new();
    let html = GpuiComponentsAdapter::default()
        .import(&artifact, &mut import_context)
        .expect("GPUI components importer should recover HTML");
    let recovered = html.html();

    assert_eq!(recovered.matches(r#"class="ms""#).count(), 1, "{recovered}");
    assert!(recovered.contains("search"), "{recovered}");
}

#[test]
fn gpui_components_importer_preserves_parent_when_collapsing_material_symbol_child() {
    let source = r#"
        struct View;

        impl gpui::Render for View {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl gpui::IntoElement {
                {
                    // source tag: div
                    gpui::div()
                        .flex()
                        .child({
                            // source tag: span
                            // html classes: ms
                            // preserved CSS: display: inline-block
                            gpui::div()
                                .text_size(gpui::px(17.0))
                                .child(htmlswap_material_symbol_icon("folder_open"))
                        })
                        .child("Open")
                }
            }
        }
    "#;

    let artifact = TargetArtifact::new().with_file("view.rs", source);
    let mut import_context = AdapterContext::new();
    let html = GpuiComponentsAdapter::default()
        .import(&artifact, &mut import_context)
        .expect("GPUI components importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered.contains(r#"<div style="display: flex">"#),
        "{recovered}"
    );
    assert_eq!(recovered.matches(r#"class="ms""#).count(), 1, "{recovered}");
    assert!(recovered.contains(r#"<span class="ms""#), "{recovered}");
    assert!(recovered.contains("display: inline-block"), "{recovered}");
    assert!(recovered.contains("font-size: 17px"), "{recovered}");
    assert!(recovered.contains("folder_open"), "{recovered}");
    assert!(recovered.contains("Open"), "{recovered}");
    assert!(!recovered.contains(r#"<div class="ms""#), "{recovered}");
}

#[test]
fn gpui_importer_recovers_control_flow_source_hints() {
    let source = r#"
        struct View;

        impl gpui::Render for View {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl gpui::IntoElement {
                gpui::div()
                    .children(
                        // source control-flow: for, expr=animIK.fields, item=f, placeholder=hint-placeholder-count="4"
                        self.anim_ik_fields().iter().map(|f| {
                            gpui::div().child(format!("{}", f.label))
                        }).collect::<Vec<_>>()
                    )
            }
        }
    "#;

    let artifact = TargetArtifact::new().with_file("view.rs", source);
    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&artifact, &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered
            .contains(r#"<sc-for list="{{ animIK.fields }}" as="f" hint-placeholder-count="4">"#),
        "{recovered}"
    );
}

#[test]
fn gpui_importer_recovers_gpui_event_methods() {
    let source = r#"
        struct View;

        impl gpui::Render for View {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl gpui::IntoElement {
                gpui::div()
                    .on_any_mouse_down(_cx.listener(move |this, _event, _window, _cx| {
                        this.drag_start(_event, _window, _cx);
                    }))
                    .on_input(_cx.listener(move |this, _event, _window, _cx| {
                        this.rename(_event, _window, _cx);
                    }))
                    .on_change(_cx.listener(move |this, _event, _window, _cx| {
                        this.commit_name(_event, _window, _cx);
                    }))
            }
        }
    "#;

    let artifact = TargetArtifact::new().with_file("view.rs", source);
    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&artifact, &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered.contains(r#"onmousedown="{{ dragStart }}""#),
        "{recovered}"
    );
    assert!(
        recovered.contains(r#"oninput="{{ rename }}""#),
        "{recovered}"
    );
    assert!(
        recovered.contains(r#"onchange="{{ commitName }}""#),
        "{recovered}"
    );
}

#[test]
fn gpui_components_importer_recovers_component_text_inputs() {
    let compiled = Compiler::new().compile_fragment(
        r#"<input id="project-name" value="Aether" placeholder="Project name" type="password">"#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI components adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());

    let mut import_context = AdapterContext::new();
    let html = GpuiComponentsAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI components importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered.contains(
            r#"<input id="project-name" value="Aether" placeholder="Project name" type="password">"#
        ),
        "{recovered}"
    );
}

#[test]
fn gpui_importer_preserves_source_owned_text_input_change_event() {
    let compiled = Compiler::new().with_frontend(Frontend::dc()).compile_fragment(
        r#"<input data-htmlswap-state-owner="source" value="{{ animSample.label }}" onChange="{{ animSample.onRename }}" />"#,
        &CompileAssets::new(),
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI adapter should emit Rust");
    let code = gpui.code();
    assert!(
        code.contains("template action: change=\"{{ animSample.onRename }}\""),
        "{code}"
    );

    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered.contains(r#"onchange="{{ animSample.onRename }}""#),
        "{recovered}"
    );
    assert!(!recovered.contains(r#"oninput="{{ animSample.onRename }}""#));
}

#[test]
fn gpui_importer_recovers_click_count_guard_as_doubleclick() {
    let compiled = Compiler::new().with_frontend(Frontend::dc()).compile_fragment(
        r#"<sc-for list="{{ nodes }}" as="n"><div onDoubleClick="{{ n.onOpen }}">{{ n.name }}</div></sc-for>"#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());

    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered.contains(r#"ondoubleclick="{{ n.onOpen }}""#),
        "{recovered}"
    );
    assert!(
        !recovered.contains(r#"onclick="{{ n.onOpen }}""#),
        "{recovered}"
    );
}

#[test]
fn gpui_importer_recovers_source_metadata_comments() {
    let compiled = Compiler::new().with_frontend(Frontend::dc()).compile_fragment(
        r#"<div data-htmlswap-region="project.manager.nav" data-htmlswap-component="nav-item" data-htmlswap-key="{{ n.label }}" data-htmlswap-tone="neutral" data-htmlswap-variant="ghost" href="Aether Editor.dc.html">Open</div>"#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());

    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered.contains(r#"data-htmlswap-region="project.manager.nav""#),
        "{recovered}"
    );
    assert!(
        recovered.contains(r#"data-htmlswap-component="nav-item""#),
        "{recovered}"
    );
    assert!(
        recovered.contains(r#"data-htmlswap-key="{{ n.label }}""#),
        "{recovered}"
    );
    assert!(
        recovered.contains(r#"data-htmlswap-tone="neutral""#),
        "{recovered}"
    );
    assert!(
        recovered.contains(r#"data-htmlswap-variant="ghost""#),
        "{recovered}"
    );
    assert!(
        recovered.contains(r#"href="Aether Editor.dc.html""#),
        "{recovered}"
    );
    assert!(!recovered.contains(r#"id="{{ n.label }}""#), "{recovered}");
}

#[test]
fn gpui_importer_keeps_complex_clickable_containers_as_divs() {
    let compiled = Compiler::new().with_frontend(Frontend::dc()).compile_fragment(
        r#"<div onClick="{{ row.open }}"><button onClick="{{ row.toggle }}">Toggle</button></div>"#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());

    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered.contains(r#"<div id="htmlswap_interactive_0" onclick="{{ row.open }}">"#),
        "{recovered}"
    );
    assert!(
        !recovered.contains(r#"<button onclick="{{ row.open }}">"#),
        "{recovered}"
    );
}

#[test]
fn gpui_components_importer_collapses_source_input_wrappers() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"<input value="{{ gdTableName }}" onInput="{{ gdOnRenameTable }}" spellcheck="false" title="Rename table" style="font-size:13.5px;border:1px solid transparent;margin-left:-5px;max-width:100%;" style-hover="background:#16181c;border-color:#2c313a;" style-focus="background:#16181c;border-color:#3160a8;"/>"#,
            &CompileAssets::new(),
        );
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI components adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());

    let mut import_context = AdapterContext::new();
    let html = GpuiComponentsAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI components importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered.contains(r#"<input value="{{ gdTableName }}""#),
        "{recovered}"
    );
    assert!(
        recovered.contains(r#"oninput="{{ gdOnRenameTable }}""#),
        "{recovered}"
    );
    assert!(recovered.contains(r#"spellcheck="false""#), "{recovered}");
    assert!(recovered.contains(r#"title="Rename table""#), "{recovered}");
    assert!(recovered.contains("margin-left: -5px"), "{recovered}");
    assert!(recovered.contains("max-width: 100%"), "{recovered}");
    assert!(!recovered.contains("<input><input"), "{recovered}");
}

#[test]
fn gpui_importer_unwraps_generated_multi_child_loop_body() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"<sc-for list="{{ steps }}" as="s"><span>{{ s.label }}</span><span data-htmlswap-style="{{ s.connector }}"></span></sc-for>"#,
            &CompileAssets::new(),
        );
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());

    let mut import_context = AdapterContext::new();
    let html = GpuiAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered.contains(r#"<sc-for list="{{ steps }}" as="s">"#),
        "{recovered}"
    );
    assert!(recovered.contains(r#"<span>"#), "{recovered}");
    assert!(
        recovered.contains(r#"<span style="{{ s.connector }}"></span>"#),
        "{recovered}"
    );
    assert!(
        !recovered.contains("<sc-for list=\"{{ steps }}\" as=\"s\">\n    <div>"),
        "{recovered}"
    );
}

#[test]
fn gpui_components_importer_keeps_source_input_hint_through_generated_setup_block() {
    let source = r#"
        struct View {
            _htmlswap_subscriptions: Vec<gpui::Subscription>,
        }

        impl gpui::Render for View {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl gpui::IntoElement {
                {
                    // source tag: input
                    {
                        let htmlswap_input_key = format!("input_0:{}", 1);
                        let htmlswap_input_value = (format!("{}", row.value)).to_string();
                        let htmlswap_input_placeholder = (format!("{}", row.placeholder)).to_string();
                        let htmlswap_input = if let Some(htmlswap_input) = self._htmlswap_component_text_inputs.get(&htmlswap_input_key) {
                            htmlswap_input.clone()
                        } else {
                            let htmlswap_input = _cx.new(|cx| {
                                gpui_component::input::InputState::new(_window, cx).default_value(htmlswap_input_value.clone()).placeholder(htmlswap_input_placeholder.clone())
                            });
                            let htmlswap_subscription = _cx.subscribe_in(&htmlswap_input, _window, {
                                let row = row.clone();
                                move |this: &mut Self, input, event: &gpui_component::input::InputEvent, window, cx| {
                                    if matches!(event, gpui_component::input::InputEvent::Change) {
                                        let _htmlswap_value = input.read(cx).value().to_string();
                                        row.on_value(&_htmlswap_value, window, cx);
                                    }
                                }
                            });
                            self._htmlswap_subscriptions.push(htmlswap_subscription);
                            htmlswap_input
                        };
                        gpui::div().child(gpui_component::input::Input::new(&htmlswap_input).w_full().h_full())
                    }
                        .id("htmlswap_interactive_0")
                        .text_size(gpui::px(12.0))
                }
            }
        }
    "#;

    let artifact = TargetArtifact::new().with_file("view.rs", source);
    let mut import_context = AdapterContext::new();
    let html = GpuiComponentsAdapter::default()
        .import(&artifact, &mut import_context)
        .expect("GPUI components importer should recover HTML");
    let recovered = html.html();

    assert!(recovered.contains(r#"<input"#), "{recovered}");
    assert!(
        !recovered.contains(r#"data-htmlswap-state-owner="source""#),
        "{recovered}"
    );
    assert!(
        recovered.contains(r#"value="{{ row.value }}""#),
        "{recovered}"
    );
    assert!(
        recovered.contains(r#"placeholder="{{ row.placeholder }}""#),
        "{recovered}"
    );
    assert!(recovered.contains("font-size: 12px"), "{recovered}");
    assert!(!recovered.contains("<div>\n  <input"), "{recovered}");
}

#[test]
fn gpui_components_importer_recovers_field_backed_dynamic_input_placeholder() {
    let source = r#"
        struct View {
            input_0: gpui::Entity<gpui_component::input::InputState>,
        }

        impl View {
            fn new(window: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> Self {
                let mut this = Self {
                    input_0: cx.new(|cx| gpui_component::input::InputState::new(window, cx)),
                };
                let htmlswap_initial_input_0 = format!("{}", this.wiz().proj_type);
                this.input_0.update(cx, |state, cx| {
                    state.set_value(htmlswap_initial_input_0.clone().into(), window, cx);
                });
                let htmlswap_placeholder_input_0 = format!("{}", this.wiz().proj_placeholder);
                this.input_0.update(cx, |state, cx| {
                    state.set_placeholder(htmlswap_placeholder_input_0.clone().into(), window, cx);
                });
                this
            }
        }

        impl gpui::Render for View {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl gpui::IntoElement {
                {
                    // source tag: input
                    // unmapped attribute: spellcheck="false"
                    gpui::div().child(gpui_component::input::Input::new(&self.input_0).w_full().h_full())
                }
            }
        }
    "#;

    let artifact = TargetArtifact::new().with_file("view.rs", source);
    let mut import_context = AdapterContext::new();
    let html = GpuiComponentsAdapter::default()
        .import(&artifact, &mut import_context)
        .expect("GPUI components importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered.contains(r#"value="{{ wiz.projType }}""#),
        "{recovered}"
    );
    assert!(
        recovered.contains(r#"placeholder="{{ wiz.projPlaceholder }}""#),
        "{recovered}"
    );
    assert!(recovered.contains(r#"spellcheck="false""#), "{recovered}");
}

#[test]
fn gpui_components_importer_recovers_material_symbol_class() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"<span class="ms" style="font-family: Material Symbols Outlined">search</span>"#,
            &CompileAssets::new(),
        );
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI components adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());

    let mut import_context = AdapterContext::new();
    let html = GpuiComponentsAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI components importer should recover HTML");
    let recovered = html.html();

    assert!(recovered.contains(r#"class="ms""#), "{recovered}");
    assert!(recovered.contains("search"), "{recovered}");
    assert!(!recovered.contains("data-htmlswap-icon"), "{recovered}");
}

#[test]
fn gpui_components_importer_restores_substituted_titlebar_window_controls() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r##"
            <div class="titlebar" data-htmlswap-component="titlebar" style="display:flex;align-items:center">
                <span>Aether</span>
                <div class="window-controls" data-htmlswap-slot="window-controls" style="display:flex;margin-left:auto;gap:2px">
                    <span class="ms" style="font-family:Material Symbols Outlined;width:28px;height:24px" style-hover="background-color:#2c313a">remove</span>
                    <span class="ms" style="font-family:Material Symbols Outlined;width:28px;height:24px" style-hover="background-color:#2c313a">crop_square</span>
                    <span class="ms" style="font-family:Material Symbols Outlined;width:28px;height:24px" style-hover="background-color:#c0453f;color:#fff">close</span>
                </div>
            </div>
            "##,
            &CompileAssets::new(),
        );
    assert!(compiled.diagnostics.is_empty());

    let mut adapt_context = AdapterContext::new();
    let gpui = GpuiComponentsAdapter::default()
        .adapt(&compiled.value, &mut adapt_context)
        .expect("GPUI components adapter should emit Rust");
    assert!(adapt_context.diagnostics().is_empty());
    assert!(gpui.code().contains("gpui_component::TitleBar::new()"));
    assert!(
        gpui.code()
            .contains("htmlswap raw child HTML omitted: <div")
    );

    let mut import_context = AdapterContext::new();
    let html = GpuiComponentsAdapter::default()
        .import(&TargetArtifact::from(gpui.artifact), &mut import_context)
        .expect("GPUI components importer should recover HTML");
    let recovered = html.html();

    assert!(
        recovered.contains(r#"data-htmlswap-slot="window-controls""#),
        "{recovered}"
    );
    assert!(recovered.contains("remove"), "{recovered}");
    assert!(recovered.contains("crop_square"), "{recovered}");
    assert!(recovered.contains("close"), "{recovered}");
    assert!(
        recovered.contains(r#"style-hover="background-color: #c0453f; color: #fff""#),
        "{recovered}"
    );
}

#[test]
fn gpui_components_importer_recovers_tab_bar_and_tabs() {
    let source = r#"
        struct View;

        impl gpui::Render for View {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl gpui::IntoElement {
                gpui_component::tab::TabBar::new("views")
                    .underline()
                    .with_size(gpui_component::Size::Small)
                    .child(
                        gpui_component::tab::Tab::new()
                            .selected(true)
                            .label("Scene")
                    )
                    .child(
                        gpui_component::tab::Tab::new()
                            .disabled(true)
                            .label("Game")
                    )
            }
        }
    "#;

    let artifact = TargetArtifact::new().with_file("view.rs", source);
    let mut import_context = AdapterContext::new();
    let html = GpuiComponentsAdapter::default()
        .import(&artifact, &mut import_context)
        .expect("GPUI components importer should recover HTML");
    let recovered = html.html();

    for expected in [
        r#"<div data-htmlswap-component="tabs" role="tablist" id="views" data-htmlswap-variant="underline" data-htmlswap-size="sm">"#,
        r#"<button data-htmlswap-component="tab" role="tab" aria-selected="true">"#,
        r#"<button data-htmlswap-component="tab" role="tab" disabled>"#,
        "Scene",
        "Game",
    ] {
        assert!(recovered.contains(expected), "{recovered}");
    }
}
