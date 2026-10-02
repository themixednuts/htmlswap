//! Stylesheet motion and the CSS features it depends on: nesting,
//! `@starting-style`, `@keyframes`, `@view-transition` and the
//! `::view-transition-*` pseudo-elements.

use htmlswap::motion::{Easing, Transition, animations, transitions};
use htmlswap::{
    RenderElement, RenderNode, RenderPlan, RenderStyleCondition, StyleProperty, ViewTransitionName,
    ViewTransitionPart, compile_fragment,
};

fn compile(source: &str) -> RenderPlan {
    let compiled = compile_fragment(source);
    assert!(
        !compiled.diagnostics.has_errors(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );
    compiled.value
}

fn find<'a>(nodes: &'a [RenderNode], id: &str) -> Option<&'a RenderElement> {
    nodes.iter().find_map(|node| {
        let RenderNode::Element(element) = node else {
            return None;
        };
        let matches = element
            .attributes
            .iter()
            .any(|attribute| attribute.name == "id" && attribute.value == id);
        if matches {
            Some(element.as_ref())
        } else {
            find(&element.children, id)
        }
    })
}

fn base_value<'a>(element: &'a RenderElement, property: &StyleProperty) -> Option<&'a str> {
    element
        .stylesheet_declarations
        .iter()
        .chain(&element.styles)
        .filter(|declaration| declaration.property == *property)
        .map(|declaration| declaration.value.as_str())
        .next_back()
}

fn variant_values<'a>(
    element: &'a RenderElement,
    condition: &RenderStyleCondition,
    property: &StyleProperty,
) -> Vec<&'a str> {
    element
        .style_variants
        .iter()
        .filter(|variant| variant.conditions.contains(condition))
        .flat_map(|variant| &variant.declarations)
        .filter(|declaration| declaration.property == *property)
        .map(|declaration| declaration.value.as_str())
        .collect()
}

