use std::io::{Read, Write};
use std::net::TcpListener;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::thread;

use htmlswap::{
    ArcStr, BundleConfig, CancellationToken, CompileAssets, Compiler, CompilerCache,
    CompilerCacheFileSet, CompilerOptions, CompilerParallelism, EmitContext, Emitter, Expr,
    Frontend, JobContext, JobRunner, OutputBoundary, OutputUnitKind, RenderActionArgument,
    RenderActionEffect, RenderActionHandlerEffect, RenderAnnotationKind, RenderControlFlowKind,
    RenderDensity, RenderElement, RenderNode, RenderScriptKind, RenderSize, RenderStateKind,
    RenderStateOwner, RenderStateValueSource, RenderStyleCondition, RenderThemeScope, RenderTone,
    RenderValidationConstraint, RenderVariant, RouteTarget, SourceId, SourceKind, StyleProperty,
    StyleValue, TextEmitter, ThemeTokenKind, UiRole, compile_fragment,
    compile_fragment_with_assets, parse_fragment,
};
use url::Url;

fn temp_workspace(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "htmlswap-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after UNIX epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("temp directory should be created");
    root
}

fn file_url(path: &Path) -> String {
    Url::from_file_path(path)
        .expect("test path should convert to a file URL")
        .to_string()
}

fn find_element_by_tag<'a>(nodes: &'a [RenderNode], tag: &str) -> Option<&'a RenderElement> {
    for node in nodes {
        let RenderNode::Element(element) = node else {
            continue;
        };

        if element.source_tag.as_str() == tag {
            return Some(element);
        }

        if let Some(found) = find_element_by_tag(&element.children, tag) {
            return Some(found);
        }
    }

    None
}

fn collect_elements_by_tag<'a>(
    nodes: &'a [RenderNode],
    tag: &str,
    elements: &mut Vec<&'a RenderElement>,
) {
    for node in nodes {
        let RenderNode::Element(element) = node else {
            continue;
        };

        if element.source_tag.as_str() == tag {
            elements.push(element);
        }
        collect_elements_by_tag(&element.children, tag, elements);
    }
}

#[test]
fn compiles_basic_fragment_to_render_plan() {
    let compiled = compile_fragment(
        r#"<div class="row gap" style="display: flex"><button disabled>Save</button><input placeholder="Name"></div>"#,
    );

    assert!(compiled.diagnostics.is_empty());
    assert_eq!(compiled.value.nodes.len(), 1);

    let RenderNode::Element(root) = &compiled.value.nodes[0] else {
        panic!("expected root element");
    };

    assert_eq!(root.role, UiRole::Container);
    assert_eq!(root.classes, ["row", "gap"]);
    assert_eq!(root.styles[0].property.as_str(), "display");
    assert_eq!(root.styles[0].value.as_str(), "flex");

    let mut emit_context = EmitContext::new();
    let output = TextEmitter
        .emit(&compiled.value, &mut emit_context)
        .expect("text emission should not fail");

    assert_eq!(
        output,
        "container <div> .row.gap {display: flex}\n  button <button> [disabled]\n    text \"Save\"\n  text-input <input> [placeholder=\"Name\"]\n"
    );
}

#[test]
fn compiles_full_html_document_without_fragment_parse_noise() {
    let source = r#"<!doctype html>
<html>
<head>
    <meta charset="utf-8">
    <style>.primary { color: red; }</style>
    <script>function save() {}</script>
</head>
<body>
    <button class="primary" onclick="save()">Save</button>
</body>
</html>"#;
    let compiled = Compiler::new().compile_fragment(source, &CompileAssets::new());

    assert!(
        compiled
            .diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.message.contains("DOCTYPE in body")
                && !diagnostic.message.contains("</body> with no <body>"))
    );
    assert_eq!(compiled.value.plan.nodes.len(), 1);

    let RenderNode::Element(button) = &compiled.value.plan.nodes[0] else {
        panic!("expected button element");
    };

    assert_eq!(button.source_tag, "button");
    assert_eq!(button.styles[0].property, StyleProperty::Color);
    assert_eq!(button.styles[0].value.as_str(), "red");
    assert!(button.actions[0].resolved);
}

#[test]
fn full_html_document_preserves_stripped_root_styles_as_plan_metadata() {
    let source = r#"<!doctype html>
<html style="height: 100%;">
<head>
    <style>
        html, body { width: 100vw; height: 100vh; }
        body { display: flex; background: var(--background); }
        :root { --background: #1b1e23; }
    </style>
</head>
<body style="margin: 0;">
    <main class="app">Editor</main>
</body>
</html>"#;
    let compiled = Compiler::new().compile_fragment(source, &CompileAssets::new());

    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );
    assert_eq!(compiled.value.plan.nodes.len(), 1);

    let root = &compiled.value.plan.root;
    assert!(!root.is_empty());
    assert!(
        root.styles
            .iter()
            .any(|style| style.property == StyleProperty::Width && style.value.as_str() == "100vw")
    );
    assert!(root
        .styles
        .iter()
        .any(|style| style.property == StyleProperty::Height && style.value.as_str() == "100vh"));
    assert!(root
        .styles
        .iter()
        .any(|style| style.property == StyleProperty::Display && style.value.as_str() == "flex"));
    assert!(
        root.styles
            .iter()
            .any(|style| style.property == StyleProperty::Margin && style.value.as_str() == "0")
    );

    let background = compiled
        .value
        .plan
        .theme
        .token("--background")
        .expect("root background token should be tracked");
    assert!(background.referenced);
}

#[test]
fn compiler_defaults_to_vanilla_html_source() {
    let compiled = Compiler::new().compile_fragment(
        r#"<sc-if value="{{ shown }}"><button>{{ label }}</button></sc-if>"#,
        &CompileAssets::new(),
    );

    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let RenderNode::Element(root) = &compiled.value.plan.nodes[0] else {
        panic!("expected sc-if custom element");
    };
    assert_eq!(root.source_tag, "sc-if");
    assert!(root.control_flow.is_none());
    assert!(
        root.attributes
            .iter()
            .any(|attribute| attribute.name == "value" && attribute.template.is_none())
    );
}

#[test]
fn source_semantics_and_regions_lower_to_structured_primitives() {
    let compiled = compile_fragment(
        r#"
            <section data-htmlswap-region="editor.project">
                <button
                    data-htmlswap-variant="outline"
                    data-htmlswap-tone="danger"
                    data-htmlswap-size="small"
                    data-htmlswap-density="dense"
                    data-htmlswap-semantic-emphasis="high"
                >Delete</button>
            </section>
        "#,
    );

    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let RenderNode::Element(root) = &compiled.value.nodes[0] else {
        panic!("expected root section");
    };
    assert_eq!(
        root.region.as_ref().map(|region| region.as_str()),
        Some("editor.project")
    );
    assert!(root.semantics.is_none());

    let button = find_element_by_tag(&compiled.value.nodes, "button")
        .expect("expected button to inherit source region");
    assert_eq!(
        button.region.as_ref().map(|region| region.as_str()),
        Some("editor.project")
    );
    let semantics = button
        .semantics
        .as_ref()
        .expect("button should expose source semantics");
    assert!(matches!(
        semantics.variant.as_ref(),
        Some(RenderVariant::Outline)
    ));
    assert!(matches!(semantics.tone.as_ref(), Some(RenderTone::Danger)));
    assert!(matches!(semantics.size.as_ref(), Some(RenderSize::Sm)));
    assert!(matches!(
        semantics.density.as_ref(),
        Some(RenderDensity::Compact)
    ));
    assert_eq!(semantics.extras.len(), 1);
    assert_eq!(semantics.extras[0].axis, "emphasis");
    assert_eq!(semantics.extras[0].value, "high");
}

#[test]
fn invalid_source_regions_warn_and_do_not_route_descendants() {
    let compiled = compile_fragment(
        r#"<section data-htmlswap-region="src/view.rs"><button>Bad</button></section>"#,
    );

    assert!(compiled.diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("must be a dotted semantic namespace")
    }));

    let RenderNode::Element(root) = &compiled.value.nodes[0] else {
        panic!("expected root section");
    };
    assert!(root.region.is_none());
    let button = find_element_by_tag(&compiled.value.nodes, "button").expect("expected button");
    assert!(button.region.is_none());
}

#[test]
fn source_templates_and_control_flow_lower_to_structured_primitives() {
    let compiled = Compiler::new().with_frontend(Frontend::dc()).compile_fragment(
        r#"<sc-for list="{{ items }}" as="item" hint-placeholder-count="3"><button style="{{ item.style }}" style-hover="color: red;" onmousedown="{{ item.onDown }}">{{ item.label }}</button></sc-for>"#,
        &CompileAssets::new(),
    );

    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let RenderNode::Element(root) = &compiled.value.plan.nodes[0] else {
        panic!("expected sc-for element");
    };
    let control_flow = root
        .control_flow
        .as_ref()
        .expect("sc-for should lower to source control-flow metadata");
    assert!(matches!(control_flow.kind, RenderControlFlowKind::For));
    assert_eq!(
        control_flow
            .binding
            .as_ref()
            .map(|binding| binding.name.as_str()),
        Some("item")
    );
    assert!(matches!(
        control_flow.expression.as_ref(),
        Some(Expr::Path(path)) if path.as_slice() == ["items"]
    ));
    assert_eq!(
        control_flow.placeholder.as_deref(),
        Some("hint-placeholder-count=\"3\"")
    );
    assert!(root.attributes.is_empty());

    let RenderNode::Element(button) = &root.children[0] else {
        panic!("expected child button");
    };
    assert_eq!(button.dynamic_styles.len(), 1);
    assert_eq!(
        button.dynamic_styles[0]
            .expression
            .expressions()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["item.style"]
    );
    assert_eq!(button.style_variants.len(), 1);
    assert!(matches!(
        button.style_variants[0].conditions.as_slice(),
        [RenderStyleCondition::PseudoClass(value)] if value == "hover"
    ));
    assert_eq!(button.actions.len(), 1);
    assert_eq!(button.actions[0].event.as_str(), "mousedown");
    let template = button.actions[0]
        .template
        .as_ref()
        .expect("template event should preserve its source expression");
    assert_eq!(
        template
            .expressions()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["item.onDown"]
    );
}

