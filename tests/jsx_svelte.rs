use htmlswap::{
    Adapter, AdapterArtifact, AdapterContext, CompileAssets, Compiler, CompilerOptions,
    SourceFrontendKind, SvelteAdapter, SvelteAdapterOptions,
};
use std::path::{Path, PathBuf};

fn emit_svelte_jsx(source: &str) -> String {
    let (code, diagnostics) = emit_svelte_jsx_with_diagnostics(source);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    code
}

fn emit_svelte_jsx_with_diagnostics(source: &str) -> (String, Vec<String>) {
    let compiler = Compiler::try_with_options(
        CompilerOptions::new().with_source_frontend(SourceFrontendKind::Jsx),
    )
    .expect("compiler options should be valid");
    let compiled =
        compiler.compile_fragment_named("fixture.jsx".to_owned(), source, &CompileAssets::new());
    let mut diagnostics = compiled
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.clone())
        .collect::<Vec<_>>();

    let mut cx = AdapterContext::new();
    let options = SvelteAdapterOptions {
        component_name: "Demo".into(),
        emit_source_comments: false,
        ..Default::default()
    };
    let output = SvelteAdapter::new(options)
        .adapt(&compiled.value, &mut cx)
        .expect("Svelte adapter should emit code");
    diagnostics.extend(
        cx.diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.message.clone()),
    );
    (output.code().to_owned(), diagnostics)
}

fn emit_svelte_jsx_project_from_dir(root: &Path) -> (AdapterArtifact, Vec<String>) {
    let compiler = Compiler::try_with_options(
        CompilerOptions::new().with_source_frontend(SourceFrontendKind::Jsx),
    )
    .expect("compiler options should be valid");
    let mut sources = jsx_project_sources(root);
    sources.sort_by(|left, right| left.0.cmp(&right.0));
    let compiled = compiler.compile_jsx_svelte_project(
        sources,
        SvelteAdapterOptions {
            component_name: "View".into(),
            emit_source_comments: false,
            ..Default::default()
        },
    );
    let diagnostics = compiled
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.clone())
        .collect::<Vec<_>>();
    (compiled.value, diagnostics)
}

fn jsx_project_sources(root: &Path) -> Vec<(String, String)> {
    std::fs::read_dir(root)
        .expect("fixture directory should be readable")
        .map(|entry| entry.expect("fixture entry should be readable").path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("jsx"))
        .filter(|path| {
            !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    matches!(
                        name,
                        "design-canvas.jsx" | "tweaks-panel.jsx" | "theme-tweaks.jsx"
                    )
                })
        })
        .map(|path| {
            let name = path.to_string_lossy().replace('\\', "/");
            let source = std::fs::read_to_string(&path).expect("fixture should be readable");
            (name, source)
        })
        .collect()
}

fn generated_file<'a>(artifact: &'a AdapterArtifact, path: &str) -> &'a str {
    artifact
        .files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("expected generated file `{path}`"))
        .contents
        .as_str()
}

fn assert_project_has_no_unresolved_internal_refs(artifact: &AdapterArtifact) {
    for file in &artifact.files {
        assert!(
            !file.contents.contains("htmlswap-jsx-component-placeholder"),
            "{} still contains JSX component placeholder:\n{}",
            file.path,
            file.contents
        );
        assert!(
            !file.contents.contains("unsupported JSX component"),
            "{} still contains unsupported component text",
            file.path
        );
        assert!(
            !file.contents.contains("window."),
            "{} still contains a corpus-internal window reference:\n{}",
            file.path,
            file.contents
        );
    }
}

