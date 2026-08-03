use htmlswap::{RenderElement, RenderNode, RenderStyleCondition, StyleProperty, compile_fragment};

fn compile_button(source: &str) -> Box<RenderElement> {
    let compiled = compile_fragment(source);
    assert!(
        !compiled.diagnostics.has_errors(),
        "{:?}",
        compiled.diagnostics.as_slice()
    );
    find_element(compiled.value.nodes, "button").expect("compiled plan should contain a button")
}

fn find_element(nodes: Vec<RenderNode>, tag: &str) -> Option<Box<RenderElement>> {
    for node in nodes {
        let RenderNode::Element(mut element) = node else {
            continue;
        };
        if element.source_tag == tag {
            return Some(element);
        }
        if let Some(found) = find_element(std::mem::take(&mut element.children), tag) {
            return Some(found);
        }
    }
    None
}

fn state_values<'a>(
    element: &'a RenderElement,
    state: &str,
    property: &StyleProperty,
) -> Vec<&'a str> {
    element
        .style_variants
        .iter()
        .filter(|variant| {
            variant.conditions.iter().any(
                |condition| matches!(condition, RenderStyleCondition::PseudoClass(value) if value == state),
            )
        })
        .flat_map(|variant| &variant.declarations)
        .filter(|declaration| declaration.property == *property)
        .map(|declaration| declaration.value.as_str())
        .collect()
}

#[test]
fn conditional_cascade_prefers_specificity_over_later_source_order() {
    let button = compile_button(
        r#"
            <style>
              #save:hover { background-color: #112233; }
              button:hover { background-color: #445566; border-color: #778899; }
            </style>
            <button id="save">Save</button>
        "#,
    );

    assert_eq!(
        state_values(&button, "hover", &StyleProperty::BackgroundColor),
        ["#123"]
    );
    assert_eq!(
        state_values(&button, "hover", &StyleProperty::BorderColor),
        ["#789"]
    );
}

#[test]
fn conditional_cascade_honors_important_before_specificity() {
    let button = compile_button(
        r#"
            <style>
              #save:hover { color: #112233; }
              button:hover { color: #445566 !important; }
            </style>
            <button id="save">Save</button>
        "#,
    );

    assert_eq!(
        state_values(&button, "hover", &StyleProperty::Color),
        ["#456"]
    );
}

#[test]
fn conditional_cascade_uses_later_order_for_equal_specificity() {
    let button = compile_button(
        r#"
            <style>
              .action:hover { color: #112233; }
              .action:hover { color: #445566; }
            </style>
            <button class="action">Save</button>
        "#,
    );

    assert_eq!(
        state_values(&button, "hover", &StyleProperty::Color),
        ["#456"]
    );
}

#[test]
fn selector_list_preserves_each_matching_dynamic_branch() {
    let button = compile_button(
        r#"
            <style>
              button:hover, #save:focus { color: #112233; }
            </style>
            <button id="save">Save</button>
        "#,
    );

    assert_eq!(
        state_values(&button, "hover", &StyleProperty::Color),
        ["#123"]
    );
    assert_eq!(
        state_values(&button, "focus", &StyleProperty::Color),
        ["#123"]
    );
}

#[test]
fn stylesheet_important_beats_normal_inline_base_and_state_styles() {
    let button = compile_button(
        r#"
            <style>
              #save { color: #112233 !important; }
              #save:hover { background-color: #445566 !important; }
            </style>
            <button
              id="save"
              style="color: #778899"
              style-hover="background-color: #aabbcc"
            >Save</button>
        "#,
    );

    assert_eq!(
        button
            .styles
            .iter()
            .find(|declaration| declaration.property == StyleProperty::Color)
            .map(|declaration| declaration.value.as_str()),
        Some("#123")
    );
    assert_eq!(
        state_values(&button, "hover", &StyleProperty::BackgroundColor),
        ["#456"]
    );
}

#[test]
fn inline_important_beats_stylesheet_important_for_base_and_state_styles() {
    let button = compile_button(
        r#"
            <style>
              #save { color: #112233 !important; }
              #save:hover { background-color: #445566 !important; }
            </style>
            <button
              id="save"
              style="color: #778899 !important"
              style-hover="background-color: #aabbcc !important"
            >Save</button>
        "#,
    );

    assert_eq!(
        button
            .styles
            .iter()
            .find(|declaration| declaration.property == StyleProperty::Color)
            .map(|declaration| declaration.value.as_str()),
        Some("#789")
    );
    assert_eq!(
        state_values(&button, "hover", &StyleProperty::BackgroundColor),
        ["#abc"]
    );
}