#[test]
fn dc_dynamic_style_objects_contribute_static_style_declarations() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"
                <x-dc>
                    <div style="{{ item.rowStyle }}" style-hover="{{ item.hoverStyle }}">Project</div>
                    <script data-dc-script>
                        class Component extends DCLogic {
                            renderVals() {
                                const item = {
                                    rowStyle: {
                                        display: 'flex',
                                        alignItems: 'center',
                                        height: '52px',
                                        padding: '0 12px',
                                        background: this.state.active ? '#111111' : '#222222'
                                    },
                                    hoverStyle: {
                                        color: '#ffffff',
                                        gap: '8px'
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

    let root = find_element_by_tag(&compiled.value.plan.nodes, "div").expect("expected div");
    assert_eq!(root.dynamic_styles.len(), 2);
    assert!(root.styles.iter().any(|style| {
        style.property == StyleProperty::Display && style.value.as_str() == "flex"
    }));
    assert!(root.styles.iter().any(|style| {
        style.property == StyleProperty::AlignItems && style.value.as_str() == "center"
    }));
    assert!(root.styles.iter().any(|style| {
        style.property == StyleProperty::Height && style.value.as_str() == "52px"
    }));
    assert!(root.styles.iter().any(|style| {
        style.property == StyleProperty::Padding && style.value.as_str() == "0 12px"
    }));
    assert!(
        root.styles
            .iter()
            .all(|style| style.property != StyleProperty::Background)
    );
    assert!(root.style_variants.iter().any(|variant| {
        matches!(
            variant.conditions.as_slice(),
            [RenderStyleCondition::PseudoClass(state)] if state == "hover"
        ) && variant
            .declarations
            .iter()
            .any(|style| style.property == StyleProperty::Gap && style.value.as_str() == "8px")
    }));
}

#[test]
fn dc_loop_scoped_dynamic_style_objects_resolve_generic_style_fields() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"
                <x-dc>
                    <sc-for list="{{ navItems }}" as="n">
                        <div style="{{ n.style }}">{{ n.label }}</div>
                    </sc-for>
                    <sc-for list="{{ gemGroups }}" as="grp">
                        <sc-for list="{{ grp.items }}" as="g">
                            <span style="{{ g.style }}">{{ g.name }}</span>
                        </sc-for>
                    </sc-for>
                    <script data-dc-script>
                        class Component extends DCLogic {
                            renderVals() {
                                const navItems = rows.map(row => ({
                                    label: row.label,
                                    style: {
                                        display: 'flex',
                                        alignItems: 'center',
                                        gap: '10px',
                                        height: '36px'
                                    }
                                }));
                                const gemGroups = cats.map(cat => ({
                                    items: gems.map(gem => ({
                                        name: gem.name,
                                        style: {
                                            display: 'flex',
                                            alignItems: 'flex-start',
                                            padding: '10px 12px',
                                            borderRadius: '8px'
                                        }
                                    }))
                                }));
                                return { navItems, gemGroups };
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

    let nav = find_element_by_tag(&compiled.value.plan.nodes, "div").expect("expected nav item");
    assert!(nav.styles.iter().any(|style| {
        style.property == StyleProperty::Height && style.value.as_str() == "36px"
    }));
    assert!(
        nav.styles.iter().any(|style| {
            style.property == StyleProperty::Gap && style.value.as_str() == "10px"
        })
    );

    let gem = find_element_by_tag(&compiled.value.plan.nodes, "span").expect("expected gem item");
    assert!(gem.styles.iter().any(|style| {
        style.property == StyleProperty::Padding && style.value.as_str() == "10px 12px"
    }));
    assert!(gem.styles.iter().any(|style| {
        style.property == StyleProperty::BorderRadius && style.value.as_str() == "8px"
    }));
}

#[test]
fn dc_loop_scoped_array_style_objects_keep_common_static_fields() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"
                <x-dc>
                    <sc-for list="{{ bottomTabs }}" as="bt">
                        <div style="{{ bt.style }}">{{ bt.label }}</div>
                    </sc-for>
                    <script data-dc-script>
                        class Component extends DCLogic {
                            renderVals() {
                                const bottomTabs = [
                                    {
                                        label: 'Asset Browser',
                                        style: {
                                            display: 'flex',
                                            alignItems: 'center',
                                            gap: '6px',
                                            height: '32px',
                                            padding: '0 12px',
                                            cursor: 'default',
                                            color: '#e4e7ec',
                                            background: '#1b1e23',
                                            boxShadow: 'inset 0 -2px 0 #4188e0',
                                            fontWeight: 500
                                        }
                                    },
                                    {
                                        label: 'Console',
                                        style: {
                                            display: 'flex',
                                            alignItems: 'center',
                                            gap: '6px',
                                            height: '32px',
                                            padding: '0 12px',
                                            cursor: 'default',
                                            color: '#9aa4b2',
                                            background: 'transparent',
                                            boxShadow: 'none',
                                            fontWeight: 400
                                        }
                                    }
                                ];
                                return { bottomTabs };
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

    let tab = find_element_by_tag(&compiled.value.plan.nodes, "div").expect("expected tab");
    assert!(tab.styles.iter().any(|style| {
        style.property == StyleProperty::Display && style.value.as_str() == "flex"
    }));
    assert!(tab.styles.iter().any(|style| {
        style.property == StyleProperty::AlignItems && style.value.as_str() == "center"
    }));
    assert!(
        tab.styles
            .iter()
            .any(|style| style.property == StyleProperty::Gap && style.value.as_str() == "6px")
    );
    assert!(tab.styles.iter().any(|style| {
        style.property == StyleProperty::Height && style.value.as_str() == "32px"
    }));
    assert!(tab.styles.iter().any(|style| {
        style.property == StyleProperty::Padding && style.value.as_str() == "0 12px"
    }));
    assert!(
        tab.styles
            .iter()
            .all(|style| style.property != StyleProperty::Color
                && style.property != StyleProperty::Background
                && style.property != StyleProperty::BoxShadow
                && style.property != StyleProperty::FontWeight)
    );
}

#[test]
fn dc_nested_loop_array_style_objects_keep_common_static_fields() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"
                <x-dc>
                    <sc-for list="{{ menus }}" as="menu">
                        <sc-for list="{{ menu.items }}" as="item">
                            <button style="{{ item.style }}">{{ item.label }}</button>
                        </sc-for>
                    </sc-for>
                    <script data-dc-script>
                        class Component extends DCLogic {
                            renderVals() {
                                const menus = [
                                    {
                                        label: 'File',
                                        items: [
                                            {
                                                label: 'New',
                                                style: {
                                                    display: 'flex',
                                                    alignItems: 'center',
                                                    gap: '8px',
                                                    minHeight: '28px',
                                                    padding: '0 10px',
                                                    color: '#e4e7ec'
                                                }
                                            },
                                            {
                                                label: 'Open',
                                                style: {
                                                    display: 'flex',
                                                    alignItems: 'center',
                                                    gap: '8px',
                                                    minHeight: '28px',
                                                    padding: '0 10px',
                                                    color: '#9aa4b2'
                                                }
                                            }
                                        ]
                                    },
                                    {
                                        label: 'Edit',
                                        items: [
                                            {
                                                label: 'Undo',
                                                style: {
                                                    display: 'flex',
                                                    alignItems: 'center',
                                                    gap: '8px',
                                                    minHeight: '28px',
                                                    padding: '0 10px',
                                                    color: '#cad0da'
                                                }
                                            }
                                        ]
                                    }
                                ];
                                return { menus };
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

    let item = find_element_by_tag(&compiled.value.plan.nodes, "button").expect("expected button");
    assert!(item.styles.iter().any(|style| {
        style.property == StyleProperty::Display && style.value.as_str() == "flex"
    }));
    assert!(item.styles.iter().any(|style| {
        style.property == StyleProperty::MinHeight && style.value.as_str() == "28px"
    }));
    assert!(
        item.styles
            .iter()
            .all(|style| style.property != StyleProperty::Color)
    );
}

#[test]
fn dc_dynamic_style_helpers_and_aliases_contribute_static_fields() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"
                <x-dc>
                    <sc-for list="{{ tabs }}" as="tab">
                        <button style="{{ tab.style }}">{{ tab.label }}</button>
                    </sc-for>
                    <div style="{{ leftToggle.style }}">Left</div>
                    <div style="{{ pipeSegStyle }}">Pipeline</div>
                    <div style="{{ diagSegStyle }}">Diagnostics</div>
                    <script data-dc-script>
                        class Component extends DCLogic {
                            tabStyle(active) {
                                return {
                                    display: 'flex',
                                    alignItems: 'center',
                                    gap: '6px',
                                    height: '32px',
                                    padding: '0 12px',
                                    color: active ? '#fff' : '#999',
                                    background: active ? '#111' : 'transparent',
                                    fontWeight: active ? 500 : 400
                                };
                            }

                            renderVals() {
                                const pBtn = on => ({
                                    width: '26px',
                                    height: '20px',
                                    display: 'flex',
                                    alignItems: 'center',
                                    justifyContent: 'center',
                                    borderRadius: '4px',
                                    color: on ? '#4188e0' : '#5a6068'
                                });
                                const segB = {
                                    display: 'flex',
                                    alignItems: 'center',
                                    gap: '6px',
                                    padding: '0 12px',
                                    cursor: 'default',
                                    borderLeft: '1px solid #2c313a'
                                };
                                return {
                                    tabs: [
                                        { label: 'Asset Browser', style: this.tabStyle(true) },
                                        { label: 'Console', style: this.tabStyle(false) }
                                    ],
                                    leftToggle: { style: pBtn(true) },
                                    pipeSegStyle: segB,
                                    diagSegStyle: { ...segB, gap: '9px' }
                                };
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

    let button =
        find_element_by_tag(&compiled.value.plan.nodes, "button").expect("expected tab button");
    assert!(button.styles.iter().any(|style| {
        style.property == StyleProperty::Display && style.value.as_str() == "flex"
    }));
    assert!(button.styles.iter().any(|style| {
        style.property == StyleProperty::Height && style.value.as_str() == "32px"
    }));
    assert!(button.styles.iter().all(|style| {
        style.property != StyleProperty::Color
            && style.property != StyleProperty::Background
            && style.property != StyleProperty::FontWeight
    }));

    let mut divs = Vec::new();
    collect_elements_by_tag(&compiled.value.plan.nodes, "div", &mut divs);
    let left_toggle = divs[0];
    assert!(
        left_toggle.styles.iter().any(|style| {
            style.property == StyleProperty::Width && style.value.as_str() == "26px"
        })
    );
    assert!(left_toggle.styles.iter().any(|style| {
        style.property == StyleProperty::JustifyContent && style.value.as_str() == "center"
    }));

    let pipe = divs[1];
    assert!(pipe.styles.iter().any(|style| {
        style.property == StyleProperty::Padding && style.value.as_str() == "0 12px"
    }));
    assert!(
        pipe.styles
            .iter()
            .any(|style| style.property == StyleProperty::Gap && style.value.as_str() == "6px")
    );

    let diag = divs[2];
    assert!(
        diag.styles
            .iter()
            .any(|style| style.property == StyleProperty::Gap && style.value.as_str() == "9px")
    );
    assert!(diag.styles.iter().any(|style| {
        style.property == StyleProperty::BorderLeft && style.value.as_str() == "1px solid #2c313a"
    }));
}

#[test]
fn dc_frontend_unwraps_x_dc_root_and_preserves_helmet_script_references() {
    let source = r#"<x-dc>
        <helmet>
            <script type="module" src="logic.js"></script>
            <style>.ms { font-family: 'Material Symbols Outlined'; }</style>
            <script data-dc-script data-props='{"title":{"tsType":"string"}}'>
                class Component extends DCLogic { renderVals() { return { title: "Aether" }; } }
            </script>
        </helmet>
        <section>{{ title }}</section>
    </x-dc>"#;
    let assets = CompileAssets::new().with_script(
        Some("logic.js".to_owned()),
        "export function save() { return true; }",
    );
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment_named(Some("View.dc.html".to_owned()), source, &assets);

    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );
    assert_eq!(compiled.value.plan.nodes.len(), 1);
    let RenderNode::Element(section) = &compiled.value.plan.nodes[0] else {
        panic!("expected rendered section, not the x-dc wrapper");
    };
    assert_eq!(section.source_tag, "section");
    assert!(
        compiled
            .value
            .plan
            .head
            .iter()
            .any(|element| element.html.contains(".ms { font-family"))
    );
    assert_eq!(compiled.value.plan.scripts.len(), 1);
    assert_eq!(compiled.value.plan.scripts[0].src, "logic.js");
    assert_eq!(
        compiled.value.plan.scripts[0].kind,
        RenderScriptKind::Module
    );
    assert_eq!(
        compiled.value.plan.scripts[0].resolved_source,
        Some(SourceId::new(1))
    );
    assert!(compiled.value.plan.annotations.iter().any(|annotation| {
        annotation.kind == RenderAnnotationKind::SourceMetadata
            && annotation
                .value
                .contains(r#"{"title":{"tsType":"string"}}"#)
    }));
}

#[test]
fn dc_frontend_lowers_component_imports_to_source_intent() {
    let compiled = Compiler::new().with_frontend(Frontend::dc()).compile_fragment(
        r#"<dc-import name="Card" item="{{ item }}" label="Project {{ item.name }}" hint-size="100%,120px" data-kind="project"><span>Child</span></dc-import><x-import component="Chart" from="./Chart.jsx" dc-props="{{ chartProps }}" hint-size="320px,180px"></x-import>"#,
        &CompileAssets::new(),
    );

    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );
    let RenderNode::Element(root) = &compiled.value.plan.nodes[0] else {
        panic!("expected dc-import element");
    };
    let hints = root
        .source_intent
        .as_ref()
        .expect("dc-import should expose component source intent");
    assert!(hints.component.as_ref().is_some_and(|id| id.is("Card")));
    assert!(
        hints
            .props
            .iter()
            .any(|prop| prop.name == "item" && prop.value == "{{ item }}")
    );
    let item_prop = hints
        .props
        .iter()
        .find(|prop| prop.name == "item")
        .expect("item prop should be present");
    assert!(matches!(
        item_prop
            .template
            .as_ref()
            .and_then(|template| template.single_expression()),
        Some(Expr::Path(path)) if path.as_slice() == ["item"]
    ));
    assert!(
        hints
            .props
            .iter()
            .any(|prop| prop.name == "hintSize" && prop.value == "100%,120px")
    );
    assert!(
        hints
            .props
            .iter()
            .find(|prop| prop.name == "hintSize")
            .is_some_and(|prop| prop.template.is_none())
    );
    let label_prop = hints
        .props
        .iter()
        .find(|prop| prop.name == "label")
        .expect("label prop should be present");
    assert!(label_prop.template.as_ref().is_some_and(|template| {
        template.single_expression().is_none()
            && template
                .expressions()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                == ["item.name"]
    }));
    assert!(
        hints
            .props
            .iter()
            .any(|prop| prop.name == "data-kind" && prop.value == "project")
    );

    let RenderNode::Element(x_import) = &compiled.value.plan.nodes[1] else {
        panic!("expected x-import element");
    };
    let hints = x_import
        .source_intent
        .as_ref()
        .expect("x-import should expose component source intent");
    assert!(hints.component.as_ref().is_some_and(|id| id.is("Chart")));
    assert_eq!(hints.component_source.as_deref(), Some("./Chart.jsx"));
    assert!(
        hints
            .props
            .iter()
            .any(|prop| prop.name == "from" && prop.value == "./Chart.jsx")
    );
    assert!(
        hints
            .props
            .iter()
            .any(|prop| prop.name == "dcProps" && prop.value == "{{ chartProps }}")
    );
    let dc_props = hints
        .props
        .iter()
        .find(|prop| prop.name == "dcProps")
        .expect("dcProps should be present");
    assert!(matches!(
        dc_props
            .template
            .as_ref()
            .and_then(|template| template.single_expression()),
        Some(Expr::Path(path)) if path.as_slice() == ["chartProps"]
    ));
    assert!(
        hints
            .props
            .iter()
            .any(|prop| prop.name == "hintSize" && prop.value == "320px,180px")
    );
}

#[test]
fn dc_frontend_warns_for_non_grammar_holes() {
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment(
            r#"<button title="{{ a + b }}">{{ save() }}</button>"#,
            &CompileAssets::new(),
        );

    let messages = compiled
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 2);
    assert!(
        messages
            .iter()
            .any(|message| message.contains("DC hole `a + b`"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("DC hole `save()`"))
    );
}

#[test]
fn vue_frontend_lowers_bindings_events_and_control_flow() {
    let compiled = Compiler::new().with_frontend(Frontend::vue()).compile_fragment(
        r#"<ul><li v-for="item in items" :title="item.title" :style="item.style" @click="item.onClick">{{ item.label }}</li></ul>"#,
        &CompileAssets::new(),
    );

    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let list_item =
        find_element_by_tag(&compiled.value.plan.nodes, "li").expect("expected Vue loop list item");
    let control_flow = list_item
        .control_flow
        .as_ref()
        .expect("v-for should lower to source control-flow metadata");
    assert!(matches!(control_flow.kind, RenderControlFlowKind::For));
    assert_eq!(
        control_flow
            .binding
            .as_ref()
            .map(|binding| binding.name.as_str()),
        Some("item")
    );
    assert!(matches!(
        control_flow.expression.as_ref(),
        Some(Expr::Path(path)) if path.as_slice() == ["items"]
    ));
    assert_eq!(list_item.actions.len(), 1);
    assert_eq!(list_item.actions[0].event.as_str(), "click");
    assert_eq!(
        list_item.actions[0]
            .template
            .as_ref()
            .expect("Vue event path should become a template handler")
            .expressions()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["item.onClick"]
    );
    assert_eq!(
        list_item.dynamic_styles[0]
            .expression
            .expressions()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["item.style"]
    );
    let title = list_item
        .attributes
        .iter()
        .find(|attribute| attribute.name == "title")
        .expect("dynamic title should stay a target attribute");
    assert_eq!(
        title
            .template
            .as_ref()
            .expect("dynamic title should carry a template")
            .expressions()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["item.title"]
    );

    let RenderNode::Text(text) = &list_item.children[0] else {
        panic!("expected templated text child");
    };
    assert_eq!(
        text.template
            .as_ref()
            .expect("Vue mustache should become a template")
            .expressions()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["item.label"]
    );
}

#[test]
fn vue_frontend_lowers_conditionals_and_v_model_initial_state() {
    let compiled = Compiler::new().with_frontend(Frontend::vue()).compile_fragment(
        r#"<section><p v-if="isReady">Ready</p><p v-else>Fallback</p><input v-model="name" placeholder="Name"></section>"#,
        &CompileAssets::new(),
    );

    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let RenderNode::Element(section) = &compiled.value.plan.nodes[0] else {
        panic!("expected section");
    };
    let RenderNode::Element(ready) = &section.children[0] else {
        panic!("expected conditional paragraph");
    };
    assert!(matches!(
        ready.control_flow.as_ref().map(|flow| &flow.kind),
        Some(RenderControlFlowKind::If)
    ));
    assert!(matches!(
        ready
            .control_flow
            .as_ref()
            .and_then(|flow| flow.expression.as_ref()),
        Some(Expr::Path(path)) if path.as_slice() == ["isReady"]
    ));

    let RenderNode::Element(fallback) = &section.children[1] else {
        panic!("expected else paragraph");
    };
    assert!(matches!(
        fallback.control_flow.as_ref().map(|flow| &flow.kind),
        Some(RenderControlFlowKind::Else)
    ));

    let input =
        find_element_by_tag(&compiled.value.plan.nodes, "input").expect("expected v-model input");
    let state = input.state.as_ref().expect("input should bind state");
    let RenderStateKind::TextInput(text) = &state.kind else {
        panic!("v-model input should be text state");
    };
    assert!(text.initial_value.is_none());
    assert_eq!(
        text.initial_template
            .as_ref()
            .expect("v-model should become a dynamic initial value")
            .expressions()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["name"]
    );
}

#[test]
fn source_switch_lowers_to_first_class_control_flow_primitives() {
    let compiled = Compiler::new().with_frontend(Frontend::dc()).compile_fragment(
        r#"<sc-switch value="{{ activeView }}"><sc-case value="{{ sceneView }}">Scene</sc-case><sc-default>Other</sc-default></sc-switch>"#,
        &CompileAssets::new(),
    );

    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let RenderNode::Element(root) = &compiled.value.plan.nodes[0] else {
        panic!("expected sc-switch element");
    };
    let control_flow = root
        .control_flow
        .as_ref()
        .expect("sc-switch should lower to source control-flow metadata");
    assert!(matches!(control_flow.kind, RenderControlFlowKind::Switch));
    assert!(matches!(
        control_flow.expression.as_ref(),
        Some(Expr::Path(path)) if path.as_slice() == ["activeView"]
    ));

    let RenderNode::Element(case) = &root.children[0] else {
        panic!("expected sc-case element");
    };
    assert!(matches!(
        case.control_flow.as_ref().map(|flow| &flow.kind),
        Some(RenderControlFlowKind::Case)
    ));

    let RenderNode::Element(default) = &root.children[1] else {
        panic!("expected sc-default element");
    };
    assert!(matches!(
        default.control_flow.as_ref().map(|flow| &flow.kind),
        Some(RenderControlFlowKind::Default)
    ));
}

#[test]
fn compiler_parallelism_options_preserve_ordered_semantics() {
    let source =
        r#"<script>function save() {}</script><button class="card" onclick="save()">Save</button>"#;
    let assets = CompileAssets::new()
        .with_stylesheet(Some("a.css".to_owned()), ".card { color: red; }")
        .with_stylesheet(Some("b.css".to_owned()), ".card { color: blue; }")
        .with_script(Some("external.js".to_owned()), "function save() {}");
    let sequential = Compiler::try_with_options(
        CompilerOptions::new().with_parallelism(CompilerParallelism::sequential()),
    )
    .expect("sequential compiler should build")
    .compile_fragment(source, &assets);
    let parallel = Compiler::try_with_options(CompilerOptions::new().with_parallelism(
        CompilerParallelism::rayon_threads(NonZeroUsize::new(2).expect("nonzero thread count")),
    ))
    .expect("rayon compiler should build")
    .compile_fragment(source, &assets);

    assert!(sequential.diagnostics.is_empty());
    assert!(parallel.diagnostics.is_empty());
    assert_eq!(parallel.value.plan, sequential.value.plan);

    let RenderNode::Element(button) = &parallel.value.plan.nodes[0] else {
        panic!("expected button element");
    };
    let style_span = button.styles[0]
        .span
        .expect("winning CSS declaration should keep a source span");
    let style_source = parallel
        .value
        .sources
        .file(style_span.source)
        .expect("style source should be registered");
    assert_eq!(style_source.name(), Some("b.css"));

    let action_span = button.actions[0]
        .action_span
        .expect("inline action should win before external action");
    let action_source = parallel
        .value
        .sources
        .file(action_span.source)
        .expect("action source should be registered");
    assert_eq!(action_source.kind(), SourceKind::Html);
}

#[test]
fn compiler_default_options_use_auto_parallelism() {
    assert_eq!(
        Compiler::new().options().parallelism,
        CompilerParallelism::auto()
    );
}

#[test]
fn compiled_fragments_include_bundle_plan() {
    let compiled = Compiler::new().compile_fragment("<button>Save</button>", &CompileAssets::new());

    assert!(compiled.diagnostics.is_empty());
    assert_eq!(compiled.value.bundle.units.len(), 1);
    assert_eq!(compiled.value.bundle.units[0].kind, OutputUnitKind::View);
    assert!(matches!(
        compiled.value.bundle.units[0].boundary,
        OutputBoundary::SameModule
    ));
}

#[test]
fn compiler_job_context_observes_cancellation() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    let compiler = Compiler::with_jobs(
        CompilerOptions::new(),
        JobContext::new(JobRunner::inline(), cancel),
    );
    let compiled = compiler.compile_fragment("<div>Cancelled</div>", &CompileAssets::new());

    assert!(compiled.diagnostics.has_errors());
    assert!(
        compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message == "compilation cancelled")
    );
    assert!(compiled.value.plan.nodes.is_empty());
    assert_eq!(compiled.value.sources.files().len(), 1);
}

#[test]
fn compiler_cache_rebases_external_asset_spans_to_current_sources() {
    let cache = CompilerCache::new();
    let compiler = Compiler::new().with_cache(cache.clone());
    let source = r#"<button class="card" onclick="save()">Save</button>"#;
    let app_css = ".card { color: red; }";
    let app_js = "const save = () => {};";

    let first_assets = CompileAssets::new()
        .with_stylesheet(Some("app.css".to_owned()), app_css)
        .with_script(Some("app.js".to_owned()), app_js);
    let first = compiler.compile_fragment(source, &first_assets);
    assert!(first.diagnostics.is_empty());

    let second_assets = CompileAssets::new()
        .with_stylesheet(Some("reset.css".to_owned()), "button { padding: 0; }")
        .with_stylesheet(Some("app.css".to_owned()), app_css)
        .with_script(Some("app.js".to_owned()), app_js);
    let second = compiler.compile_fragment(source, &second_assets);
    assert!(second.diagnostics.is_empty());

    let RenderNode::Element(button) = &second.value.plan.nodes[0] else {
        panic!("expected button element");
    };
    let style_span = button
        .styles
        .iter()
        .find(|style| style.property.as_str() == "color")
        .expect("cached CSS declaration should be present")
        .span
        .expect("cached CSS span should be present");
    let style_source = second
        .value
        .sources
        .file(style_span.source)
        .expect("cached CSS source should resolve");
    assert_eq!(style_source.name(), Some("app.css"));
    assert_eq!(
        second
            .value
            .sources
            .source_text(style_span)
            .expect("cached CSS span should resolve"),
        app_css
    );

    let action_span = button.actions[0]
        .action_span
        .expect("cached JS action span should be present");
    let action_source = second
        .value
        .sources
        .file(action_span.source)
        .expect("cached JS source should resolve");
    assert_eq!(action_source.name(), Some("app.js"));
    assert_eq!(
        second
            .value
            .sources
            .source_text(action_span)
            .expect("cached JS span should resolve"),
        "save = () => {}"
    );

    let stats = cache.stats();
    assert_eq!(stats.html_entries, 1);
    assert_eq!(stats.stylesheet_entries, 2);
    assert_eq!(stats.script_entries, 1);
}

#[test]
fn compiler_cache_reuses_inline_stylesheets_and_scripts() {
    let cache = CompilerCache::new();
    let compiler = Compiler::new().with_cache(cache.clone());
    let source = r#"
        <style>.card { color: red; }</style>
        <script>const save = () => {};</script>
        <button class="card" onclick="save()">Save</button>
    "#;

    let first = compiler.compile_fragment(source, &CompileAssets::new());
    assert!(first.diagnostics.is_empty());
    let second = compiler.compile_fragment(source, &CompileAssets::new());
    assert!(second.diagnostics.is_empty());

    let RenderNode::Element(button) = &second.value.plan.nodes[0] else {
        panic!("expected button element");
    };
    assert_eq!(button.styles[0].value.as_str(), "red");
    assert!(button.actions[0].resolved);

    let stats = cache.stats();
    assert_eq!(stats.html_entries, 1);
    assert_eq!(stats.inline_stylesheet_entries, 1);
    assert_eq!(stats.inline_script_entries, 1);
}

#[test]
fn compiler_cache_file_set_detects_changed_inputs() {
    let root = std::env::temp_dir().join(format!(
        "htmlswap-file-set-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after UNIX epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("temp directory should be created");
    let path = root.join("input.html");
    std::fs::write(&path, "<div>One</div>").expect("input should be written");

    let mut file_set = CompilerCacheFileSet::read([&path]).expect("file set should read input");
    assert!(
        !file_set
            .refresh_changed()
            .expect("unchanged file should read")
    );

    std::fs::write(&path, "<div>Changed</div>").expect("input should be updated");
    assert!(
        file_set
            .refresh_changed()
            .expect("changed file should read")
    );
    assert!(
        !file_set
            .refresh_changed()
            .expect("refreshed file should read")
    );

    std::fs::remove_dir_all(root).expect("temp directory should be removed");
}

#[test]
fn text_controls_lower_to_state_and_form_control_primitives() {
    let compiled = compile_fragment(
        r#"
            <script>function save() {}</script>
            <form onsubmit="save()">
                <label for="name">Full name</label>
                <input id="name" name="full_name" type="email" value="Ada" placeholder="Name" required minlength="2" maxlength="64" pattern=".+@.+" oninput="save()">
                <textarea id="bio" rows="4" placeholder="Bio">About</textarea>
            </form>
        "#,
    );

    assert!(compiled.diagnostics.is_empty());

    let RenderNode::Element(form) = &compiled.value.nodes[0] else {
        panic!("expected form element");
    };
    let RenderNode::Element(input) = &form.children[1] else {
        panic!("expected input element");
    };
    let RenderNode::Element(textarea) = &form.children[2] else {
        panic!("expected textarea element");
    };

    assert_eq!(compiled.value.state.bindings.len(), 2);
    assert_eq!(compiled.value.state.bindings[0].id, "name");
    assert_eq!(compiled.value.state.bindings[1].id, "bio");
    assert_eq!(
        compiled.value.state.bindings[0].owner,
        RenderStateOwner::Target
    );
    assert_eq!(compiled.value.state.forms.len(), 1);
    assert_eq!(compiled.value.state.forms[0].id, "form_0");
    assert_eq!(compiled.value.state.forms[0].controls.len(), 2);
    assert!(compiled.value.state.forms[0].validation.required);
    assert_eq!(
        compiled.value.state.forms[0].controls[0]
            .state_id
            .as_deref(),
        Some("name")
    );
    assert_eq!(
        compiled.value.state.forms[0].controls[0]
            .control
            .name
            .as_deref(),
        Some("full_name")
    );
    assert_eq!(
        compiled.value.state.forms[0].controls[0]
            .control
            .label
            .as_deref(),
        Some("Full name")
    );
    assert_eq!(
        compiled.value.state.forms[0].controls[0]
            .control
            .validation
            .constraints,
        [
            RenderValidationConstraint::InputType("email".into()),
            RenderValidationConstraint::MinLength(2),
            RenderValidationConstraint::MaxLength(64),
            RenderValidationConstraint::Pattern(".+@.+".into()),
        ]
    );
    assert_eq!(
        compiled.value.state.forms[0].controls[1]
            .state_id
            .as_deref(),
        Some("bio")
    );
    assert_eq!(compiled.value.state.actions.len(), 2);
    assert!(matches!(
        &compiled.value.state.actions[0].effects[..],
        [
            RenderActionEffect::ValidateForm { .. },
            RenderActionEffect::SubmitForm(_),
            RenderActionEffect::Invoke { .. },
        ]
    ));
    assert_eq!(
        compiled.value.state.actions[1].effects[0],
        RenderActionEffect::UpdateState {
            state_id: "name".into(),
            source: RenderStateValueSource::EventValue,
        }
    );

    let input_state = input.state.as_ref().expect("input should bind state");
    assert_eq!(input_state.id, "name");
    let RenderStateKind::TextInput(input_state) = &input_state.kind else {
        panic!("input state should be a text input");
    };
    assert_eq!(input_state.initial_value.as_deref(), Some("Ada"));
    assert_eq!(input_state.placeholder.as_deref(), Some("Name"));
    assert!(!input_state.multiline);

    let control = input
        .form_control
        .as_ref()
        .expect("input should bind form control metadata");
    assert_eq!(control.name.as_deref(), Some("full_name"));
    assert_eq!(control.label.as_deref(), Some("Full name"));
    assert!(control.required);

    let textarea_state = textarea.state.as_ref().expect("textarea should bind state");
    let RenderStateKind::TextInput(textarea_state) = &textarea_state.kind else {
        panic!("textarea state should be a text input");
    };
    assert_eq!(textarea_state.initial_value.as_deref(), Some("About"));
    assert_eq!(textarea_state.placeholder.as_deref(), Some("Bio"));
    assert!(textarea_state.multiline);
    assert_eq!(textarea_state.rows, Some(4));
}

#[test]
fn imported_script_bindings_resolve_event_handlers() {
    let assets = CompileAssets::new().with_script(
        Some("app.mjs".to_owned()),
        "import { save } from './actions.js';",
    );
    let compiled = Compiler::new()
        .compile_fragment(r#"<button onclick="return save();">Save</button>"#, &assets);

    assert!(compiled.diagnostics.is_empty());

    let RenderNode::Element(button) = &compiled.value.plan.nodes[0] else {
        panic!("expected button element");
    };

    let action = &button.actions[0];
    assert_eq!(action.action.as_deref(), Some("save"));
    assert!(action.resolved);

    let action_span = action
        .action_span
        .expect("imported action should keep source span");
    let action_source = compiled
        .value
        .sources
        .file(action_span.source)
        .expect("action source should resolve");
    assert_eq!(action_source.name(), Some("app.mjs"));
    assert_eq!(
        compiled
            .value
            .sources
            .source_text(action_span)
            .expect("action span should resolve"),
        "save"
    );
}

#[test]
fn external_script_references_are_preserved_for_cdn_sources() {
    let source = r#"
        <script src="https://cdn.example.test/actions.mjs" type="module" async></script>
        <button onclick="save()">Save</button>
    "#;
    let compiled =
        Compiler::try_with_options(CompilerOptions::new().with_external_script_resolution(false))
            .expect("compiler should build")
            .compile_fragment(source, &CompileAssets::new());

    assert!(
        compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("onclick"))
    );
    assert_eq!(compiled.value.plan.scripts.len(), 1);
    let script = &compiled.value.plan.scripts[0];
    assert_eq!(script.src, "https://cdn.example.test/actions.mjs");
    assert_eq!(script.kind, RenderScriptKind::Module);
    assert!(script.async_script);
    assert!(!script.defer);
    assert!(script.span.is_some());
    assert!(script.resolved_source.is_none());

    let assets = CompileAssets::new().with_script(
        Some("https://cdn.example.test/actions.mjs".to_owned()),
        "export function save() {}",
    );
    let resolved = Compiler::new().compile_fragment(source, &assets);

    assert!(resolved.diagnostics.is_empty());
    assert_eq!(resolved.value.plan.scripts.len(), 1);
    let script = &resolved.value.plan.scripts[0];
    let resolved_source = script
        .resolved_source
        .expect("matching script asset should resolve the CDN reference");
    let source_file = resolved
        .value
        .sources
        .file(resolved_source)
        .expect("resolved script source should be registered");
    assert_eq!(
        source_file.name(),
        Some("https://cdn.example.test/actions.mjs")
    );
}

#[test]
fn external_script_urls_are_resolved_and_analyzed() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let address = listener
        .local_addr()
        .expect("test server address should resolve");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("test request should connect");
        let mut request = [0_u8; 1024];
        let _ = stream
            .read(&mut request)
            .expect("test request should be readable");
        let body = "export function save(event) { event.preventDefault(); }";
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("test response should write");
    });

    let url = format!("http://{address}/actions.mjs");
    let source = format!(
        r#"<script src="{url}" type="module"></script><button onclick="save(event)">Save</button>"#
    );
    let compiled = Compiler::new().compile_fragment(source, &CompileAssets::new());
    server.join().expect("test server should finish");

    assert!(compiled.diagnostics.is_empty());
    assert_eq!(compiled.value.plan.scripts.len(), 1);
    let script = &compiled.value.plan.scripts[0];
    let resolved_source = script
        .resolved_source
        .expect("fetched CDN script should be registered");
    let source_file = compiled
        .value
        .sources
        .file(resolved_source)
        .expect("fetched script source should resolve");
    assert_eq!(source_file.name(), Some(url.as_str()));
    assert!(source_file.text().contains("function save"));

    let RenderNode::Element(button) = &compiled.value.plan.nodes[0] else {
        panic!("expected button element");
    };
    let action = &button.actions[0];
    assert!(action.resolved);
    assert_eq!(action.action.as_deref(), Some("save"));
    assert!(matches!(
        action.handler.effects.as_slice(),
        [RenderActionHandlerEffect::PreventDefault { .. }]
    ));
    let action_span = action
        .action_span
        .expect("resolved CDN action should keep fetched source span");
    assert_eq!(action_span.source, resolved_source);
}

#[test]
fn linked_stylesheets_and_css_imports_resolve_from_filesystem() {
    let root = temp_workspace("linked-css");
    let html_path = root.join("index.html");
    let app_css = root.join("app.css");
    let theme_css = root.join("theme.css");
    std::fs::write(
        &app_css,
        "@import \"./theme.css\";\nbutton.primary { color: red; }",
    )
    .expect("app stylesheet should be written");
    std::fs::write(&theme_css, ".primary { background-color: blue; }")
        .expect("theme stylesheet should be written");

    let compiled = Compiler::new().compile_fragment_named(
        Some(file_url(&html_path)),
        r#"<link rel="stylesheet" href="app.css"><button class="primary">Save</button>"#,
        &CompileAssets::new(),
    );

    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let button = find_element_by_tag(&compiled.value.plan.nodes, "button")
        .expect("button should be lowered");
    let color = button
        .styles
        .iter()
        .find(|style| style.property.as_str() == "color")
        .expect("linked stylesheet should apply color");
    let background = button
        .styles
        .iter()
        .find(|style| style.property.as_str() == "background-color")
        .expect("imported stylesheet should apply background color");

    assert_eq!(color.value.as_str(), "red");
    assert_eq!(background.value.as_str(), "#00f");

    let app_url = file_url(&app_css);
    let theme_url = file_url(&theme_css);
    let color_source = compiled
        .value
        .sources
        .file(
            color
                .span
                .expect("color style should keep source span")
                .source,
        )
        .expect("color style source should resolve");
    let background_source = compiled
        .value
        .sources
        .file(
            background
                .span
                .expect("background style should keep source span")
                .source,
        )
        .expect("background style source should resolve");

    assert_eq!(color_source.name(), Some(app_url.as_str()));
    assert_eq!(background_source.name(), Some(theme_url.as_str()));
    assert!(
        compiled
            .value
            .sources
            .source_text(background.span.expect("background span should exist"))
            .expect("background span should resolve")
            .contains("background-color: blue")
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn script_import_graph_resolves_filesystem_modules_with_typescript_syntax() {
    let root = temp_workspace("script-imports");
    let html_path = root.join("index.html");
    let app_ts = root.join("app.ts");
    let actions_ts = root.join("actions.ts");
    let dynamic_ts = root.join("dynamic.ts");
    std::fs::write(
        &app_ts,
        "import { save } from './actions.ts';\nimport('./dynamic.ts');",
    )
    .expect("app script should be written");
    std::fs::write(
        &actions_ts,
        "export function save(event: Event): void { event.preventDefault(); }",
    )
    .expect("actions script should be written");
    std::fs::write(
        &dynamic_ts,
        "export function hover(event: MouseEvent): void { event.stopPropagation(); }",
    )
    .expect("dynamic script should be written");

    let compiled = Compiler::new().compile_fragment_named(
        Some(file_url(&html_path)),
        r#"<script src="app.ts" type="module"></script><button onclick="save(event)" onmouseover="hover(event)">Save</button>"#,
        &CompileAssets::new(),
    );

    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let app_url = file_url(&app_ts);
    let actions_url = file_url(&actions_ts);
    let dynamic_url = file_url(&dynamic_ts);
    let script = compiled
        .value
        .plan
        .scripts
        .first()
        .expect("script reference should be preserved");
    let resolved_source = script
        .resolved_source
        .expect("relative script src should point to the resolved source");
    let script_source = compiled
        .value
        .sources
        .file(resolved_source)
        .expect("resolved script source should exist");
    assert_eq!(script_source.name(), Some(app_url.as_str()));
    assert!(
        compiled
            .value
            .sources
            .files()
            .iter()
            .any(|source| source.name() == Some(actions_url.as_str()))
    );
    assert!(
        compiled
            .value
            .sources
            .files()
            .iter()
            .any(|source| source.name() == Some(dynamic_url.as_str()))
    );

    let button = find_element_by_tag(&compiled.value.plan.nodes, "button")
        .expect("button should be lowered");
    let click = button
        .actions
        .iter()
        .find(|action| action.event.as_str() == "click")
        .expect("click action should be present");
    let hover = button
        .actions
        .iter()
        .find(|action| action.event.as_str() == "mouseover")
        .expect("mouseover action should be present");

    assert!(click.resolved);
    assert_eq!(click.action.as_deref(), Some("save"));
    assert!(matches!(
        click.handler.effects.as_slice(),
        [RenderActionHandlerEffect::PreventDefault { .. }]
    ));
    let click_source = compiled
        .value
        .sources
        .file(
            click
                .action_span
                .expect("click action should keep definition source span")
                .source,
        )
        .expect("click action source should resolve");
    assert_eq!(click_source.name(), Some(actions_url.as_str()));

    assert!(hover.resolved);
    assert_eq!(hover.action.as_deref(), Some("hover"));
    assert!(matches!(
        hover.handler.effects.as_slice(),
        [RenderActionHandlerEffect::StopPropagation { .. }]
    ));
    let hover_source = compiled
        .value
        .sources
        .file(
            hover
                .action_span
                .expect("hover action should keep definition source span")
                .source,
        )
        .expect("hover action source should resolve");
    assert_eq!(hover_source.name(), Some(dynamic_url.as_str()));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn inline_scripts_default_to_typescript_syntax() {
    let compiled = Compiler::new().compile_fragment(
        r#"<script>const save = (event: MouseEvent): void => { event.preventDefault(); };</script><button onclick="save(event)">Save</button>"#,
        &CompileAssets::new(),
    );

    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let button = find_element_by_tag(&compiled.value.plan.nodes, "button")
        .expect("button should be lowered");
    let action = &button.actions[0];
    assert!(action.resolved);
    assert_eq!(action.action.as_deref(), Some("save"));
    assert!(matches!(
        action.handler.effects.as_slice(),
        [RenderActionHandlerEffect::PreventDefault { .. }]
    ));
    assert_eq!(
        action
            .action_span
            .expect("inline TypeScript action should keep an HTML source span")
            .source,
        SourceId::primary()
    );
}

#[test]
fn handler_analysis_tracks_inline_and_referenced_event_context() {
    let assets = CompileAssets::new().with_script(
        Some("app.js".to_owned()),
        "function save(event) { event.preventDefault(); }\nfunction change(value) {}",
    );
    let compiled = Compiler::new().compile_fragment(
        r#"
            <form onsubmit="save(event)">
                <input id="name" name="full_name" oninput="change(event.target.value)">
            </form>
        "#,
        &assets,
    );

    assert!(compiled.diagnostics.is_empty());

    let RenderNode::Element(form) = &compiled.value.plan.nodes[0] else {
        panic!("expected form element");
    };
    let submit = &form.actions[0];
    assert_eq!(submit.action.as_deref(), Some("save"));
    assert_eq!(
        submit.handler.primary_invocation().unwrap().arguments,
        [RenderActionArgument::Event]
    );
    assert!(matches!(
        submit.handler.effects.as_slice(),
        [RenderActionHandlerEffect::PreventDefault { .. }]
    ));

    let RenderNode::Element(input) = &form.children[0] else {
        panic!("expected input element");
    };
    let input_action = &input.actions[0];
    assert_eq!(input_action.action.as_deref(), Some("change"));
    assert_eq!(
        input_action.handler.primary_invocation().unwrap().arguments,
        [RenderActionArgument::ElementValue]
    );

    let submit_plan = compiled
        .value
        .plan
        .state
        .actions
        .iter()
        .find(|action| action.event == "submit")
        .expect("submit action should be planned");
    assert!(
        submit_plan
            .effects
            .contains(&RenderActionEffect::PreventDefault)
    );
    assert!(
        submit_plan
            .effects
            .iter()
            .any(|effect| matches!(effect, RenderActionEffect::SubmitForm(_)))
    );
    assert!(
        submit_plan
            .effects
            .iter()
            .any(|effect| matches!(effect, RenderActionEffect::Invoke { .. }))
    );
}

#[test]
fn state_owner_hint_lowers_to_state_binding() {
    let compiled =
        compile_fragment(r#"<input id="name" data-htmlswap-state-owner="external" value="Ada">"#);

    assert!(compiled.diagnostics.is_empty());
    assert_eq!(compiled.value.state.bindings.len(), 1);
    assert_eq!(
        compiled.value.state.bindings[0].owner,
        RenderStateOwner::External
    );
}

#[test]
fn bundle_config_routes_output_boundaries_without_source_module_hints() {
    let assets = CompileAssets::new()
        .with_stylesheet(Some("app.css".to_owned()), ".card { color: red; }")
        .with_script(Some("app.js".to_owned()), "function save() {}");
    let bundle = BundleConfig::new()
        .with_boundary(
            RouteTarget::Role(UiRole::Button),
            OutputUnitKind::Component,
            OutputBoundary::SeparateModule {
                path_hint: Some("components/card.rs".to_owned()),
            },
        )
        .with_boundary(
            RouteTarget::Style(StyleProperty::Color),
            OutputUnitKind::Style,
            OutputBoundary::SeparateModule {
                path_hint: Some("styles/card.css".to_owned()),
            },
        )
        .with_boundary(
            RouteTarget::event("click"),
            OutputUnitKind::Action,
            OutputBoundary::SeparateModule {
                path_hint: Some("actions/card.rs".to_owned()),
            },
        );
    let compiled = Compiler::try_with_options(CompilerOptions::new().with_bundle(bundle))
        .expect("compiler should build")
        .compile_fragment(
            r#"<button class="card" onclick="save()">Save</button>"#,
            &assets,
        );

    assert!(compiled.diagnostics.is_empty());

    let RenderNode::Element(button) = &compiled.value.plan.nodes[0] else {
        panic!("expected button element");
    };
    assert!(button.source_intent.is_none());

    let bundle = &compiled.value.bundle;
    assert!(bundle.units.iter().any(|unit| {
        unit.kind == OutputUnitKind::Component
            && matches!(
                &unit.boundary,
                OutputBoundary::SeparateModule { path_hint: Some(path) }
                    if path == "components/card.rs"
            )
    }));
    assert!(bundle.units.iter().any(|unit| {
        unit.kind == OutputUnitKind::Style
            && matches!(
                &unit.boundary,
                OutputBoundary::SeparateModule { path_hint: Some(path) }
                    if path == "styles/card.css"
            )
    }));
    assert!(bundle.units.iter().any(|unit| {
        unit.kind == OutputUnitKind::Action
            && matches!(
                &unit.boundary,
                OutputBoundary::SeparateModule { path_hint: Some(path) }
                    if path == "actions/card.rs"
            )
    }));
}

#[test]
fn bundle_config_routes_inherited_source_regions_to_boundaries() {
    let bundle = BundleConfig::new().with_boundary(
        RouteTarget::region("editor.project").expect("region route should parse"),
        OutputUnitKind::Action,
        OutputBoundary::SeparateModule {
            path_hint: Some("actions/editor_project.rs".to_owned()),
        },
    );
    let compiled = Compiler::try_with_options(CompilerOptions::new().with_bundle(bundle))
        .expect("compiler should build")
        .compile_fragment(
            r#"
                <section data-htmlswap-region="editor.project">
                    <button onclick="save()">Save</button>
                </section>
            "#,
            &CompileAssets::new().with_script(Some("app.js".to_owned()), "function save() {}"),
        );

    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );

    let bundle = &compiled.value.bundle;
    assert!(bundle.units.iter().any(|unit| {
        unit.kind == OutputUnitKind::Action
            && matches!(
                &unit.boundary,
                OutputBoundary::SeparateModule { path_hint: Some(path) }
                    if path == "actions/editor_project.rs"
            )
    }));
}

#[test]
fn event_handler_attributes_become_diagnostics() {
    let compiled = compile_fragment(r#"<button onclick="save()">Save</button>"#);

    assert!(
        compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("onclick"))
    );

    let mut emit_context = EmitContext::new();
    let output = TextEmitter
        .emit(&compiled.value, &mut emit_context)
        .expect("text emission should not fail");

    assert_eq!(output, "button <button> @click=save?\n  text \"Save\"\n");
}

#[test]
fn parser_repairs_html_before_lowering() {
    let compiled = compile_fragment("<p>One<p>Two");

    assert_eq!(compiled.value.nodes.len(), 2);

    let mut emit_context = EmitContext::new();
    let output = TextEmitter
        .emit(&compiled.value, &mut emit_context)
        .expect("text emission should not fail");

    assert_eq!(
        output,
        "paragraph <p>\n  text \"One\"\nparagraph <p>\n  text \"Two\"\n"
    );
}

#[test]
fn html_parse_diagnostics_keep_source_spans() {
    let parsed = parse_fragment("<button><button>Save</button>");

    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.span.is_some())
    );
}

#[test]
fn html_parser_attaches_source_spans() {
    let source = r#"<button class="primary" disabled>Save</button>"#;
    let parsed = parse_fragment(source);

    assert!(parsed.diagnostics.is_empty());

    let htmlswap::HtmlNode::Element(element) = &parsed.value.nodes[0] else {
        panic!("expected parsed element");
    };

    let element_span = element.span.expect("element should have a source span");
    assert_eq!(&source[element_span.start..element_span.end], source);

    let class_span = element.attributes[0]
        .span
        .expect("class attribute should have a source span");
    assert_eq!(
        &source[class_span.start..class_span.end],
        r#"class="primary""#
    );

    let disabled_span = element.attributes[1]
        .span
        .expect("boolean attribute should have a source span");
    assert_eq!(&source[disabled_span.start..disabled_span.end], "disabled");

    let htmlswap::HtmlNode::Text(text) = &element.children[0] else {
        panic!("expected parsed text");
    };
    let text_span = text.span.expect("text should have a source span");
    assert_eq!(&source[text_span.start..text_span.end], "Save");
}

#[test]
fn css_selectors_are_mapped_onto_elements() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        "button.primary[data-kind=\"save\"] { color: red; }\n.toolbar > .child { display: flex; }\n#save { gap: 8px; }",
    );
    let compiled = compile_fragment_with_assets(
        r#"<div class="toolbar"><button id="save" class="primary child" data-kind="save" disabled style="padding: 4px">Save</button></div>"#,
        &assets,
    );

    assert!(compiled.diagnostics.is_empty());

    let mut emit_context = EmitContext::new();
    let output = TextEmitter
        .emit(&compiled.value, &mut emit_context)
        .expect("text emission should not fail");

    assert_eq!(
        output,
        "container <div> .toolbar\n  button <button> .primary.child [id=\"save\" data-kind=\"save\" disabled] {color: red; display: flex; gap: 8px; padding: 4px}\n    text \"Save\"\n"
    );
}

#[test]
fn conditional_css_rules_lower_to_style_variants() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        ".primary:hover { color: red; }\n.primary::before { color: green; }\n@media (min-width: 600px) { .primary { color: blue; } }",
    );
    let compiled =
        compile_fragment_with_assets(r#"<button class="primary">Save</button>"#, &assets);

    assert!(compiled.diagnostics.is_empty());

    let RenderNode::Element(button) = &compiled.value.nodes[0] else {
        panic!("expected button element");
    };

    assert!(button.styles.is_empty());
    assert_eq!(button.style_variants.len(), 2);
    assert_eq!(button.pseudo_elements.len(), 1);
    assert_eq!(button.pseudo_elements[0].kind, "before");
    assert_eq!(
        button.pseudo_elements[0].styles[0].property,
        StyleProperty::Color
    );
    assert_eq!(button.pseudo_elements[0].styles[0].value.as_str(), "green");
    assert!(
        button
            .style_variants
            .iter()
            .any(|variant| variant.conditions
                == [RenderStyleCondition::PseudoClass("hover".into())]
                && variant.declarations[0].value.as_str() == "red")
    );
    assert!(button.style_variants.iter().any(|variant| matches!(
        variant.conditions.as_slice(),
        [RenderStyleCondition::Media(query)] if query.contains("600px")
    )
        && variant.declarations[0].property.as_str() == "color"));
    assert!(
        button
            .style_variants
            .iter()
            .all(|variant| variant.span.is_some())
    );
}

#[test]
fn pseudo_element_content_lowers_to_synthetic_children() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        r#".badge::before { content: "New"; color: red; }"#,
    );
    let compiled = compile_fragment_with_assets(r#"<span class="badge">Item</span>"#, &assets);

    assert!(compiled.diagnostics.is_empty());

    let RenderNode::Element(badge) = &compiled.value.nodes[0] else {
        panic!("expected badge element");
    };

    assert!(badge.style_variants.is_empty());
    assert_eq!(badge.pseudo_elements.len(), 1);
    let pseudo = &badge.pseudo_elements[0];
    assert_eq!(pseudo.kind, "before");
    assert_eq!(pseudo.styles[0].property, StyleProperty::Color);
    let RenderNode::Text(text) = &pseudo.children[0] else {
        panic!("expected generated text node");
    };
    assert_eq!(text.value, "New");
}

