use htmlswap::{
    CompileAssets, Compiler, CompilerOptions, Expr, RenderControlFlowKind, RenderElement,
    RenderNode, SourceFrontendKind, SourceKind,
};

fn compile_jsx(source: &str) -> htmlswap::CompiledFragment {
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
    compiled.value
}

#[test]
fn lowers_jsx_host_element_classes_styles_and_text() {
    let compiled = compile_jsx(
        r#"
function Demo() {
  const label = 'Hello';
  const W = 120;
  return <div className="p-2 text-sm" style={{ width: W, borderRadius: 8, color: 'var(--ink)' }}>{label}</div>;
}
window.Demo = Demo;
"#,
    );

    assert_eq!(compiled.sources.files()[0].kind(), SourceKind::JavaScript);
    assert_eq!(compiled.plan.source_logic[0].dialect, "jsx");
    let RenderNode::Element(element) = &compiled.plan.nodes[0] else {
        panic!("expected root element");
    };
    assert_eq!(element.source_tag, "div");
    assert_eq!(element.classes, ["p-2", "text-sm"]);
    assert!(
        element
            .styles
            .iter()
            .any(|style| style.property.as_str() == "border-radius")
    );
    assert!(element.dynamic_styles.iter().any(|style| {
        style
            .expression
            .segments
            .iter()
            .any(|segment| matches!(segment, htmlswap::TemplateSegment::Literal(value) if value == "width: "))
    }));
    assert!(matches!(
        &element.children[0],
        RenderNode::Text(text) if text.template.is_some()
    ));
}

#[test]
fn lowers_map_callbacks_to_keyed_control_flow_with_locals() {
    let compiled = compile_jsx(include_str!("fixtures/jsx/graph-banded.jsx"));
    let flows = collect_control_flow_elements(&compiled.plan.nodes);
    let sector_flow = flows
        .iter()
        .find(|element| {
            element
                .control_flow
                .as_ref()
                .and_then(|flow| flow.binding.as_ref())
                .is_some_and(|binding| binding.name == "s")
        })
        .expect("expected SECTORS each block");
    let flow = sector_flow.control_flow.as_ref().unwrap();
    assert_eq!(flow.kind, RenderControlFlowKind::For);
    assert_eq!(flow.index_binding.as_ref().unwrap().name, "i");
    assert!(matches!(flow.key.as_ref(), Some(Expr::Member { property, .. }) if property == "rel"));
    assert!(flow.locals.iter().any(|local| {
        local.name == "a0"
            && matches!(&local.value, Expr::Opaque(value) if value.contains("(perSector + gap)"))
    }));

    let item_flow = flows
        .iter()
        .find(|element| {
            element
                .control_flow
                .as_ref()
                .and_then(|flow| flow.binding.as_ref())
                .is_some_and(|binding| binding.name == "item")
        })
        .expect("expected item each block");
    let item_flow = item_flow.control_flow.as_ref().unwrap();
    assert!(
        matches!(item_flow.key.as_ref(), Some(Expr::Member { property, .. }) if property == "name")
    );
    assert!(item_flow.locals.iter().any(|local| local.name == "fill"));
}

#[test]
fn discovers_exported_function_components_without_leaking_export_syntax() {
    for source in [
        "export function Demo() { const label = 'named'; return <p>{label}</p>; }",
        "export default function Demo() { const label = 'default'; return <p>{label}</p>; }",
        "function Demo() { const label = 'identifier'; return <p>{label}</p>; } export default Demo;",
    ] {
        let compiled = compile_jsx(source);
        let script = compiled.plan.source_logic[0].body.as_str();
        assert!(script.contains("const label"));
        assert!(!script.contains("export"));
        assert!(
            matches!(&compiled.plan.nodes[0], RenderNode::Element(element) if element.source_tag == "p")
        );
    }
}

#[test]
fn lowers_jsx_components_to_source_intent_and_spread_attrs() {
    let compiled = compile_jsx(
        r#"
function Demo() {
  const props = { tone: 'warm' };
  return <Card {...props} title="Hello"><span>Child</span></Card>;
}
window.Demo = Demo;
"#,
    );

    let RenderNode::Element(element) = &compiled.plan.nodes[0] else {
        panic!("expected component element");
    };
    assert_eq!(element.source_tag, "jsx-component");
    let intent = element
        .source_intent
        .as_ref()
        .expect("component should carry source intent");
    assert!(intent.component.as_ref().is_some_and(|id| id.is("Card")));
    assert!(
        intent
            .props
            .iter()
            .any(|prop| prop.name == "title" && prop.value == "Hello")
    );
    assert!(
        element
            .attributes
            .iter()
            .any(|attribute| attribute.name == "{...}" && attribute.template.is_some())
    );
}

#[test]
fn compiles_shared_component_modules_without_copying_jsx_into_script() {
    let compiled = compile_jsx(
        r#"
const Icon = {
  search: (p = {}) => <svg {...p}><path d="M0 0" /></svg>,
};
const TOKEN = { bg: 'var(--panel)' };
function TopBar() { return <header />; }
Object.assign(window, { Icon, TopBar });
"#,
    );

    assert!(compiled.plan.nodes.is_empty());
    let script = compiled.plan.source_logic[0].body.as_str();
    assert!(script.contains("const TOKEN"));
    assert!(!script.contains("<svg"));
    assert!(!script.contains("function TopBar"));
    assert!(!script.contains("Object.assign"));
}

fn collect_control_flow_elements(nodes: &[RenderNode]) -> Vec<&RenderElement> {
    let mut elements = Vec::new();
    for node in nodes {
        if let RenderNode::Element(element) = node {
            if element.control_flow.is_some() {
                elements.push(element.as_ref());
            }
            elements.extend(collect_control_flow_elements(&element.children));
        }
    }
    elements
}