#[test]
fn project_mode_resolves_component_data_member_and_alias_imports() {
    let (artifact, diagnostics) =
        emit_svelte_jsx_project_from_dir(Path::new("tests/fixtures/jsx/project"));
    assert!(diagnostics.is_empty(), "{diagnostics:?}");

    let view = generated_file(&artifact, "View.svelte");
    assert!(
        view.contains("import Badge from \"./Badge.svelte\";"),
        "{view}"
    );
    assert!(
        view.contains("import IconSearch from \"./IconSearch.svelte\";"),
        "{view}"
    );
    assert!(
        view.contains("import KindBadge from \"./KindBadge.svelte\";"),
        "{view}"
    );
    assert!(
        view.contains("import { nodeOf } from './graph-data.ts';"),
        "{view}"
    );
    assert!(view.contains("<Badge label={node.path}>"), "{view}");
    assert!(view.contains("<IconSearch size={16}>"), "{view}");
    assert!(view.contains("<KindBadge kind={node.kind}>"), "{view}");

    let graph_data = generated_file(&artifact, "graph-data.ts");
    assert!(graph_data.contains("export const NODES"), "{graph_data}");
    assert!(
        graph_data.contains("export function nodeOf"),
        "{graph_data}"
    );
    assert_project_has_no_unresolved_internal_refs(&artifact);
}

#[test]
fn corpus_project_mode_emits_no_unresolved_internal_refs() {
    let Some(corpus) = std::env::var_os("HTMLSWAP_JSX_CORPUS") else {
        eprintln!("skipping optional corpus smoke test; HTMLSWAP_JSX_CORPUS is not set");
        return;
    };
    let corpus = PathBuf::from(corpus);
    if !corpus.exists() {
        eprintln!("skipping corpus smoke test; {} is absent", corpus.display());
        return;
    }

    let (artifact, diagnostics) = emit_svelte_jsx_project_from_dir(&corpus);
    let serious = diagnostics
        .iter()
        .filter(|message| {
            message.contains("parser stopped")
                || message.contains("outside Phase")
                || message.contains("unsupported JSX component")
                || message.contains("Svelte adapter failed")
        })
        .collect::<Vec<_>>();
    assert!(serious.is_empty(), "{diagnostics:?}");
    assert!(
        artifact
            .files
            .iter()
            .any(|file| file.path == "graph-data.ts")
    );
    assert!(
        artifact
            .files
            .iter()
            .any(|file| file.path == "Explorer.svelte")
    );
    assert!(
        artifact
            .files
            .iter()
            .any(|file| file.path == "IconSearch.svelte")
    );
    assert_project_has_no_unresolved_internal_refs(&artifact);
}

#[test]
fn emits_graph_banded_svelte_phase_zero_surface() {
    let code = emit_svelte_jsx(include_str!("fixtures/jsx/graph-banded.jsx"));

    assert!(code.contains("<script lang=\"ts\">"));
    assert!(code.contains("const SECTORS = ["));
    assert!(code.contains("const W = 420;"));
    assert!(code.contains("const H = 260;"));
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

    assert!(code.contains("class={active ? \"on\" : \"off\"}"));
    assert!(code.contains("active ? \"on\" : \"off\""));
    assert!(!code.contains("className"));
}

#[test]
fn emits_react_style_numeric_units_and_unitless_values() {
    let code = emit_svelte_jsx(
        r#"
function Demo() {
  return <div style={{ width: 1280, lineHeight: 1.05, opacity: 0.72, fontWeight: 600, aspectRatio: 1.5, borderRadius: 8 }} />;
}
window.Demo = Demo;
"#,
    );

    assert!(code.contains("width: 1280px"), "{code}");
    assert!(code.contains("line-height: 1.05"), "{code}");
    assert!(code.contains("opacity: 0.72"), "{code}");
    assert!(code.contains("font-weight: 600"), "{code}");
    assert!(code.contains("aspect-ratio: 1.5"), "{code}");
    assert!(code.contains("border-radius: 8px"), "{code}");
    assert!(!code.contains("line-height: 1.05px"), "{code}");
}

