use htmlswap::{
    CompileAssets, Compiler, Frontend, RoundTripFeatureKind, RoundTripMismatchKind,
    RoundTripOptions, compare_roundtrip_plans,
};

#[test]
fn roundtrip_checker_treats_click_and_doubleclick_as_distinct_events() {
    let compiler = Compiler::new().with_frontend(Frontend::dc());
    let assets = CompileAssets::new();
    let expected =
        compiler.compile_fragment(r#"<div onDoubleClick="{{ n.onOpen }}">Open</div>"#, &assets);
    let actual = compiler.compile_fragment(r#"<div onclick="{{ n.onOpen }}">Open</div>"#, &assets);
    assert!(expected.diagnostics.is_empty());
    assert!(actual.diagnostics.is_empty());

    let comparison = compare_roundtrip_plans(
        &expected.value.plan,
        &actual.value.plan,
        RoundTripOptions::new(),
    );

    assert!(!comparison.is_match());
    assert!(
        comparison.mismatches().iter().any(|mismatch| {
            mismatch.feature_kind == RoundTripFeatureKind::Action
                && mismatch.kind == RoundTripMismatchKind::Missing
                && mismatch.feature.contains("event=doubleclick")
        }),
        "{comparison}"
    );
    assert!(
        comparison.mismatches().iter().any(|mismatch| {
            mismatch.feature_kind == RoundTripFeatureKind::Action
                && mismatch.kind == RoundTripMismatchKind::Extra
                && mismatch.feature.contains("event=click")
        }),
        "{comparison}"
    );
}

#[test]
fn roundtrip_checker_accepts_matching_action_kinds() {
    let compiler = Compiler::new().with_frontend(Frontend::dc());
    let assets = CompileAssets::new();
    let source = r#"<button onclick="{{ save }}" onDoubleClick="{{ inspect }}" style-hover="background: red">Save</button>"#;
    let expected = compiler.compile_fragment(source, &assets);
    let actual = compiler.compile_fragment(source, &assets);
    assert!(expected.diagnostics.is_empty());
    assert!(actual.diagnostics.is_empty());

    let comparison = compare_roundtrip_plans(
        &expected.value.plan,
        &actual.value.plan,
        RoundTripOptions::new(),
    );

    assert!(comparison.is_match(), "{comparison}");
}

#[test]
fn roundtrip_checker_normalizes_equivalent_css_spellings() {
    let compiler = Compiler::new();
    let assets = CompileAssets::new();
    let expected = compiler.compile_fragment(
        r#"<div style="padding: 4px 8px; margin: 1px 2px 3px 4px; border: 1px solid #2C313A; font-weight: 600; font-family: 'IBM Plex Mono', monospace; border-radius: 50%; box-sizing: border-box; background: none"></div>"#,
        &assets,
    );
    let actual = compiler.compile_fragment(
        r#"<div style="padding-top: 4px; padding-right: 8px; padding-bottom: 4px; padding-left: 8px; margin-top: 1px; margin-right: 2px; margin-bottom: 3px; margin-left: 4px; border-width: 1px; border-color: #2c313a; font-weight: semibold; font-family: IBM Plex Mono; border-radius: 9999px"></div>"#,
        &assets,
    );
    assert!(expected.diagnostics.is_empty());
    assert!(actual.diagnostics.is_empty());

    let comparison = compare_roundtrip_plans(
        &expected.value.plan,
        &actual.value.plan,
        RoundTripOptions::new(),
    );

    assert!(comparison.is_match(), "{comparison}");
}

#[test]
fn roundtrip_checker_keeps_non_solid_border_style_mismatches() {
    let compiler = Compiler::new();
    let assets = CompileAssets::new();
    let expected =
        compiler.compile_fragment(r#"<div style="border: 1px dashed #2c313a"></div>"#, &assets);
    let actual = compiler.compile_fragment(
        r#"<div style="border-width: 1px; border-color: #2c313a"></div>"#,
        &assets,
    );
    assert!(expected.diagnostics.is_empty());
    assert!(actual.diagnostics.is_empty());

    let comparison = compare_roundtrip_plans(
        &expected.value.plan,
        &actual.value.plan,
        RoundTripOptions::new(),
    );

    assert!(!comparison.is_match());
    assert!(
        comparison.mismatches().iter().any(|mismatch| {
            mismatch.feature_kind == RoundTripFeatureKind::Style
                && mismatch.kind == RoundTripMismatchKind::Missing
                && mismatch.feature.contains("border-top-style=dashed")
        }),
        "{comparison}"
    );
}

#[test]
fn roundtrip_checker_normalizes_gpui_flex_and_relative_line_height() {
    let compiler = Compiler::new();
    let assets = CompileAssets::new();
    let expected =
        compiler.compile_fragment(r#"<div style="flex: 0 0 212px; line-height: 1.4; overflow-y: auto; aspect-ratio: 1/1"></div>"#, &assets);
    let actual =
        compiler.compile_fragment(r#"<div style="flex-grow: 0; flex-shrink: 0; flex-basis: 212px; line-height: 140%; overflow-y: scroll; aspect-ratio: 1"></div>"#, &assets);
    assert!(expected.diagnostics.is_empty());
    assert!(actual.diagnostics.is_empty());

    let comparison = compare_roundtrip_plans(
        &expected.value.plan,
        &actual.value.plan,
        RoundTripOptions::new(),
    );

    assert!(comparison.is_match(), "{comparison}");
}

#[test]
fn roundtrip_checker_normalizes_gpui_inset_viewport_and_default_noops() {
    let compiler = Compiler::new();
    let assets = CompileAssets::new();
    let expected = compiler.compile_fragment(
        r#"<div style="inset: 0; width: 100vw; height: 100vh; align-items: stretch; align-self: stretch; flex-shrink: 1; user-select: none; vertical-align: middle; pointer-events: auto"></div>"#,
        &assets,
    );
    let actual = compiler.compile_fragment(
        r#"<div style="top: 0px; right: 0px; bottom: 0px; left: 0px; width: 100%; height: 100%"></div>"#,
        &assets,
    );
    assert!(expected.diagnostics.is_empty());
    assert!(actual.diagnostics.is_empty());

    let comparison = compare_roundtrip_plans(
        &expected.value.plan,
        &actual.value.plan,
        RoundTripOptions::new(),
    );

    assert!(comparison.is_match(), "{comparison}");
}

#[test]
fn roundtrip_checker_normalizes_fixed_grid_template_spellings() {
    let compiler = Compiler::new();
    let assets = CompileAssets::new();
    let expected = compiler.compile_fragment(
        r#"<div style="grid-template-columns: 1fr 1fr"></div>"#,
        &assets,
    );
    let actual = compiler.compile_fragment(
        r#"<div style="grid-template-columns: repeat(2, 1fr)"></div>"#,
        &assets,
    );
    assert!(expected.diagnostics.is_empty());
    assert!(actual.diagnostics.is_empty());

    let comparison = compare_roundtrip_plans(
        &expected.value.plan,
        &actual.value.plan,
        RoundTripOptions::new(),
    );

    assert!(comparison.is_match(), "{comparison}");
}

#[test]
fn roundtrip_checker_rejects_swapped_sibling_order() {
    let compiler = Compiler::new();
    let assets = CompileAssets::new();
    let expected = compiler.compile_fragment(
        r#"<div><button id="save">Save</button><button id="cancel">Cancel</button></div>"#,
        &assets,
    );
    let actual = compiler.compile_fragment(
        r#"<div><button id="cancel">Cancel</button><button id="save">Save</button></div>"#,
        &assets,
    );
    assert!(expected.diagnostics.is_empty());
    assert!(actual.diagnostics.is_empty());

    let comparison = compare_roundtrip_plans(
        &expected.value.plan,
        &actual.value.plan,
        RoundTripOptions::new(),
    );

    assert!(!comparison.is_match());
    assert!(
        comparison
            .mismatches()
            .iter()
            .any(|mismatch| mismatch.feature.contains("$/div[0]/button[0]")),
        "{comparison}"
    );
}
