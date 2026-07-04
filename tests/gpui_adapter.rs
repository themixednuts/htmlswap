mod common;

use common::emitted_child_signatures_in_stack_order;
use htmlswap::{
    Adapter, AdapterContext, CompileAssets, Compiler, Frontend, GPUI_CRATE_VERSION, GpuiAdapter,
    GpuiAdapterOptions, Importer, SourceMappingKind, TargetArtifact,
};

#[test]
fn gpui_adapter_emits_gpui_code_and_dependency_metadata() {
    let assets = CompileAssets::new()
        .with_stylesheet(
            Some("app.css".to_owned()),
            "/* css note */\n.primary { display: flex; gap: 8px; color: red; }",
        )
        .with_script(Some("app.js".to_owned()), "// js note\nfunction save() {}");
    let compiled = Compiler::new().compile_fragment(
        r#"<!-- html note --><button id="save" class="primary" onclick="save()">Save</button>"#,
        &assets,
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();
    let dependencies = output.dependencies();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert_eq!(dependencies[0].package, "gpui");
    assert_eq!(dependencies[0].version_req, GPUI_CRATE_VERSION);
    assert_eq!(dependencies.len(), 1);

    assert!(code.contains("use gpui::{"));
    assert!(code.contains("AppContext as _"));
    assert!(code.contains("use gpui::prelude::FluentBuilder as _;"));
    assert!(!code.contains("gpui_component"));
    assert!(code.contains("pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self"));
    assert!(code.contains("\tfn render("));
    assert!(!code.contains("    fn render("));
    assert!(!code.contains("\r\n"));
    assert!(code.contains("// html-comment: html note"));
    assert!(code.contains("// css-comment: css note"));
    assert!(code.contains("// js-line-comment: js note"));
    assert!(code.contains("div()"));
    assert!(code.contains(".id(\"save\")"));
    assert!(code.contains("\"Save\""));
    assert!(code.contains(".flex()"));
    assert!(code.contains(".gap(gpui::px(8.0))"));
    assert!(code.contains(".text_color(gpui::rgb(0xFF0000))"));
    assert!(code.contains("save();"));
    assert!(code.contains("// html classes: primary"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_preserves_head_and_source_logic_metadata_comments() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"<x-dc>
                <helmet>
                    <link rel="stylesheet" href="/fonts.css">
                    <style>.ms { font-family: 'Material Symbols Outlined'; }</style>
                    <title>Aether</title>
                </helmet>
                <script data-dc-script type="module" data-props='{"editor":true}'>
                    class Component extends DCLogic {}
                </script>
                <div>Hi</div>
            </x-dc>"#,
            &CompileAssets::new(),
        );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(code.contains(r#"// source head: <link rel="stylesheet" href="/fonts.css">"#));
    assert!(code.contains(
        "// source head: <style>.ms { font-family: 'Material Symbols Outlined'; }</style>"
    ));
    assert!(code.contains("// source head: <title>Aether</title>"));
    assert!(code.contains("source logic: dialect=dc, type=module"));
    assert!(code.contains("data-props={\"editor\":true}"));
    assert!(code.contains("body omitted"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_can_debug_original_source_html_above_elements() {
    let compiled = Compiler::new().compile_fragment(
        r#"<button id="save" data-note="a > b"><span>Save</span></button>"#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut options = GpuiAdapterOptions::default();
    options.emit_debug_source_html_comments = true;

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::new(options)
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    assert!(
        code.contains(r#"// source html: <button id="save" data-note="a > b">"#),
        "{code}"
    );
    assert!(code.contains(r#"// source html: <span>"#), "{code}");
    assert!(code.contains("// layout source key:"), "{code}");
    assert!(code.contains("// layout source span:"), "{code}");
    assert!(
        !code.contains(r#"source html: <button id="save" data-note="a > b"><span>"#),
        "{code}"
    );
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_can_emit_source_stable_layout_debug_ids() {
    let compiled = Compiler::new().compile_fragment(
        r#"<div><span>Label</span><button id="save">Save</button></div>"#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut options = GpuiAdapterOptions::default();
    options.emit_debug_layout_ids = true;

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::new(options)
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(code.contains(".id(\"htmlswap_debug_source_s0_0_"));
    assert!(code.contains(".id(\"htmlswap_debug_source_s0_5_"));
    assert!(code.contains(".id(\"save\")"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_maps_material_symbol_spans_to_svg_icons() {
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
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit icon code");
    let code = output.code();
    let compact_code = code.split_whitespace().collect::<String>();

    syn::parse_file(code).expect("GPUI icon output should be valid Rust");

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
fn gpui_adapter_collapses_html_text_whitespace_in_generated_children() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"<x-dc>
                <div>
                    <span>{{ tab.label }}
                    </span>
                    <button>Asset
                        Browser</button>
                </div>
            </x-dc>"#,
            &CompileAssets::new(),
        );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");
    assert!(
        code.contains(r#"format!("{}", self.tab().label)"#),
        "{code}"
    );
    assert!(code.contains(r#""Asset Browser""#), "{code}");
    assert!(!code.contains(r#"\n                    ""#), "{code}");
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_maps_images_and_supported_style_properties() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        r#"
            img.asset {
                display: grid;
                margin-top: 1rem;
                margin-right: auto;
                margin-bottom: 0;
                margin-left: 5%;
                padding-top: 2px;
                padding-right: 3px;
                padding-bottom: 4px;
                padding-left: 5px;
                width: 50%;
                min-width: 10px;
                max-height: 2rem;
                border: 1px solid #336699cc;
                border-radius: 4px;
                background-color: rgba(255, 0, 0, 0.5);
                font-size: 14px;
                font-weight: 700;
                font-family: "Inter";
                line-height: 20px;
            }
        "#,
    );
    let compiled = Compiler::new()
        .compile_fragment(r#"<img id="logo" class="asset" src="logo.png">"#, &assets);
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(code.contains("gpui::img(\"logo.png\")"));
    assert!(code.contains(".id(\"logo\")"));
    assert!(!code.contains("unmapped attribute: src"));
    assert!(code.contains(".grid()"));
    assert!(code.contains(".mt(gpui::rems(1.0))"));
    assert!(code.contains(".mr(gpui::auto())"));
    assert!(code.contains(".mb(gpui::px(0.0))"));
    assert!(code.contains(".ml(gpui::relative(0.05))"));
    assert!(code.contains(".pt(gpui::px(2.0))"));
    assert!(code.contains(".pr(gpui::px(3.0))"));
    assert!(code.contains(".pb(gpui::px(4.0))"));
    assert!(code.contains(".pl(gpui::px(5.0))"));
    assert!(code.contains(".w(gpui::relative(0.5))"));
    assert!(code.contains(".min_w(gpui::px(10.0))"));
    assert!(code.contains(".max_h(gpui::rems(2.0))"));
    assert!(code.contains(".border(gpui::px(1.0))"));
    assert!(code.contains(".border_color(gpui::rgba(0x336699CC))"));
    assert!(code.contains(".rounded(gpui::px(4.0))"));
    assert!(code.contains(".bg(gpui::rgba(0xFF000080))"));
    assert!(code.contains(".text_size(gpui::px(14.0))"));
    assert!(code.contains(".font_weight(gpui::FontWeight::BOLD)"));
    assert!(code.contains(".font_family(\"Inter\")"));
    assert!(code.contains(".line_height(gpui::px(20.0))"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_maps_common_layout_css_properties() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        r#"
            .panel {
                display: inline-flex;
                flex: 0 0 32px;
                flex-direction: column;
                flex-wrap: wrap;
                align-items: center;
                align-self: stretch;
                align-content: center;
                justify-content: space-between;
                overflow: hidden;
                overflow-y: auto;
                grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
                position: absolute;
                inset: 0;
                top: 9px;
                left: 50%;
                width: 100vw;
                height: 100vh;
                background: #16181c;
                border-bottom: 1px solid #2c313a;
                padding: 0 12px;
                margin: 16px 0 6px;
                white-space: nowrap;
                text-overflow: ellipsis;
                text-align: right;
                font-style: normal;
                line-height: 1.4;
                opacity: .7;
                cursor: ew-resize;
                color: inherit;
                box-shadow: 0 12px 30px #00000080;
                box-sizing: border-box;
                user-select: none;
                outline: none;
            }
        "#,
    );
    let compiled =
        Compiler::new().compile_fragment(r#"<div id="panel" class="panel">Project</div>"#, &assets);
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();
    let compact_code = code.split_whitespace().collect::<String>();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(code.contains(".flex()"));
    assert!(code.contains("preserved CSS: display: inline-flex"));
    assert!(
        code.contains(
            "preserved CSS: grid-template-columns: repeat(auto-fill, minmax(180px, 1fr))"
        )
    );
    assert!(code.contains(".flex_none()"));
    assert!(code.contains(".flex_basis(gpui::px(32.0))"));
    assert!(code.contains(".flex_col()"));
    assert!(code.contains(".flex_wrap()"));
    assert!(code.contains(".items_center()"));
    assert!(code.contains(".content_center()"));
    assert!(code.contains(".justify_between()"));
    assert!(code.contains(".overflow_hidden()"));
    assert!(code.contains(".overflow_y_scroll()"));
    assert!(code.contains(".absolute()"));
    assert!(code.contains(".inset(gpui::px(0.0))"));
    assert!(code.contains(".top(gpui::px(9.0))"));
    assert!(code.contains(".left(gpui::relative(0.5))"));
    assert!(code.contains(".w_full()"));
    assert!(code.contains(".h_full()"));
    assert!(code.contains(".bg(gpui::rgb(0x16181C))"));
    assert!(code.contains(".border_b(gpui::px(1.0))"));
    assert!(code.contains(".border_color(gpui::rgb(0x2C313A))"));
    assert!(code.contains(".pt(gpui::px(0.0))"));
    assert!(code.contains(".pr(gpui::px(12.0))"));
    assert!(code.contains(".pb(gpui::px(0.0))"));
    assert!(code.contains(".pl(gpui::px(12.0))"));
    assert!(code.contains(".mt(gpui::px(16.0))"));
    assert!(code.contains(".mr(gpui::px(0.0))"));
    assert!(code.contains(".mb(gpui::px(6.0))"));
    assert!(code.contains(".ml(gpui::px(0.0))"));
    assert!(code.contains(".whitespace_nowrap()"));
    assert!(code.contains(".text_ellipsis()"));
    assert!(code.contains(".text_right()"));
    assert!(code.contains(".not_italic()"));
    assert!(code.contains(".line_height(gpui::relative(1.4))"));
    assert!(code.contains(".opacity(0.7)"));
    assert!(code.contains(".cursor_ew_resize()"));
    assert!(compact_code.contains(".shadow(std::vec![gpui::BoxShadow{"));
    assert!(!code.contains("unmapped CSS: box-sizing: border-box"));
    assert!(!code.contains("unmapped CSS: user-select: none"));
    assert!(!code.contains("unmapped CSS: outline: none"));
    assert!(!code.contains("unmapped CSS: color: inherit"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_makes_flex_direction_a_flex_container() {
    let compiled = Compiler::new().compile_fragment(
        r#"<div style="flex-direction:column"><span>Top</span><span>Bottom</span></div>"#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();
    let compact_code = code.split_whitespace().collect::<String>();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(compact_code.contains(".flex().flex_col()"), "{code}");
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_preserves_html_body_root_layout_styles() {
    let source = r#"<!doctype html>
<html>
<head>
    <style>
        html, body {
            width: 100vw;
            height: 100vh;
        }
        body {
            display: flex;
            align-items: center;
            justify-content: center;
            margin: 0;
            background: #1b1e23;
        }
        .app {
            width: 100%;
            height: 100%;
        }
    </style>
</head>
<body>
    <main class="app">Editor</main>
</body>
</html>"#;
    let compiled = Compiler::new().compile_fragment(source, &CompileAssets::new());
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();
    let compact_code = code.split_whitespace().collect::<String>();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(
        compact_code
            .contains("gpui::div().w_full().h_full().flex().items_center().justify_center()")
    );
    assert!(code.contains(".m(gpui::px(0.0))"));
    assert!(code.contains(".bg(gpui::rgb(0x1B1E23))"));
    assert!(compact_code.matches(".w_full()").count() >= 2);
    assert!(compact_code.matches(".h_full()").count() >= 2);
    assert!(compact_code.contains("\"Editor\""));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_does_not_size_plain_multi_node_fragments() {
    let compiled =
        Compiler::new().compile_fragment(r#"<p>One</p><p>Two</p>"#, &CompileAssets::new());
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(!code.contains(".w_full()"));
    assert!(!code.contains(".h_full()"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_sizes_retained_dc_root_wrappers() {
    let compiled = Compiler::new().compile_fragment(
        r#"<x-dc><main style="height:100%;width:100%;">Editor</main></x-dc>"#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(code.contains("// source tag: x-dc"));
    assert!(code.contains("gpui::div().size_full()"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_preserves_source_child_order_with_stack_scan() {
    let compiled = Compiler::new().compile_fragment(
        r#"<div><div id="first"></div><div id="second"></div><div id="third"></div></div>"#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert_eq!(
        emitted_child_signatures_in_stack_order(code),
        ["id:first", "id:second", "id:third"]
    );
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_preserves_mixed_text_and_element_child_order_with_stack_scan() {
    let compiled = Compiler::new().compile_fragment(
        r#"<div>Alpha<span id="middle"></span>Omega</div>"#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert_eq!(
        emitted_child_signatures_in_stack_order(code),
        ["text:Alpha", "id:middle", "text:Omega"]
    );
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_places_pseudo_children_with_stack_scan() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        r#"
            .panel::before { content: "Before"; color: green; }
            .panel::after { content: "After"; color: blue; }
        "#,
    );
    let compiled = Compiler::new().compile_fragment(
        r#"<div class="panel"><span id="main"></span></div>"#,
        &assets,
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert_eq!(
        emitted_child_signatures_in_stack_order(code),
        ["text:Before", "id:main", "text:After"]
    );
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_plans_numeric_z_index_with_debuggable_child_order() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        r#"
            .menu { position: absolute; z-index: 80; }
            .backdrop { position: fixed; z-index: 79; }
        "#,
    );
    let compiled = Compiler::new().compile_fragment(
        r#"<div><div id="menu" class="menu"></div><div id="backdrop" class="backdrop"></div><div id="base"></div></div>"#,
        &assets,
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert_eq!(
        emitted_child_signatures_in_stack_order(code),
        ["id:base", "id:backdrop", "id:menu"]
    );
    assert!(code.contains("htmlswap source-order: 0"));
    assert!(code.contains("htmlswap source-order: 1"));
    assert!(code.contains("htmlswap source-order: 2"));
    assert!(code.contains("htmlswap paint-order: 0 z-index: auto"));
    assert!(code.contains("htmlswap paint-order: 1 z-index: 79"));
    assert!(code.contains("htmlswap paint-order: 2 z-index: 80"));
    assert!(code.contains("planner CSS: z-index: 80"));
    assert!(code.contains("planner CSS: z-index: 79"));
    assert!(!code.contains("unmapped CSS: z-index"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_keeps_in_flow_z_index_children_in_source_order() {
    let compiled = Compiler::new().compile_fragment(
        r#"
            <div style="display:flex;flex-direction:column">
                <div id="menubar" style="position:relative;z-index:60"></div>
                <div id="toolbar" style="position:relative;z-index:40"></div>
                <div id="workspace"></div>
            </div>
        "#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert_eq!(
        emitted_child_signatures_in_stack_order(code),
        ["id:menubar", "id:toolbar", "id:workspace"]
    );
    assert!(code.contains("htmlswap source-order: 0"));
    assert!(code.contains("htmlswap source-order: 1"));
    assert!(code.contains("htmlswap source-order: 2"));
    assert!(code.contains("htmlswap paint-order: 0 z-index: 60"));
    assert!(code.contains("htmlswap paint-order: 1 z-index: 40"));
    assert!(code.contains("htmlswap paint-order: 2 z-index: auto"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_plans_control_flow_wrapper_z_index_with_debuggable_child_order() {
    let compiled = Compiler::new().with_frontend(Frontend::dc()).compile_fragment(
        r#"<div><div id="menu" style="position:absolute;z-index:90;"></div><sc-if value="{{ open }}"><div id="backdrop" style="position:fixed;z-index:85;"></div></sc-if></div>"#,
        &CompileAssets::new(),
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert_eq!(
        emitted_child_signatures_in_stack_order(code),
        ["id:backdrop", "id:menu"]
    );
    assert!(code.contains("htmlswap source-order: 0"));
    assert!(code.contains("htmlswap control-flow source-order: 1"));
    assert!(code.contains("htmlswap paint-order: 0 z-index: 85"));
    assert!(code.contains("htmlswap paint-order: 1 z-index: 90"));
    assert!(code.contains("planner CSS: z-index: 90"));
    assert!(code.contains("planner CSS: z-index: 85"));
    assert!(!code.contains("unmapped CSS: z-index"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_maps_fixed_grid_template_columns_to_stock_gpui_grid() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        ".grid { display: grid; grid-template-columns: repeat(3, 1fr); }",
    );
    let compiled = Compiler::new().compile_fragment(r#"<div class="grid"></div>"#, &assets);
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(code.contains(".grid()"));
    assert!(code.contains(".grid_cols(3)"));
    assert!(!code.contains("unmapped CSS: grid-template-columns"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_makes_scroll_overflow_elements_stateful_for_stock_gpui() {
    let assets = CompileAssets::new()
        .with_stylesheet(Some("app.css".to_owned()), ".scroll { overflow-y: auto; }");
    let compiled = Compiler::new().compile_fragment(r#"<div class="scroll"></div>"#, &assets);
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    let id_index = code
        .find(".id(\"htmlswap_interactive_0\")")
        .expect("scroll overflow element should receive a generated id");
    let overflow_index = code
        .find(".overflow_y_scroll()")
        .expect("scroll overflow should map to stock GPUI scroll overflow");
    assert!(id_index < overflow_index);
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_partially_maps_mixed_inset_box_shadows() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        ".preview { box-shadow: inset -10px -12px 26px #0009, 0 8px 20px #0006; }",
    );
    let compiled =
        Compiler::new().compile_fragment(r#"<div class="preview">Preview</div>"#, &assets);
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();
    let compact_code = code.split_whitespace().collect::<String>();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(compact_code.contains(".shadow(std::vec![gpui::BoxShadow{"));
    assert!(compact_code.contains("offset:gpui::point(gpui::px(0.0),gpui::px(8.0))"));
    assert!(compact_code.contains("blur_radius:gpui::px(20.0)"));
    assert!(
        code.contains("preserved CSS: box-shadow: inset -10px -12px 26px #0009, 0 8px 20px #0006"),
        "{code}"
    );
    assert!(!compact_code.contains("gpui::px(-10.0)"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_preserves_unsupported_background_approximations() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        ".preview { background: radial-gradient(circle at 34% 28%, #c9ccd2 0%, #24272c 100%); }",
    );
    let compiled =
        Compiler::new().compile_fragment(r#"<div class="preview">Preview</div>"#, &assets);
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(code.contains(".bg(gpui::rgb("), "{code}");
    assert!(
        code.contains(
            "preserved CSS: background: radial-gradient(circle at 34% 28%, #c9ccd2 0%, #24272c 100%)"
        ),
        "{code}"
    );
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_maps_additional_editor_css_properties() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        r#"
            .editor-tile {
                aspect-ratio: 1;
                background: linear-gradient(135deg, #3160a8, #5ac0c0);
                background-image: none;
                background-size: auto;
                border-radius: 50%;
                flex: 1 1 110px;
                width: fit-content;
                min-width: max-content;
                letter-spacing: normal;
                pointer-events: auto;
                position: absolute;
                left: calc(50% - 22px);
                top: calc(50% + 6px);
                right: calc(124px + (100% - 124px) * 0);
                text-transform: uppercase;
                transform-origin: top;
                white-space: pre;
                word-break: normal;
            }
            .centered-dot {
                position: absolute;
                left: 50%;
                top: 50%;
                width: 80px;
                height: 40px;
                transform: translate(-50%, -25%);
            }
            .pixel-shift {
                position: absolute;
                top: 20px;
                transform: translateY(-22px);
            }
            .inset-line {
                box-shadow: inset 0 -2px 0 #4188e0;
            }
        "#,
    );
    let compiled = Compiler::new().compile_fragment(
        r#"<div class="editor-tile">Tile</div><div class="centered-dot"></div><div class="pixel-shift"></div><div class="inset-line"></div>"#,
        &assets,
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();
    let compact_code = code.split_whitespace().collect::<String>();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(code.contains("aspect_ratio = Some(1.0)"));
    assert!(
        compact_code
            .contains(".bg(gpui::linear_gradient(135.0,gpui::linear_color_stop(gpui::rgb(0x3160A8),0.0),gpui::linear_color_stop(gpui::rgb(0x5AC0C0),1.0),))")
    );
    assert!(code.contains(".rounded_full()"));
    assert!(code.contains(".flex_grow_1()"));
    assert!(code.contains(".flex_basis(gpui::px(110.0))"));
    assert!(code.contains(".w(gpui::auto())"));
    assert!(code.contains(".min_w(gpui::auto())"));
    assert!(code.contains(".absolute()"));
    assert!(code.contains(".left(gpui::relative(0.5))"));
    assert!(code.contains(".ml(gpui::px(-22.0))"));
    assert!(code.contains(".top(gpui::relative(0.5))"));
    assert!(code.contains(".mt(gpui::px(6.0))"));
    assert!(code.contains(".right(gpui::px(124.0))"));
    assert!(code.contains(".ml(gpui::px(-40.0))"));
    assert!(code.contains(".mt(gpui::px(-10.0))"));
    assert!(code.contains(".mt(gpui::px(-22.0))"));
    assert!(code.contains(".border_b(gpui::px(2.0))"));
    assert!(code.contains(".border_color(gpui::rgb(0x4188E0))"));
    assert!(code.contains("preserved CSS: width: fit-content"));
    assert!(code.contains("preserved CSS: min-width: max-content"));
    assert!(code.contains("preserved CSS: left: calc(50% - 22px)"));
    assert!(code.contains("preserved CSS: top: calc(50% + 6px)"));
    assert!(code.contains("preserved CSS: transform: translate(-50%, -25%)"));
    assert!(code.contains("preserved CSS: transform: translateY(-22px)"));
    assert!(code.contains("preserved CSS: box-shadow: inset 0 -2px #4188e0"));
    assert!(code.contains(".whitespace_nowrap()"));
    assert!(code.contains("\"TILE\""));
    assert!(!code.contains("unmapped CSS: aspect-ratio: 1"));
    assert!(!code.contains("unmapped CSS: background: linear-gradient"));
    assert!(!code.contains("unmapped CSS: background-image: none"));
    assert!(!code.contains("unmapped CSS: background-size: auto"));
    assert!(!code.contains("unmapped CSS: border-radius: 50%"));
    assert!(!code.contains("unmapped CSS: flex: 1 1 110px"));
    assert!(!code.contains("unmapped CSS: width: fit-content"));
    assert!(!code.contains("unmapped CSS: min-width: max-content"));
    assert!(!code.contains("unmapped CSS: letter-spacing: normal"));
    assert!(!code.contains("unmapped CSS: pointer-events: auto"));
    assert!(!code.contains("unmapped CSS: position: absolute"));
    assert!(!code.contains("unmapped CSS: left: calc(50% - 22px)"));
    assert!(!code.contains("unmapped CSS: top: calc(50% + 6px)"));
    assert!(!code.contains("unmapped CSS: right: calc(124px + (100% - 124px) * 0)"));
    assert!(!code.contains("unmapped CSS: text-transform: uppercase"));
    assert!(!code.contains("unmapped CSS: transform: translate(-50%, -25%)"));
    assert!(!code.contains("unmapped CSS: transform: translateY(-22px)"));
    assert!(!code.contains("unmapped CSS: box-shadow: inset 0 -2px #4188e0"));
    assert!(!code.contains("unmapped CSS: transform-origin: top"));
    assert!(!code.contains("unmapped CSS: white-space: pre"));
    assert!(!code.contains("unmapped CSS: word-break: normal"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_emits_marker_free_source_map_after_formatting() {
    let assets = CompileAssets::new()
        .with_stylesheet(
            Some("app.css".to_owned()),
            "/* css note */\n.primary { color: red; }",
        )
        .with_script(Some("app.js".to_owned()), "// js note\nfunction save() {}");
    let compiled = Compiler::new().compile_fragment(
        r#"<!-- html note --><button id="save" class="primary" onclick="save()">Save</button>"#,
        &assets,
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let file = output
        .artifact
        .files
        .first()
        .expect("GPUI output should contain a generated file");
    let source_map = file
        .source_map
        .as_ref()
        .expect("GPUI output should contain a generated source map");

    assert!(!file.contents.contains("htmlswap-source-map"));
    for mapping in source_map.mappings() {
        assert!(mapping.generated.start <= mapping.generated.end);
        assert!(mapping.generated.end <= file.contents.len());
        assert!(
            compiled
                .value
                .sources
                .source_text(mapping.original)
                .is_some()
        );
    }

    let kinds = source_map
        .mappings()
        .iter()
        .map(|mapping| mapping.kind)
        .collect::<Vec<_>>();
    assert!(kinds.contains(&SourceMappingKind::Element));
    assert!(kinds.contains(&SourceMappingKind::Attribute));
    assert!(kinds.contains(&SourceMappingKind::Text));
    assert!(kinds.contains(&SourceMappingKind::Style));
    assert!(kinds.contains(&SourceMappingKind::Action));
    assert!(kinds.contains(&SourceMappingKind::Comment));

    let id_mapping = source_map
        .mappings()
        .iter()
        .find(|mapping| {
            mapping.kind == SourceMappingKind::Attribute
                && compiled
                    .value
                    .sources
                    .source_text(mapping.original)
                    .is_some_and(|source| source == r#"id="save""#)
        })
        .expect("id attribute should map to generated id method");
    assert_eq!(
        file.contents[id_mapping.generated.start..id_mapping.generated.end].trim(),
        ".id(\"save\")"
    );
}

#[test]
fn gpui_adapter_emits_supported_dynamic_style_variants() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        ".primary:hover { color: red; background-color: blue; }\n.primary:focus { border: 1px solid red; }\n.primary::before { content: \"New\"; color: green; }",
    );
    let compiled =
        Compiler::new().compile_fragment(r#"<button class="primary">Save</button>"#, &assets);
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(
        code.contains(
            ".hover(|this| this.text_color(gpui::rgb(0xFF0000)).bg(gpui::rgb(0x0000FF)))"
        )
    );
    assert!(code.contains(".id(\"htmlswap_interactive_"));
    assert!(
        code.contains(
            ".focus(|this| this.border(gpui::px(1.0)).border_color(gpui::rgb(0xFF0000)))"
        )
    );
    assert!(code.contains("\"New\""));
    assert!(code.contains(".text_color(gpui::rgb(0x008000))"));
    assert!(adapter_context.diagnostics().iter().all(|diagnostic| {
        !diagnostic.message.contains("`.primary:hover`")
            && !diagnostic.message.contains("`.primary:before`")
    }));
}

#[test]
fn gpui_adapter_emits_dc_dynamic_dock_lengths_and_flex_tracks() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"<x-dc>
            <script data-dc-script>
                class Component extends DCLogic {
                    state = { leftW: 264, bottomH: 240 };
                    renderVals() {
                        const st = this.state;
                        return {
                            leftDockStyle: {
                                width: st.leftW + 'px',
                                flex: '0 0 ' + st.leftW + 'px',
                                display: 'flex',
                                flexDirection: 'column'
                            },
                            bottomDockStyle: {
                                height: st.bottomH + 'px',
                                flex: '0 0 ' + st.bottomH + 'px',
                                display: 'flex',
                                flexDirection: 'column'
                            }
                        };
                    }
                }
            </script>
            <div style="{{ leftDockStyle }}">
                <div style="{{ bottomDockStyle }}">Assets</div>
            </div>
        </x-dc>"#,
            &CompileAssets::new(),
        );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");
    let compact_code = code.split_whitespace().collect::<String>();

    assert!(
        code.contains("fn htmlswap_dynamic_definite_length"),
        "{code}"
    );
    assert!(
        compact_code
            .contains(".w(htmlswap_dynamic_definite_length(format!(\"{}px\",self.left_w())))")
    );
    assert!(
        compact_code
            .contains(".h(htmlswap_dynamic_definite_length(format!(\"{}px\",self.bottom_h())))")
    );
    assert!(code.contains(".flex_shrink_0()"));
    assert!(code.contains(".flex_none()"));
    assert!(
        compact_code
            .contains(".flex_basis(htmlswap_dynamic_length(format!(\"{}px\",self.left_w())))")
    );
    assert!(
        compact_code
            .contains(".flex_basis(htmlswap_dynamic_length(format!(\"{}px\",self.bottom_h())))")
    );
    assert!(code.contains(".flex()"));
    assert!(code.contains(".flex_col()"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_emits_source_dynamic_loops_bindings_and_handlers() {
    let compiled = Compiler::new().with_frontend(Frontend::dc()).compile_fragment(
            r#"<sc-for list="{{ items }}" as="item"><button style="{{ item.style }}" style-hover="{{ item.hoverStyle }}" onclick="{{ item.onClick }}">{{ item.label }}</button></sc-for>"#,
            &CompileAssets::new(),
        );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(code.contains("self.items()"));
    assert!(code.contains(".enumerate()"));
    assert!(code.contains(".map(|(htmlswap_index_"));
    assert!(code.contains("let htmlswap_capture_item = item.clone();"));
    assert!(code.contains(".on_click({"));
    assert!(code.contains(".id(format!(\"htmlswap_interactive_"));
    assert!(code.contains(":{}\", htmlswap_index_"));
    assert!(!code.contains(".id(\"htmlswap_interactive_"));
    assert!(code.contains("htmlswap_capture_item.on_click(_event, _window, _cx);"));
    assert!(code.contains("format!(\"{}\", item.label)"));
    assert!(code.contains("dynamic style: \"{{ item.style }}\" [item.style]"));
    assert!(code.contains("dynamic hover style: \"{{ item.hoverStyle }}\" [item.hoverStyle]"));
    assert!(code.contains("source control-flow: for, expr=items, item=item"));
    assert!(!code.contains("template action:"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_emits_vue_loops_bindings_and_handlers() {
    let compiled = Compiler::new().with_frontend(Frontend::vue()).compile_fragment(
        r#"<div><button v-for="item in items" :title="item.title" :style="item.style" @click="item.onOpen">{{ item.label }}</button></div>"#,
        &CompileAssets::new(),
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(code.contains("self.items()"));
    assert!(code.contains(".enumerate()"));
    assert!(code.contains(".map(|(htmlswap_index_"));
    assert!(code.contains(".on_click({"));
    assert!(code.contains(".id(format!(\"htmlswap_interactive_"));
    assert!(code.contains(":{}\", htmlswap_index_"));
    assert!(code.contains("let htmlswap_capture_item = item.clone();"));
    assert!(code.contains("htmlswap_capture_item.on_open(_event, _window, _cx);"));
    assert!(code.contains("format!(\"{}\", item.label)"));
    assert!(code.contains("let htmlswap_tooltip"));
    assert!(code.contains("item.title"));
    assert!(code.contains(".to_string();"));
    assert!(code.contains("dynamic style: \"{{ item.style }}\" [item.style]"));
    assert!(!code.contains("template action:"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_emits_conditionals_and_non_click_template_handlers() {
    let compiled = Compiler::new().with_frontend(Frontend::dc()).compile_fragment(
            r#"<sc-if value="{{ modeScene }}"><button onMouseEnter="{{ openMenu }}">{{ title }} {{ count }}</button></sc-if>"#,
            &CompileAssets::new(),
        );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(code.contains(".when(self.mode_scene(), |this|"));
    assert!(code.contains(".on_hover(_cx.listener(move |this, is_hovered, _window, _cx|"));
    assert!(code.contains("if *is_hovered"));
    assert!(code.contains("this.open_menu(is_hovered, _window, _cx);"));
    assert!(code.contains("format!(\"{} {}\", self.title(), self.count())"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_emits_double_click_template_handlers() {
    let compiled = Compiler::new().with_frontend(Frontend::dc()).compile_fragment(
        r#"<sc-for list="{{ nodes }}" as="n"><div onDoubleClick="{{ n.onOpen }}">{{ n.name }}</div></sc-for>"#,
        &CompileAssets::new(),
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();
    let compact_code = code.split_whitespace().collect::<String>();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(code.contains(".on_click({"));
    assert!(code.contains("let htmlswap_capture_n = n.clone();"));
    assert!(compact_code.contains("_cx.listener(move|this,_event:&gpui::ClickEvent,_window,_cx|"));
    assert!(code.contains("if _event.click_count() == 2"));
    assert!(code.contains("htmlswap_capture_n.on_open(_event, _window, _cx);"));
    assert!(!code.contains("template action:"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_emits_dynamic_title_tooltips_for_base_gpui() {
    let compiled = Compiler::new().with_frontend(Frontend::dc()).compile_fragment(
        r#"<sc-for list="{{ tools }}" as="tool"><div title="{{ tool.title }}">{{ tool.label }}</div></sc-for>"#,
        &CompileAssets::new(),
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(code.contains("struct HtmlswapTooltipView"));
    assert!(code.contains(".tooltip({"));
    assert!(code.contains("let htmlswap_tooltip = (format!(\"{}\", tool.title)).to_string();"));
    assert!(code.contains("_cx.new(|_| HtmlswapTooltipView {"));
    assert!(code.contains("text: htmlswap_tooltip"));
    assert!(!code.contains("unmapped tooltip"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_bridges_dc_source_owned_text_inputs_without_gpui_components() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"<x-dc>
                <input id="project-name" data-htmlswap-state-owner="source" value="{{ name }}" placeholder="{{ prompt }}" title="Rename project" onInput="{{ onName }}">
            </x-dc>"#,
            &CompileAssets::new(),
        );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(code.contains("struct HtmlswapGpuiTextInput"));
    assert!(code.contains("impl gpui::EntityInputHandler for HtmlswapGpuiTextInput"));
    assert!(code.contains("gpui::Entity<HtmlswapGpuiTextInput>"));
    assert!(code.contains("_htmlswap_subscriptions: Vec<gpui::Subscription>"));
    assert!(code.contains("cx.subscribe_in("));
    assert!(code.contains("&this.project_name"));
    assert!(code.contains("this.on_name(&_htmlswap_value, window, cx);"));
    assert!(code.contains("format!(\"{}\", this.name())"));
    assert!(
        code.contains("let htmlswap_placeholder_project_name = format!(\"{}\", this.prompt())")
    );
    assert!(code.contains("this.project_name.update(cx, |state, cx| {"));
    assert!(code.contains("state.set_placeholder"));
    assert!(code.contains("let htmlswap_tooltip = (\"Rename project\").to_string();"));
    assert!(!code.contains("gpui_component::input"));
    assert!(!code.contains("{{ prompt }}"));
    assert!(!code.contains("source-owned and cannot be mutated"));
    assert!(!code.contains("unmapped action: input={{ onName }}"));
    assert!(!code.contains("unmapped tooltip"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_bridges_loop_scoped_source_owned_text_inputs() {
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

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(code.contains("_htmlswap_gpui_text_inputs"));
    let compact_code = code.split_whitespace().collect::<String>();
    assert!(code.contains(".enumerate()"));
    assert!(code.contains("htmlswap_index_"));
    assert!(code.contains("format!(\"input_0:{}\""));
    assert!(code.contains("HtmlswapGpuiTextInput::new"));
    assert!(code.contains("htmlswap_input_value.clone()"));
    assert!(code.contains("let htmlswap_input_placeholder"));
    assert!(code.contains("format!(\"{}\", p.placeholder"));
    assert!(code.contains("state.set_placeholder("));
    assert!(code.contains("htmlswap_input_placeholder.clone()"));
    assert!(code.contains("let htmlswap_capture_p = p.clone();"));
    assert!(compact_code.contains("htmlswap_capture_p.on_x(&_htmlswap_value,window,cx,);"));
    assert!(!code.contains("this.p().x"));
    assert!(!code.contains("{{ p.placeholder }}"));
    assert!(!code.contains("source-owned and cannot be mutated"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_preserves_href_as_source_route_metadata() {
    let compiled = Compiler::new().compile_fragment(
        r#"<a href="Aether Editor.dc.html">Open</a>"#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(code.contains(r#"// source prop href="Aether Editor.dc.html""#));
    assert!(!code.contains("unmapped attribute: href"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_lowers_switch_to_conditional_branches() {
    let compiled = Compiler::new().with_frontend(Frontend::dc()).compile_fragment(
        r#"<sc-switch value="{{ activeView }}"><sc-case value="{{ sceneView }}"><button>Scene</button></sc-case><sc-default><button>Other</button></sc-default></sc-switch>"#,
        &CompileAssets::new(),
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(code.contains(".when(self.active_view() == self.scene_view(), |this|"));
    assert!(code.contains(".when(!(self.active_view() == self.scene_view()), |this|"));
    assert!(code.contains("\"Scene\""));
    assert!(code.contains("\"Other\""));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_preserves_conditional_css_with_target_strategy_comments() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        r#"
            .primary:focus-visible { color: green; }
            @media (min-width: 600px) { .primary { color: blue; } }
            @media (min-width: 600px) { .primary::before { content: "Wide"; color: blue; } }
            @supports (display: grid) { .primary { color: yellow; } }
            @container card (min-width: 300px) { .primary { color: black; } }
        "#,
    );
    let compiled =
        Compiler::new().compile_fragment(r#"<button class="primary">Save</button>"#, &assets);
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");

    assert!(code.contains(":focus-visible"));
    assert!(code.contains("@media"));
    assert!(code.contains("@supports"));
    assert!(code.contains("@container"));
    assert!(code.contains("pseudo-element CSS selector="));
    assert!(code.contains("conditions=\"media="));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_does_not_attach_unanchored_global_pseudo_css_to_every_element() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        r#"
            ::selection { background: rgba(65, 136, 224, .35); }
            *:hover::-webkit-scrollbar-thumb { background: #39404b; }
            .primary:focus-visible { color: green; }
        "#,
    );
    let compiled =
        Compiler::new().compile_fragment(r#"<button class="primary">Save</button>"#, &assets);
    assert!(compiled.diagnostics.is_empty());

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    let code = output.code();

    syn::parse_file(code).expect("GPUI adapter should emit syntactically valid Rust");
    assert!(!code.contains(r#"pseudo-element CSS selector="::selection""#));
    assert!(!code.contains(r#"pseudo-element CSS selector="*:hover::-webkit-scrollbar-thumb""#));
    assert!(code.contains("conditional CSS :focus-visible .primary:focus-visible"));
    assert!(adapter_context.diagnostics().is_empty());
}

#[test]
fn gpui_adapter_rejects_invalid_component_names() {
    let compiled = Compiler::new().compile_fragment("", &CompileAssets::new());
    let mut adapter_context = AdapterContext::new();
    let adapter = GpuiAdapter::new(GpuiAdapterOptions {
        component_name: "not-valid".into(),
        ..GpuiAdapterOptions::default()
    });

    assert!(
        adapter
            .adapt(&compiled.value, &mut adapter_context)
            .is_err()
    );
}

#[test]
fn gpui_adapter_exposes_html_import_api() {
    let compiled = Compiler::new().compile_fragment("<div>Imported</div>", &CompileAssets::new());
    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::default()
        .adapt(&compiled.value, &mut adapter_context)
        .expect("GPUI adapter should emit code");
    assert!(adapter_context.diagnostics().is_empty());

    let input = TargetArtifact::from(output.artifact);
    let mut adapter_context = AdapterContext::new();
    let result = GpuiAdapter::default().import(&input, &mut adapter_context);

    assert!(
        result
            .expect("GPUI importer should recover HTML")
            .html()
            .contains("Imported")
    );
}