#[test]
fn emits_svg_attribute_spellings() {
    let code = emit_svelte_jsx(
        r#"
function Demo() {
  return <svg viewBox="0 0 10 10"><path strokeWidth={2} strokeLinecap="round" strokeLinejoin="bevel" fillOpacity={0.5} strokeOpacity={0.25} /><text textAnchor="middle" fontFamily="monospace" fontSize={12}>Hi</text></svg>;
}
window.Demo = Demo;
"#,
    );

    assert!(code.contains(r#"viewBox="0 0 10 10""#), "{code}");
    assert!(code.contains("stroke-width={2}"), "{code}");
    assert!(code.contains(r#"stroke-linecap="round""#), "{code}");
    assert!(code.contains(r#"stroke-linejoin="bevel""#), "{code}");
    assert!(code.contains("fill-opacity={0.5}"), "{code}");
    assert!(code.contains("stroke-opacity={0.25}"), "{code}");
    assert!(code.contains(r#"text-anchor="middle""#), "{code}");
    assert!(code.contains(r#"font-family="monospace""#), "{code}");
    assert!(code.contains("font-size={12}"), "{code}");
    assert!(!code.contains("strokeWidth"), "{code}");
    assert!(!code.contains("textAnchor"), "{code}");
}

#[test]
fn emits_dynamic_class_concatenation_and_preserves_static_tailwind() {
    let code = emit_svelte_jsx(
        r#"
function Demo() {
  const tone = 'accent';
  return <div className="grid grid-cols-[1fr_minmax(480px,_44%)] hover:bg-white/[0.03] text-[10.5px]"><span className={'btn ' + tone}>Go</span></div>;
}
window.Demo = Demo;
"#,
    );

    assert!(
        code.contains(
            r#"class="grid grid-cols-[1fr_minmax(480px,_44%)] hover:bg-white/[0.03] text-[10.5px]""#
        ),
        "{code}"
    );
    assert!(code.contains(r#"class={"btn " + tone}"#), "{code}");
    assert!(!code.contains("className"), "{code}");
}

#[test]
fn emits_inline_style_block_and_component_references() {
    let (code, diagnostics) = emit_svelte_jsx_with_diagnostics(
        r#"
function Demo() {
  return <div><TopBar /><style>{`@keyframes blink { 0% { opacity: 1 } 100% { opacity: 0 } }`}</style></div>;
}
window.Demo = Demo;
"#,
    );

    assert_eq!(
        diagnostics,
        ["JSX component tag `TopBar` is outside Phase 1 scope; emitted placeholder"]
    );
    assert!(code.contains("<style>"), "{code}");
    assert!(code.contains("@keyframes blink"), "{code}");
    assert!(!code.contains("<style>{"), "{code}");
    assert!(
        code.contains("htmlswap-jsx-component-placeholder"),
        "{code}"
    );
    assert!(
        code.contains("[unsupported JSX component: TopBar]"),
        "{code}"
    );
    assert!(!code.contains("let { TopBar }"), "{code}");
    assert!(!code.contains("<TopBar>"), "{code}");
}

#[test]
fn emits_component_spreads_and_member_component_refs() {
    let (code, diagnostics) = emit_svelte_jsx_with_diagnostics(
        r#"
function Demo() {
  const props = { title: 'Find' };
  return <Icon.search {...props} size={12} className="icon" />;
}
window.Demo = Demo;
"#,
    );

    assert_eq!(
        diagnostics,
        ["JSX component tag `Icon.search` is outside Phase 1 scope; emitted placeholder"]
    );
    assert!(
        code.contains("htmlswap-jsx-component-placeholder"),
        "{code}"
    );
    assert!(
        code.contains("[unsupported JSX component: Icon.search]"),
        "{code}"
    );
    assert!(!code.contains("let { IconSearch }"), "{code}");
    assert!(!code.contains("<IconSearch"), "{code}");
    assert!(!code.contains("<Icon.search"), "{code}");
}

#[test]
fn unwraps_fragments_and_moves_react_fragment_key_to_each() {
    let code = emit_svelte_jsx(
        r#"
function Demo() {
  return <div>{[1, 2].map(n => <React.Fragment key={n}><span>{n}</span></React.Fragment>)}</div>;
}
window.Demo = Demo;
"#,
    );

    assert!(code.contains("{#each [1, 2] as n (n)}"), "{code}");
    assert!(code.contains("<span>"), "{code}");
    assert!(!code.contains("React.Fragment"), "{code}");
    assert!(!code.contains("jsx-fragment"), "{code}");
}

#[test]
fn lowers_logical_and_ternary_jsx_branches_to_if_blocks() {
    let code = emit_svelte_jsx(
        r#"
function Demo() {
  const active = true;
  const mode = 'a';
  return <div>{active && <span>A</span>}{mode === 'a' ? <b>B</b> : <i>I</i>}</div>;
}
window.Demo = Demo;
"#,
    );

    assert!(code.contains("{#if active}"), "{code}");
    assert!(code.contains("{#if mode === \"a\"}"), "{code}");
    assert!(code.contains("{:else}"), "{code}");
    assert!(!code.contains("&& <"), "{code}");
}

#[test]
fn lowers_map_callbacks_with_early_returns_to_if_else_chains() {
    let code = emit_svelte_jsx(
        r#"
function Demo() {
  const toks = [{ t: 'br' }, { t: 'id', v: 'x' }];
  return <code>{toks.map((t, i) => { if (t.t === 'br') return <br key={i} />; return <span key={i}>{t.v}</span>; })}</code>;
}
window.Demo = Demo;
"#,
    );

    assert!(code.contains("{#each toks as t, i (i)}"), "{code}");
    assert!(code.contains("{#if t.t === \"br\"}"), "{code}");
    assert!(code.contains("{:else}"), "{code}");
    assert!(code.contains("<br />"), "{code}");
}

#[test]
fn lowers_iife_jsx_returns_with_const_locals() {
    let code = emit_svelte_jsx(
        r#"
function Demo() {
  return <svg>{(() => { const x = 1; const y = x + 1; return <text x={x} y={y}>T</text>; })()}</svg>;
}
window.Demo = Demo;
"#,
    );

    assert!(code.contains("{#if true}"), "{code}");
    assert!(code.contains("{@const x = 1}"), "{code}");
    assert!(code.contains("{@const y = x + 1}"), "{code}");
    assert!(!code.contains("return <text"), "{code}");
}

#[test]
fn emits_react_hooks_as_svelte_runes_and_lifecycle_intents() {
    let code = emit_svelte_jsx(
        r#"
function Demo({ first }) {
  const [count, setCount] = React.useState(0);
  const [label, setLabel] = React.useState('');
  const doubled = React.useMemo(() => count * 2, [count]);
  const inputRef = React.useRef(null);
  const bump = React.useCallback(() => setCount(c => c + 1), [count]);
  const sync = () => setLabel(value => { setCount(count + 1); return value; });
  React.useEffect(() => { setLabel(first + ':' + count); }, [first, count]);
  React.useEffect(() => { const id = setInterval(() => console.log(count), 1000); return () => clearInterval(id); }, []);
  React.useEffect(() => { console.log(count); }, [count]);
  return <button ref={inputRef} onClick={() => { setCount(count + 1); setCount(c => c + 1); bump(); sync(); }} data-label={label}>{doubled}</button>;
}
window.Demo = Demo;
"#,
    );

    assert!(code.contains("let count = $state(0);"), "{code}");
    assert!(code.contains("count = count + 1"), "{code}");
    assert!(code.contains("count = (c => c + 1)(count)"), "{code}");
    assert!(
        code.contains("let doubled = $derived(count * 2);"),
        "{code}"
    );
    assert!(
        code.contains("let label = $derived(first + ':' + count);"),
        "{code}"
    );
    assert!(!code.contains("let label = $state"), "{code}");
    assert!(
        code.contains("const bump = () => count = (c => c + 1)(count);"),
        "{code}"
    );
    assert!(
        code.contains("label = (value => { count = count + 1; return value; })(label)"),
        "{code}"
    );
    assert!(code.contains("onMount(() => {"), "{code}");
    assert!(code.contains("let inputRef = null;"), "{code}");
    assert!(code.contains("bind:this={inputRef}"), "{code}");
    assert!(code.contains("$effect(() => {"), "{code}");
    assert!(code.contains("console.log(count);"), "{code}");
}
