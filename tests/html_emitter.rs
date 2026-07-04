use htmlswap::{CompileAssets, Compiler, EmitContext, Emitter, HtmlEmitter};

#[test]
fn html_emitter_serializes_recoverable_render_plan_core() {
    let compiled = Compiler::new().compile_fragment(
        r#"<section class="panel active" data-kind="project" style="display: flex; gap: 8px"><button id="save" onclick="{{ save }}">Save &amp; close</button><img src="logo.png" alt="Logo"></section>"#,
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let mut cx = EmitContext::new();
    let output = HtmlEmitter
        .emit(&compiled.value.plan, &mut cx)
        .expect("HTML emitter should emit recovered HTML");
    let html = output.html();

    assert!(html.contains(
        r#"<section class="panel active" data-kind="project" style="display: flex; gap: 8px">"#
    ));
    assert!(html.contains(r#"<button id="save" onclick="{{ save }}">"#));
    assert!(html.contains("Save &amp; close"));
    assert!(html.contains(r#"<img src="logo.png" alt="Logo">"#));
    assert!(cx.diagnostics().is_empty());
}
