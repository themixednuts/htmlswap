//! Computed values on compiled documents: the custom-property and color
//! cascade, typed styles, units and media queries.

use htmlswap::computed::{
    BorderStyle, ColorScheme, ComputedScope, ComputedStyle, Display, FontFamily, GridLine,
    LengthAuto, LengthPercentage, LineHeight, MediaEnvironment, Rgba, Size, Track, TrackBreadth,
    TrackSize, Unsupported,
};
use htmlswap::{RenderElement, RenderNode, RenderPlan, StyleDeclaration, compile_fragment};

fn close(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() < 1e-3
}

fn compile(source: &str) -> RenderPlan {
    let compiled = compile_fragment(source);
    assert!(
        !compiled.diagnostics.has_errors(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );
    compiled.value
}

/// Elements from the root down to the one with `id`.
fn ancestry<'a>(nodes: &'a [RenderNode], id: &str) -> Option<Vec<&'a RenderElement>> {
    for node in nodes {
        let RenderNode::Element(element) = node else {
            continue;
        };
        let matches = element
            .attributes
            .iter()
            .any(|attribute| attribute.name == "id" && attribute.value == id);
        if matches {
            return Some(vec![element.as_ref()]);
        }
        if let Some(mut path) = ancestry(&element.children, id) {
            path.insert(0, element.as_ref());
            return Some(path);
        }
    }
    None
}

fn declarations(element: &RenderElement) -> impl Iterator<Item = &StyleDeclaration> + Clone {
    element
        .stylesheet_declarations
        .iter()
        .chain(&element.styles)
}

/// The computed style of `id`, cascading scopes down its ancestry.
fn computed(
    plan: &RenderPlan,
    id: &str,
    environment: &MediaEnvironment,
) -> (ComputedStyle, ComputedScope, Vec<(String, Unsupported)>) {
    let path = ancestry(&plan.nodes, id).unwrap_or_else(|| panic!("no #{id}"));
    let mut scope = ComputedScope::root(environment).document_root(plan.root.styles.iter());
    let mut parent = scope.clone();
    for element in &path {
        parent = scope.clone();
        scope = scope.child(declarations(element));
    }
    let element = path.last().copied().unwrap_or_else(|| panic!("no #{id}"));
    let context = scope.style_context(&parent);
    let resolved = declarations(element)
        .filter_map(|declaration| scope.resolve(declaration).map(std::borrow::Cow::into_owned))
        .collect::<Vec<_>>();
    let mut unsupported = Vec::new();
    let style = ComputedStyle::compute(&resolved, &context, |declaration, reason| {
        unsupported.push((declaration.property.as_str().to_owned(), reason));
    });
    (style, scope, unsupported)
}