#[test]
fn nested_rules_resolve_against_their_parent() {
    let plan = compile(
        r#"
        <style>
          .card {
            color: #111111;
            &:hover { color: #222222; }
            .title { font-size: 20px; }
            & > .badge { font-size: 10px; }
            opacity: 0.5;
          }
        </style>
        <div class="card" id="card">
          <h2 class="title" id="title">Title</h2>
          <span class="badge" id="badge">New</span>
        </div>
        <h2 class="title" id="outside">Not in a card</h2>
        "#,
    );
    let card = find(&plan.nodes, "card").expect("card");
    assert_eq!(base_value(card, &StyleProperty::Color), Some("#111"));
    assert_eq!(
        base_value(card, &StyleProperty::Opacity),
        Some(".5"),
        "declarations after nested rules still apply to the parent"
    );
    assert_eq!(
        variant_values(
            card,
            &RenderStyleCondition::PseudoClass("hover".into()),
            &StyleProperty::Color
        ),
        ["#222"]
    );
    let title = find(&plan.nodes, "title").expect("title");
    assert_eq!(base_value(title, &StyleProperty::FontSize), Some("20px"));
    let badge = find(&plan.nodes, "badge").expect("badge");
    assert_eq!(base_value(badge, &StyleProperty::FontSize), Some("10px"));
    let outside = find(&plan.nodes, "outside").expect("outside");
    assert_eq!(
        base_value(outside, &StyleProperty::FontSize),
        None,
        "a nested selector is relative to its parent"
    );
}

#[test]
fn starting_style_is_a_condition_not_the_base_style() {
    let plan = compile(
        r#"
        <style>
          .toast { opacity: 1; transition: opacity 200ms ease-out; }
          @starting-style { .toast { opacity: 0; } }
        </style>
        <div class="toast" id="toast">Saved</div>
        "#,
    );
    let toast = find(&plan.nodes, "toast").expect("toast");
    assert_eq!(base_value(toast, &StyleProperty::Opacity), Some("1"));
    assert_eq!(
        variant_values(
            toast,
            &RenderStyleCondition::StartingStyle,
            &StyleProperty::Opacity
        ),
        ["0"]
    );
    let list = transitions(toast.stylesheet_declarations.iter().chain(&toast.styles));
    let opacity = Transition::for_property(&list, "opacity").expect("opacity transition");
    assert_eq!(opacity.easing, Easing::EASE_OUT);
    assert!((opacity.duration_ms - 200.0).abs() < f32::EPSILON);
}

#[test]
fn keyframes_are_lowered_with_sorted_offsets() {
    let plan = compile(
        r#"
        <style>
          @keyframes slide {
            to { translate: 100px 0; }
            from, 50% { opacity: 0; }
            25% { translate: 10px 0; }
          }
          .panel { animation: slide 400ms linear both; }
        </style>
        <div class="panel" id="panel">Panel</div>
        "#,
    );
    let slide = plan.motion.keyframes("slide").expect("@keyframes slide");
    let offsets: Vec<f32> = slide.frames.iter().map(|frame| frame.offset).collect();
    assert_eq!(offsets, [0.0, 0.25, 0.5, 1.0]);

    // translate is set at 25% and 100%: before 25% it runs from the element's own value.
    let early = slide
        .interval("translate", 0.1)
        .expect("translate is animated");
    assert!(early.from.is_none());
    assert_eq!(early.to.map(|value| value.value.as_str()), Some("10px"));
    let late = slide
        .interval("translate", 0.625)
        .expect("translate is animated");
    assert_eq!(late.from.map(|value| value.value.as_str()), Some("10px"));
    assert!((late.progress - 0.5).abs() < 1e-4);
    // opacity is held at 0 through 50%, then returns to the element's value.
    let after = slide
        .interval("opacity", 0.75)
        .expect("opacity is animated");
    assert!(after.to.is_none());
    assert!(slide.interval("color", 0.5).is_none());

    let panel = find(&plan.nodes, "panel").expect("panel");
    let list = animations(panel.stylesheet_declarations.iter().chain(&panel.styles));
    assert_eq!(list[0].name.as_deref(), Some("slide"));
}

#[test]
fn view_transition_rules_keep_names_classes_and_types() {
    let plan = compile(
        r#"
        <style>
          @view-transition { navigation: auto; types: slide forwards; }
          .hero { view-transition-name: hero; view-transition-class: card; }
          ::view-transition-group(*) { animation-duration: 300ms; }
          ::view-transition-group(hero) { animation-duration: 500ms; }
          ::view-transition-old(*.card) { animation-name: fade-out; }
          html:active-view-transition-type(slide)::view-transition-old(root) {
            animation: 250ms ease-in both slide-out;
          }
          main:active-view-transition-type(slide) .hint { opacity: 0.5; }
          @media (prefers-reduced-motion: reduce) {
            ::view-transition-group(*) { animation: none; }
          }
        </style>
        <img class="hero" id="hero" alt="">
        <main><p class="hint" id="hint">Hint</p></main>
        "#,
    );
    let view_transition = &plan.motion.view_transition;
    assert!(view_transition.navigation);
    assert_eq!(view_transition.types, ["slide", "forwards"]);
    assert_eq!(view_transition.rules.len(), 5);

    let old_root = view_transition
        .rules
        .iter()
        .find(|rule| rule.part == ViewTransitionPart::Old && rule.types == ["slide"])
        .expect("typed old(root) rule");
    assert_eq!(old_root.name, ViewTransitionName::Named("root".into()));
    let card = view_transition
        .rules
        .iter()
        .find(|rule| rule.classes == ["card"])
        .expect("class rule");
    assert_eq!(card.name, ViewTransitionName::Any);

    let no_media = |_: &RenderStyleCondition| false;
    let group =
        view_transition.declarations_for(ViewTransitionPart::Group, "hero", &[], &[], no_media);
    let durations: Vec<&str> = group.iter().map(|d| d.value.as_str()).collect();
    assert_eq!(
        durations,
        [".3s", ".5s"],
        "the named group outranks the wildcard"
    );
    let reduced =
        view_transition.declarations_for(ViewTransitionPart::Group, "hero", &[], &[], |_| true);
    assert_eq!(
        reduced.last().map(|d| d.property.as_str()),
        Some("animation"),
        "media conditions are evaluated by the caller"
    );

    let untyped =
        view_transition.declarations_for(ViewTransitionPart::Old, "root", &[], &[], no_media);
    assert!(
        untyped.is_empty(),
        "a typed rule needs its type to be active"
    );
    let typed = view_transition.declarations_for(
        ViewTransitionPart::Old,
        "root",
        &[],
        &["slide".into()],
        no_media,
    );
    assert_eq!(typed.len(), 1);
    assert_eq!(
        view_transition
            .declarations_for(
                ViewTransitionPart::Old,
                "hero",
                &["card".into()],
                &[],
                no_media
            )
            .len(),
        1
    );

    let hero = find(&plan.nodes, "hero").expect("hero");
    assert_eq!(
        base_value(hero, &StyleProperty::ViewTransitionName),
        Some("hero")
    );
    let hint = find(&plan.nodes, "hint").expect("hint");
    assert_eq!(
        variant_values(
            hint,
            &RenderStyleCondition::ActiveViewTransitionType(vec!["slide".into()]),
            &StyleProperty::Opacity
        ),
        [".5"]
    );
}

#[test]
fn state_pseudo_classes_bind_to_the_element_they_are_written_on() {
    fn colors<'a>(element: &'a RenderElement, condition: &RenderStyleCondition) -> Vec<&'a str> {
        variant_values(element, condition, &StyleProperty::Color)
    }
    let plan = compile(
        r#"
        <style>
          .title { color: #000001; }
          .card:hover .title { color: #000002; }
          .card > .title:focus { color: #000003; }
          .idle:not(:hover) { color: #000004; }
          .badge + .title:hover { color: #000005; }
          .card:hover + .after { color: #000006; }
        </style>
        <div class="card" id="card">
          <span class="badge">B</span>
          <h2 class="title" id="title">Title</h2>
        </div>
        <p class="after" id="after">After</p>
        <p class="idle" id="idle">Idle</p>
        "#,
    );
    let title = find(&plan.nodes, "title").expect("title");
    assert_eq!(
        colors(
            title,
            &RenderStyleCondition::ElementState {
                pseudo: "hover".into(),
                ancestor: 1,
                negated: false
            }
        ),
        ["#000002"],
        "the card's hover, one level up, not the title's own"
    );
    assert!(
        colors(title, &RenderStyleCondition::PseudoClass("hover".into())).contains(&"#000005"),
        "a state on the subject stays a plain pseudo-class"
    );
    assert_eq!(
        colors(title, &RenderStyleCondition::PseudoClass("focus".into())),
        ["#000003"]
    );
    let idle = find(&plan.nodes, "idle").expect("idle");
    assert_eq!(
        colors(
            idle,
            &RenderStyleCondition::ElementState {
                pseudo: "hover".into(),
                ancestor: 0,
                negated: true
            }
        ),
        ["#000004"],
        ":not(:hover) is a negated state, not never-matching"
    );
    let after = find(&plan.nodes, "after").expect("after");
    assert!(
        after.style_variants.iter().all(|variant| variant
            .declarations
            .iter()
            .all(|d| d.value.as_str() != "#000006")),
        "a sibling's state is not expressible and is not misattached"
    );
}
