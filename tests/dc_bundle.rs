use htmlswap::{
    Adapter, AdapterContext, CompileAssets, Compiler, DcComponentFragment, Frontend, GpuiAdapter,
    GpuiAdapterOptions, RenderNode, RustFormatOptions, inline_dc_component_imports,
};

#[test]
fn dc_component_imports_inline_child_nodes_with_child_source_spans() {
    let compiler = Compiler::default().with_frontend(Frontend::dc());
    let assets = CompileAssets::new();
    let root = compiler.compile_fragment_named(
        Some("Root.dc.html".to_owned()),
        r#"<x-dc><section><dc-import name="Card"></dc-import></section></x-dc>"#,
        &assets,
    );
    let card = compiler.compile_fragment_named(
        Some("Card.dc.html".to_owned()),
        r#"<x-dc><div class="card"><span>Card body</span></div></x-dc>"#,
        &assets,
    );

    let expanded =
        inline_dc_component_imports(root.value, [DcComponentFragment::new("Card", card.value)]);

    let root = match &expanded.plan.nodes[0] {
        htmlswap::RenderNode::Element(element) => element,
        _ => panic!("expected root section element"),
    };
    assert_eq!(root.source_tag, "section");
    assert_eq!(root.children.len(), 1);
    let card = match &root.children[0] {
        htmlswap::RenderNode::Element(element) => element,
        _ => panic!("expected inlined card element"),
    };
    assert_eq!(card.source_tag, "div");
    let span = card.span.expect("inlined card should keep its source span");
    let source = expanded
        .sources
        .file(span.source)
        .expect("card source should be merged");
    assert_eq!(source.name(), Some("Card.dc.html"));
}

#[test]
fn gpui_debug_ids_use_child_component_source_after_dc_expansion() {
    let compiler = Compiler::default().with_frontend(Frontend::dc());
    let assets = CompileAssets::new();
    let root = compiler.compile_fragment_named(
        Some("Root.dc.html".to_owned()),
        r#"<x-dc><dc-import name="Card"></dc-import></x-dc>"#,
        &assets,
    );
    let card = compiler.compile_fragment_named(
        Some("Card.dc.html".to_owned()),
        r#"<x-dc><button onClick="{{ save }}">Save</button></x-dc>"#,
        &assets,
    );
    let expanded =
        inline_dc_component_imports(root.value, [DcComponentFragment::new("Card", card.value)]);

    let mut options = GpuiAdapterOptions::default();
    options.emit_debug_layout_ids = true;
    options.format = RustFormatOptions::disabled();
    let mut cx = AdapterContext::new();
    let output = GpuiAdapter::new(options)
        .adapt(&expanded, &mut cx)
        .expect("expanded DC fragment should emit GPUI");

    assert!(output.code().contains("htmlswap_debug_card_dc_html_"));
    assert!(!output.code().contains("htmlswap_debug_root_dc_html_"));
}

#[test]
fn dc_component_import_rewrites_literal_props_into_inlined_component_templates() {
    let compiler = Compiler::default().with_frontend(Frontend::dc());
    let assets = CompileAssets::new();
    let root = compiler.compile_fragment_named(
        Some("Root.dc.html".to_owned()),
        r#"<x-dc><dc-import name="Titlebar" title="Aether"></dc-import></x-dc>"#,
        &assets,
    );
    let titlebar = compiler.compile_fragment_named(
        Some("Titlebar.dc.html".to_owned()),
        r#"
            <x-dc><span>{{ title }}</span></x-dc>
            <script type="text/x-dc" data-dc-script>
                class Component extends DCLogic {
                    renderVals() {
                        const p = this.props;
                        return { title: p.title ?? 'Fallback' };
                    }
                }
            </script>
        "#,
        &assets,
    );

    let expanded = inline_dc_component_imports(
        root.value,
        [DcComponentFragment::new("Titlebar", titlebar.value)],
    );
    let RenderNode::Element(root) = &expanded.plan.nodes[0] else {
        panic!("expected root element");
    };
    let RenderNode::Element(span) = &root.children[0] else {
        panic!("expected inlined span");
    };
    let RenderNode::Text(text) = &span.children[0] else {
        panic!("expected span text");
    };
    assert_eq!(text.value, "Aether");
    assert!(text.template.is_none());
}