#[test]
fn css_cascade_respects_specificity_and_important() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        ".primary { color: red !important; }\n#save { color: blue; }\nbutton { color: green; }",
    );
    let compiled = compile_fragment_with_assets(
        r#"<button id="save" class="primary">Save</button>"#,
        &assets,
    );

    assert!(compiled.diagnostics.is_empty());

    let mut emit_context = EmitContext::new();
    let output = TextEmitter
        .emit(&compiled.value, &mut emit_context)
        .expect("text emission should not fail");

    assert_eq!(
        output,
        "button <button> .primary [id=\"save\"] {color: red !important}\n  text \"Save\"\n"
    );
}

#[test]
fn css_declarations_keep_external_source_spans() {
    let assets = CompileAssets::new()
        .with_stylesheet(Some("app.css".to_owned()), "\n.card { color: red; }\n");
    let compiled = Compiler::new().compile_fragment(r#"<div class="card"></div>"#, &assets);

    assert!(compiled.diagnostics.is_empty());

    let RenderNode::Element(root) = &compiled.value.plan.nodes[0] else {
        panic!("expected root element");
    };
    let span = root.styles[0]
        .span
        .expect("style declaration should have a source span");
    let source_file = compiled
        .value
        .sources
        .file(span.source)
        .expect("style source should be registered");

    assert_eq!(source_file.kind(), SourceKind::Css);
    assert_eq!(source_file.name(), Some("app.css"));
    let snippet = compiled
        .value
        .sources
        .source_text(span)
        .expect("style span should resolve");
    assert!(snippet.starts_with(".card"));
    assert!(snippet.contains("color: red"));
}

#[test]
fn source_map_keeps_arcstr_backed_input_buffers_shared() {
    let html = ArcStr::from(r#"<div class="card"></div>"#);
    let css = ArcStr::from(".card { color: red; }");
    let assets = CompileAssets::new().with_stylesheet(Some("app.css".to_owned()), css.clone());
    let compiled = Compiler::new().compile_fragment(html.clone(), &assets);

    assert!(compiled.diagnostics.is_empty());

    let html_source = compiled
        .value
        .sources
        .file(SourceId::primary())
        .expect("HTML source should be registered");
    let css_source = compiled
        .value
        .sources
        .files()
        .iter()
        .find(|source| source.kind() == SourceKind::Css)
        .expect("CSS source should be registered");

    assert!(ArcStr::ptr_eq(html_source.buffer(), &html));
    assert!(ArcStr::ptr_eq(css_source.buffer(), &css));
}

#[test]
fn source_map_returns_arcstr_backed_substrings_for_spans() {
    let source = ArcStr::from(r#"<button>Save</button>"#);
    let compiled = Compiler::new().compile_fragment(source.clone(), &CompileAssets::new());

    assert!(compiled.diagnostics.is_empty());

    let RenderNode::Element(button) = &compiled.value.plan.nodes[0] else {
        panic!("expected button element");
    };
    let RenderNode::Text(text) = &button.children[0] else {
        panic!("expected button text");
    };
    let text_span = text.span.expect("text should have a source span");
    let text_source = compiled
        .value
        .sources
        .source_substr(text_span)
        .expect("text span should resolve to a shared substring");

    assert_eq!(text_source.as_str(), "Save");
    assert!(ArcStr::ptr_eq(text_source.parent(), &source));
}

#[test]
fn inline_style_elements_are_indexed() {
    let compiled = Compiler::new().compile_fragment(
        r#"<style>.card { color: red; }</style><div class="card">Card</div>"#,
        &CompileAssets::new(),
    );

    assert!(compiled.diagnostics.is_empty());

    let RenderNode::Element(root) = &compiled.value.plan.nodes[0] else {
        panic!("expected root element");
    };

    assert_eq!(root.styles[0].property.as_str(), "color");
    assert_eq!(root.styles[0].value.as_str(), "red");
    let span = root.styles[0]
        .span
        .expect("inline stylesheet declaration should keep HTML span");
    assert_eq!(
        compiled
            .value
            .sources
            .source_text(span)
            .expect("inline stylesheet span should resolve"),
        ".card { color: red; }"
    );
}

#[test]
fn compiler_combines_html_css_and_javascript_comments_as_annotations() {
    let assets = CompileAssets::new()
        .with_stylesheet(
            Some("app.css".to_owned()),
            "/* external css */\n.card { color: red; }",
        )
        .with_script(
            Some("app.js".to_owned()),
            "// external js\nfunction save() {}",
        );
    let source = r#"<!-- html note --><style>/* inline css */ .card { color: blue; }</style><script>// inline js
const inlineSave = () => {};</script><button class="card" onclick="save()">Save</button>"#;
    let compiled = Compiler::new().compile_fragment(source, &assets);

    assert!(compiled.diagnostics.is_empty());

    let annotations = &compiled.value.plan.annotations;
    let labels = annotations
        .iter()
        .map(|annotation| (annotation.kind.clone(), annotation.value.trim().to_owned()))
        .collect::<Vec<_>>();

    assert_eq!(
        labels,
        [
            (RenderAnnotationKind::HtmlComment, "html note".to_owned()),
            (RenderAnnotationKind::CssComment, "inline css".to_owned()),
            (
                RenderAnnotationKind::JavaScriptLineComment,
                "inline js".to_owned()
            ),
            (RenderAnnotationKind::CssComment, "external css".to_owned()),
            (
                RenderAnnotationKind::JavaScriptLineComment,
                "external js".to_owned()
            ),
        ]
    );

    let snippets = annotations
        .iter()
        .map(|annotation| {
            compiled
                .value
                .sources
                .source_text(annotation.span.expect("annotation should have a span"))
                .expect("annotation span should resolve")
                .to_owned()
        })
        .collect::<Vec<_>>();

    assert_eq!(
        snippets,
        [
            "<!-- html note -->",
            "/* inline css */",
            "// inline js",
            "/* external css */",
            "// external js",
        ]
    );
}

#[test]
fn style_attributes_use_css_parser() {
    let compiled = compile_fragment(
        r#"<div style="color: red !important; background-image: url(&quot;a;b.png&quot;)"></div>"#,
    );

    assert!(compiled.diagnostics.is_empty());

    let RenderNode::Element(root) = &compiled.value.nodes[0] else {
        panic!("expected root element");
    };

    assert_eq!(root.styles.len(), 2);
    let color = root
        .styles
        .iter()
        .find(|style| style.property.as_str() == "color")
        .expect("color declaration should exist");
    assert_eq!(color.value.as_str(), "red");
    assert!(color.important);
    let background = root
        .styles
        .iter()
        .find(|style| style.property.as_str() == "background-image")
        .expect("background-image declaration should exist");
    assert_eq!(background.value.as_str(), "url(\"a;b.png\")");
}

#[test]
fn unresolved_css_values_stay_raw() {
    let compiled = compile_fragment(r#"<div style="width: calc(100% - 8px); --brand: red"></div>"#);

    assert!(compiled.diagnostics.is_empty());

    let RenderNode::Element(root) = &compiled.value.nodes[0] else {
        panic!("expected root element");
    };

    assert!(matches!(root.styles[0].value, StyleValue::Raw(_)));
    assert_eq!(root.styles[0].value.as_str(), "calc(100% - 8px)");
    assert!(matches!(root.styles[1].value, StyleValue::Raw(_)));
    assert_eq!(root.styles[1].property.as_str(), "--brand");
}

#[test]
fn css_custom_properties_lower_to_theme_plan_and_token_references() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        r#"
            :root {
                --background: #ffffff;
                --foreground: #111111;
                --radius: 8px;
            }
            .dark { --background: #111111; }
            @media (prefers-color-scheme: dark) {
                :root { --background: #000000; }
            }
            .card {
                background-color: var(--background);
                color: var(--foreground, #222222);
                border: 1px solid var(--border, #dddddd);
                border-radius: var(--radius);
            }
        "#,
    );
    let compiled = Compiler::new().compile_fragment(r#"<div class="card"></div>"#, &assets);

    assert!(compiled.diagnostics.is_empty());

    let plan = &compiled.value.plan;
    let background = plan
        .theme
        .token("--background")
        .expect("background token should be collected");
    assert_eq!(background.kind, ThemeTokenKind::Color);
    assert!(background.referenced);
    assert!(
        background
            .values
            .iter()
            .any(|value| value.scope == RenderThemeScope::Root)
    );
    assert!(
        background
            .values
            .iter()
            .any(|value| { value.scope == RenderThemeScope::Selector(".dark".into()) })
    );
    assert!(background.values.iter().any(|value| matches!(
        &value.scope,
        RenderThemeScope::Conditional {
            selector: None,
            conditions
        } if conditions
            .iter()
            .any(|condition| matches!(condition, RenderStyleCondition::Media(value) if value.contains("prefers-color-scheme")))
    )));

    let foreground = plan
        .theme
        .token("--foreground")
        .expect("foreground token should be collected");
    assert!(foreground.referenced);
    let radius = plan
        .theme
        .token("--radius")
        .expect("radius token should be collected");
    assert_eq!(radius.kind, ThemeTokenKind::Length);
    assert!(radius.referenced);
    let border = plan
        .theme
        .token("--border")
        .expect("border token should be tracked even when only referenced in a shorthand");
    assert!(border.referenced);
    assert!(border.values.is_empty());

    let RenderNode::Element(root) = &plan.nodes[0] else {
        panic!("expected root element");
    };
    assert!(root.styles.iter().any(|style| {
        style.property == StyleProperty::BackgroundColor
            && matches!(style.value, StyleValue::Token(_))
    }));
    assert!(root.styles.iter().any(|style| {
        style.property == StyleProperty::Border
            && style
                .value
                .tokens_with_span(style.span)
                .iter()
                .any(|token| token.name == "--border")
    }));
    assert!(
        compiled
            .value
            .bundle
            .units
            .iter()
            .any(|unit| { unit.kind == OutputUnitKind::Style && !unit.source.sources.is_empty() })
    );
}

#[test]
fn raw_escape_hatch_stops_lowering_subtree() {
    let source = r#"<div data-htmlswap-raw><button onclick="missing()">Raw</button></div>"#;
    let compiled = Compiler::new().compile_fragment(source, &CompileAssets::new());

    assert!(compiled.diagnostics.is_empty());

    let RenderNode::Raw(raw) = &compiled.value.plan.nodes[0] else {
        panic!("expected raw node");
    };
    assert_eq!(raw.html, source);

    let mut emit_context = EmitContext::new();
    let output = TextEmitter
        .emit(&compiled.value.plan, &mut emit_context)
        .expect("text emission should not fail");

    assert_eq!(
        output,
        "raw \"<div data-htmlswap-raw><button onclick=\\\"missing()\\\">Raw</button></div>\"\n"
    );
}

#[test]
fn foreign_namespace_elements_become_raw_nodes() {
    let source = r#"<svg viewBox="0 0 10 10"><path d="M0 0h10v10z"></path></svg>"#;
    let compiled = compile_fragment(source);

    assert!(compiled.diagnostics.is_empty());

    let RenderNode::Raw(raw) = &compiled.value.nodes[0] else {
        panic!("expected raw node");
    };
    assert_eq!(raw.html, source);
}

#[test]
fn external_script_actions_resolve_event_handlers() {
    let assets = CompileAssets::new().with_script(Some("app.js".to_owned()), "function save() {}");
    let compiled =
        compile_fragment_with_assets(r#"<button onclick="save()">Save</button>"#, &assets);

    assert!(compiled.diagnostics.is_empty());

    let mut emit_context = EmitContext::new();
    let output = TextEmitter
        .emit(&compiled.value, &mut emit_context)
        .expect("text emission should not fail");

    assert_eq!(output, "button <button> @click=save\n  text \"Save\"\n");
}

#[test]
fn inline_script_actions_resolve_event_handlers() {
    let compiled = compile_fragment_with_assets(
        r#"<script>const save = () => {};</script><button onclick="save()">Save</button>"#,
        &CompileAssets::new(),
    );

    assert!(compiled.diagnostics.is_empty());

    let mut emit_context = EmitContext::new();
    let output = TextEmitter
        .emit(&compiled.value, &mut emit_context)
        .expect("text emission should not fail");

    assert_eq!(output, "button <button> @click=save\n  text \"Save\"\n");
}

#[test]
fn inline_script_actions_resolve_to_html_source_spans() {
    let source = r#"<section><script> const save = () => {}; </script></section>"#;
    let compiled = Compiler::new().compile_fragment(
        format!(r#"{source}<button onclick="save()">Save</button>"#),
        &CompileAssets::new(),
    );
    assert!(compiled.diagnostics.is_empty());

    let RenderNode::Element(button) = &compiled.value.plan.nodes[1] else {
        panic!("expected button element");
    };
    let action_span = button.actions[0]
        .action_span
        .expect("inline action should have an HTML source span");
    assert_eq!(
        compiled
            .value
            .sources
            .source_text(action_span)
            .expect("action span should resolve"),
        "save = () => {}"
    );
}

#[test]
fn external_script_action_definitions_keep_source_spans() {
    let assets =
        CompileAssets::new().with_script(Some("app.js".to_owned()), "const save = () => {};");
    let compiled =
        Compiler::new().compile_fragment(r#"<button onclick="save()">Save</button>"#, &assets);

    assert!(compiled.diagnostics.is_empty());
    let RenderNode::Element(button) = &compiled.value.plan.nodes[0] else {
        panic!("expected button element");
    };
    let span = button.actions[0]
        .action_span
        .expect("action definition should have a source span");

    let source_file = compiled
        .value
        .sources
        .file(span.source)
        .expect("script source should be registered");
    assert_eq!(source_file.kind(), SourceKind::JavaScript);
    assert_eq!(source_file.name(), Some("app.js"));
    assert_eq!(
        compiled
            .value
            .sources
            .source_text(span)
            .expect("action span should resolve"),
        "save = () => {}"
    );
}

#[test]
fn oxc_parse_diagnostics_keep_source_spans() {
    let assets = CompileAssets::new().with_script(Some("app.js".to_owned()), "const =");
    let compiled = Compiler::new().compile_fragment("", &assets);

    assert!(!compiled.diagnostics.is_empty());
    assert!(
        compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.span.is_some_and(|span| compiled
                .value
                .sources
                .file(span.source)
                .is_some_and(|source| source.kind() == SourceKind::JavaScript)))
    );
}
