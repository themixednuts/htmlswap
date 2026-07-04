use htmlswap::{
    CompileAssets, Compiler, LayoutDebugOptions, instrument_layout_snapshot_html,
    layout_debug_id_for_span,
};

#[test]
fn layout_debug_instruments_native_html_with_stable_source_ids() {
    let source = r#"<div id="root"><span class="label">Hi</span><input value="x" /></div>"#;
    let compiled = Compiler::new().compile_fragment(source, &CompileAssets::new());
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let output = instrument_layout_snapshot_html(
        source,
        &compiled.value.plan,
        &LayoutDebugOptions::default(),
    );

    assert!(output.contains(r#"data-htmlswap-debug-id="root""#));
    assert!(output.contains(r#"data-htmlswap-debug-id="htmlswap_debug_source_s0_"#));
    assert!(output.contains(r#"data-htmlswap-source-key="source_s0_"#));
    assert!(output.contains(r#"data-htmlswap-source-span=""#));
    assert!(output.contains("window.__HTMLSWAP_LAYOUT_SNAPSHOT__"));
    assert!(output.contains("relative_x"));
    assert_eq!(output.matches(r#"data-htmlswap-debug-id="#).count(), 3);
}

#[test]
fn layout_debug_id_for_span_is_source_stable() {
    assert_eq!(
        layout_debug_id_for_span(htmlswap::Span::primary(12, 34)),
        "htmlswap_debug_s0_12_34"
    );
}
