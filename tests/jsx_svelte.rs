use htmlswap::{
    Adapter, AdapterContext, CompileAssets, Compiler, CompilerOptions, SourceFrontendKind,
    SvelteAdapter, SvelteAdapterOptions,
};

fn emit_svelte_jsx(source: &str) -> String {
    let compiler = Compiler::try_with_options(
        CompilerOptions::new().with_source_frontend(SourceFrontendKind::Jsx),
    )
    .expect("compiler options should be valid");
    let compiled =
        compiler.compile_fragment_named("fixture.jsx".to_owned(), source, &CompileAssets::new());
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );

    let mut cx = AdapterContext::new();
    let options = SvelteAdapterOptions {
        component_name: "Demo".into(),
        emit_source_comments: false,
        ..Default::default()
    };
    let output = SvelteAdapter::new(options)
        .adapt(&compiled.value, &mut cx)
        .expect("Svelte adapter should emit code");
    assert!(cx.diagnostics().is_empty(), "{:?}", cx.diagnostics());
    output.code().to_owned()
}

#[test]
fn emits_graph_banded_svelte_phase_zero_surface() {
    let code = emit_svelte_jsx(include_str!("fixtures/jsx/graph-banded.jsx"));

    assert!(code.contains("<script lang=\"ts\">"));
    assert!(code.contains("const SECTORS = ["));
    assert!(code.contains("const W = 420, H = 260;"));
    assert!(code.contains("<svg"));
    assert!(code.contains("{#each [60, 90] as r (r)}"));
    assert!(code.contains("{#each SECTORS as s, i (s.rel)}"));
    assert!(code.contains("{@const a0 = startAngle + i * (perSector + gap)}"));
    assert!(code.contains("{#each s.items as item, j (item.name)}"));
    assert!(code.contains("{@const t = colCount === 1 ? (tStart + tEnd) / 2 : tStart + (colIdx) * (tEnd - tStart) / (colCount - 1)}"));
    assert!(code.contains("text-anchor=\"middle\""));
    assert!(code.contains("stroke-width=\"1\""));
    assert!(code.contains("fill-opacity=\"0.06\""));
    assert!(code.contains("border-radius: 18px"));
    assert!(code.contains("width: ${W}px"));
    assert!(code.contains("var(--panel)"));
    assert!(!code.contains("className"));
    assert!(!code.contains("strokeWidth"));
    assert!(!code.contains("borderRadius"));
    assert!(!code.contains("textTransform"));
}

#[test]
fn emits_dynamic_class_attribute() {
    let code = emit_svelte_jsx(
        r#"
function Demo() {
  const active = true;
  return <div className={active ? 'on' : 'off'}>State</div>;
}
window.Demo = Demo;
"#,
    );

    assert!(code.contains("class={`"));
    assert!(code.contains("active ? \"on\" : \"off\""));
    assert!(!code.contains("className"));
}
