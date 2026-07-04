//! Declarative mapping between semantic axis values and `gpui-component`
//! builder methods, shared by the forward emitter (`gpui_components`) and the
//! reverse importer (`gpui_reverse`). Adding a widget semantic means adding a
//! single entry here; both directions pick it up automatically.

/// Semantic axis a builder method encodes. `TabVariant` entries only apply to
/// tab bar elements on import; the other axes are unguarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SemanticAxis {
    Tone,
    Variant,
    TabVariant,
    Density,
}

impl SemanticAxis {
    /// Source attribute the reverse importer writes the axis value to.
    pub(crate) fn attribute(self) -> &'static str {
        match self {
            Self::Tone => "data-htmlswap-tone",
            Self::Variant | Self::TabVariant => "data-htmlswap-variant",
            Self::Density => "data-htmlswap-density",
        }
    }
}

/// One axis value ↔ builder method pairing.
pub(crate) struct SemanticMethodMapping {
    pub(crate) axis: SemanticAxis,
    /// Canonical axis token, as produced by the `Render*::as_str` methods.
    pub(crate) value: &'static str,
    /// `gpui-component` builder method name, emitted as `.{method}()`.
    pub(crate) method: &'static str,
}

const fn mapping(
    axis: SemanticAxis,
    value: &'static str,
    method: &'static str,
) -> SemanticMethodMapping {
    SemanticMethodMapping {
        axis,
        value,
        method,
    }
}

pub(crate) const SEMANTIC_METHOD_MAPPINGS: &[SemanticMethodMapping] = &[
    mapping(SemanticAxis::Tone, "accent", "primary"),
    mapping(SemanticAxis::Tone, "success", "success"),
    mapping(SemanticAxis::Tone, "warning", "warning"),
    mapping(SemanticAxis::Tone, "danger", "danger"),
    mapping(SemanticAxis::Tone, "info", "info"),
    mapping(SemanticAxis::Variant, "outline", "outline"),
    mapping(SemanticAxis::Variant, "ghost", "ghost"),
    mapping(SemanticAxis::Variant, "link", "link"),
    mapping(SemanticAxis::Variant, "text", "text"),
    mapping(SemanticAxis::TabVariant, "outline", "outline"),
    mapping(SemanticAxis::TabVariant, "pill", "pill"),
    mapping(SemanticAxis::TabVariant, "segmented", "segmented"),
    mapping(SemanticAxis::TabVariant, "underline", "underline"),
    mapping(SemanticAxis::Density, "compact", "compact"),
];

/// Boolean builder method that round-trips through a marker attribute:
/// exported as `.{method}(true)`, imported back as `{attribute}=""`.
pub(crate) struct FlagMethodMapping {
    pub(crate) method: &'static str,
    pub(crate) attribute: &'static str,
}

pub(crate) const BUTTON_LOADING: FlagMethodMapping = FlagMethodMapping {
    method: "loading",
    attribute: "data-htmlswap-loading",
};

/// `RenderSize` token ↔ `gpui_component::Size` variant pairing.
pub(crate) struct ComponentSizeMapping {
    pub(crate) value: &'static str,
    pub(crate) variant: &'static str,
}

pub(crate) const COMPONENT_SIZE_MAPPINGS: &[ComponentSizeMapping] = &[
    ComponentSizeMapping {
        value: "xs",
        variant: "XSmall",
    },
    ComponentSizeMapping {
        value: "sm",
        variant: "Small",
    },
    ComponentSizeMapping {
        value: "md",
        variant: "Medium",
    },
    ComponentSizeMapping {
        value: "lg",
        variant: "Large",
    },
];

/// Forward lookup: builder method for a semantic axis value, if one exists.
pub(crate) fn semantic_method(axis: SemanticAxis, value: &str) -> Option<&'static str> {
    SEMANTIC_METHOD_MAPPINGS
        .iter()
        .find(|mapping| mapping.axis == axis && mapping.value == value)
        .map(|mapping| mapping.method)
}

/// Reverse lookup: source attribute and value for a builder method.
/// `TabVariant` entries only match when the element is a tab bar.
pub(crate) fn semantic_attribute(
    method: &str,
    is_tab_bar: bool,
) -> Option<(&'static str, &'static str)> {
    if method == BUTTON_LOADING.method {
        return Some((BUTTON_LOADING.attribute, ""));
    }
    SEMANTIC_METHOD_MAPPINGS
        .iter()
        .find(|mapping| {
            mapping.method == method && (mapping.axis != SemanticAxis::TabVariant || is_tab_bar)
        })
        .map(|mapping| (mapping.axis.attribute(), mapping.value))
}

/// Forward lookup: `gpui_component::Size` variant for a `RenderSize` token.
pub(crate) fn component_size_variant(value: &str) -> Option<&'static str> {
    COMPONENT_SIZE_MAPPINGS
        .iter()
        .find(|mapping| mapping.value == value)
        .map(|mapping| mapping.variant)
}

/// Reverse lookup: `RenderSize` token for a `gpui_component::Size` variant.
pub(crate) fn component_size_value(variant: &str) -> Option<&'static str> {
    COMPONENT_SIZE_MAPPINGS
        .iter()
        .find(|mapping| mapping.variant == variant)
        .map(|mapping| mapping.value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{RenderDensity, RenderSize, RenderTone, RenderVariant};

    #[test]
    fn semantic_mappings_round_trip() {
        for mapping in SEMANTIC_METHOD_MAPPINGS {
            assert_eq!(
                semantic_method(mapping.axis, mapping.value),
                Some(mapping.method),
                "forward lookup for {:?} `{}` must resolve to `{}`",
                mapping.axis,
                mapping.value,
                mapping.method,
            );
            let is_tab_bar = mapping.axis == SemanticAxis::TabVariant;
            assert_eq!(
                semantic_attribute(mapping.method, is_tab_bar),
                Some((mapping.axis.attribute(), mapping.value)),
                "reverse lookup for `{}` must restore {}=\"{}\"",
                mapping.method,
                mapping.axis.attribute(),
                mapping.value,
            );
        }
    }

    #[test]
    fn size_mappings_round_trip() {
        for mapping in COMPONENT_SIZE_MAPPINGS {
            assert_eq!(component_size_variant(mapping.value), Some(mapping.variant));
            assert_eq!(component_size_value(mapping.variant), Some(mapping.value));
        }
    }

    #[test]
    fn loading_flag_round_trips() {
        assert_eq!(
            semantic_attribute(BUTTON_LOADING.method, false),
            Some((BUTTON_LOADING.attribute, "")),
        );
    }

    #[test]
    fn semantic_values_are_canonical_tokens() {
        for mapping in SEMANTIC_METHOD_MAPPINGS {
            let canonical = match mapping.axis {
                SemanticAxis::Tone => RenderTone::parse(mapping.value).as_str().to_owned(),
                SemanticAxis::Variant | SemanticAxis::TabVariant => {
                    RenderVariant::parse(mapping.value).as_str().to_owned()
                }
                SemanticAxis::Density => RenderDensity::parse(mapping.value).as_str().to_owned(),
            };
            assert_eq!(
                canonical, mapping.value,
                "table value `{}` must be the canonical token so imported attributes re-parse to the same axis value",
                mapping.value,
            );
        }
        for mapping in COMPONENT_SIZE_MAPPINGS {
            assert_eq!(RenderSize::parse(mapping.value).as_str(), mapping.value);
        }
    }
}