#[test]
fn custom_properties_light_dark_and_current_color_cascade() {
    let plan = compile(
        r#"
        <style>
          main { --brand: oklch(55% 0.2 260); --gap: 0.5rem; color-scheme: light dark;
                 color: light-dark(#111, #eee); }
          .card { background-color: var(--brand); padding: var(--gap) calc(var(--gap) * 2);
                  border: 2px solid currentColor; }
          .card .title { color: var(--missing, light-dark(navy, skyblue)); }
          #reset { color: red; padding: 4px; }
          main #reset { color: initial; padding: unset; }
        </style>
        <main><div class="card" id="card"><h2 class="title" id="title">Hi</h2></div><p id="reset">Reset</p></main>
        "#,
    );
    let dark = MediaEnvironment {
        color_scheme: ColorScheme::Dark,
        ..MediaEnvironment::default()
    };
    let (card, scope, unsupported) = computed(&plan, "card", &dark);
    assert!(unsupported.is_empty(), "{unsupported:?}");
    assert_eq!(scope.scheme(), ColorScheme::Dark);
    assert_eq!(
        card.border_color.top,
        Some(Rgba::opaque(0xeeeeee)),
        "currentColor"
    );
    assert_eq!(card.border_style.left, Some(BorderStyle::Solid));
    assert_eq!(card.padding.top, Some(LengthPercentage::px(8.0)));
    assert_eq!(card.padding.right, Some(LengthPercentage::px(16.0)));
    assert!(card.background_color.is_some_and(|color| color.b > color.r));

    let (title, ..) = computed(&plan, "title", &dark);
    assert_eq!(title.color, Some(Rgba::opaque(0x87ceeb)));
    let (title, ..) = computed(&plan, "title", &MediaEnvironment::default());
    assert_eq!(title.color, Some(Rgba::opaque(0x000080)));

    let (reset, _, unsupported) = computed(&plan, "reset", &MediaEnvironment::default());
    assert!(unsupported.is_empty(), "{unsupported:?}");
    assert_eq!(
        reset.color, None,
        "a CSS-wide keyword clears earlier declarations"
    );
    assert_eq!(reset.padding.top, None);
}

#[test]
fn lengths_resolve_units_and_math() {
    let plan = compile(
        r#"
        <style>
          section { font-size: 1.5rem; }
          #box { width: calc(100% - 2em); height: 50vh; min-width: min(10rem, 300px);
                 max-width: none; margin: auto 1em; line-height: 150%; font-size: larger;
                 border-radius: 4px; inset: 0 auto; }
        </style>
        <section><div id="box">Box</div></section>
        "#,
    );
    let (style, scope, unsupported) = computed(&plan, "box", &MediaEnvironment::default());
    assert!(unsupported.is_empty(), "{unsupported:?}");
    // 1.5rem of the 16px root, then `larger` (×1.2).
    assert!(
        (scope.font_size() - 28.8).abs() < 1e-3,
        "{}",
        scope.font_size()
    );
    assert_eq!(style.font_size, Some(scope.font_size()));
    let Some(Size::Length(width)) = style.width else {
        panic!("width: {:?}", style.width);
    };
    assert!((width.px + 57.6).abs() < 1e-3 && (width.fraction - 1.0).abs() < 1e-6);
    assert_eq!(
        style.height,
        Some(Size::Length(LengthPercentage::px(384.0)))
    );
    assert_eq!(
        style.min_width,
        Some(Size::Length(LengthPercentage::px(160.0)))
    );
    assert_eq!(style.max_width, Some(Size::None));
    assert_eq!(style.margin.top, Some(LengthAuto::Auto));
    assert!(
        matches!(style.margin.right, Some(LengthAuto::Length(margin)) if close(margin.px, 28.8))
    );
    assert!(matches!(style.line_height, Some(LineHeight::Px(height)) if close(height, 43.2)));
    assert_eq!(
        style.border_radius.top_left,
        Some(LengthPercentage::px(4.0))
    );
}

#[test]
fn shorthands_grid_and_text_are_typed() {
    let plan = compile(
        r#"
        <style>
          #grid { display: grid; grid-template-columns: 200px repeat(auto-fill, minmax(100px, 1fr));
                  grid-auto-flow: column dense; gap: 8px 4%; }
          #cell { grid-column: 2 / span 3; font: italic 700 14px/1.4 "Open Sans", system-ui;
                  text-decoration: underline wavy red 2px; box-shadow: 0 1px 2px rgb(0 0 0 / 50%), inset 0 0 1px blue;
                  -webkit-line-clamp: 3; text-wrap: balance; letter-spacing: 0.1em; }
          #named { grid-template-columns: [start] 1fr [end]; background: linear-gradient(red, blue); }
        </style>
        <div id="grid"><p id="cell">Cell</p><p id="named">Named</p></div>
        "#,
    );
    let environment = MediaEnvironment::default();
    let (grid, _, unsupported) = computed(&plan, "grid", &environment);
    assert!(unsupported.is_empty(), "{unsupported:?}");
    assert_eq!(grid.display, Some(Display::Grid));
    assert_eq!(grid.row_gap, Some(LengthPercentage::px(8.0)));
    assert_eq!(grid.column_gap, Some(LengthPercentage::fraction(0.04)));
    let columns = grid.grid_template_columns.unwrap_or_default();
    assert_eq!(
        columns[0],
        Track::Size(TrackSize::Breadth(TrackBreadth::Length(
            LengthPercentage::px(200.0)
        )))
    );
    assert!(matches!(&columns[1], Track::Repeat { tracks, .. }
        if tracks == &[TrackSize::MinMax(TrackBreadth::Length(LengthPercentage::px(100.0)), TrackBreadth::Flex(1.0))]));
    assert!(
        grid.grid_auto_flow
            .is_some_and(|flow| flow.column && flow.dense)
    );

    let (cell, _, unsupported) = computed(&plan, "cell", &environment);
    assert!(unsupported.is_empty(), "{unsupported:?}");
    assert_eq!(cell.grid_column_start, Some(GridLine::Line(2)));
    assert_eq!(cell.grid_column_end, Some(GridLine::Span(3)));
    assert_eq!(cell.font_weight, Some(700.0));
    assert_eq!(cell.font_size, Some(14.0));
    assert_eq!(cell.line_height, Some(LineHeight::Number(1.4)));
    assert_eq!(
        cell.font_family,
        Some(vec![
            FontFamily::Named("Open Sans".into()),
            FontFamily::SystemUi
        ])
    );
    assert!(
        cell.text_decoration_line
            .is_some_and(|lines| lines.underline)
    );
    assert_eq!(cell.text_decoration_color, Some(Rgba::opaque(0xff0000)));
    let shadows = cell.box_shadow.unwrap_or_default();
    assert_eq!(shadows.len(), 2);
    assert_eq!(shadows[0].color.a, 128);
    assert!(shadows[1].inset);
    assert_eq!(cell.line_clamp, Some(Some(3)));
    assert!(
        cell.letter_spacing
            .is_some_and(|spacing| (spacing - 1.4).abs() < 1e-4)
    );

    let (_, _, unsupported) = computed(&plan, "named", &environment);
    let properties = unsupported
        .iter()
        .map(|(property, _)| property.as_str())
        .collect::<Vec<_>>();
    assert!(
        properties.contains(&"grid-template-columns"),
        "{unsupported:?}"
    );
    assert!(properties.contains(&"background"), "{unsupported:?}");
}
