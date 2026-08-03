use std::fmt;

use compact_str::CompactString;

use crate::source::Span;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct StyleOrigin {
    pub declaration_span: Option<Span>,
    pub selector_span: Option<Span>,
    pub candidate_span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleDeclaration {
    pub property: StyleProperty,
    pub value: StyleValue,
    pub important: bool,
    pub span: Option<Span>,
}

impl StyleDeclaration {
    #[must_use]
    pub fn new(
        property: impl Into<StyleProperty>,
        value: impl Into<CompactString>,
        important: bool,
        span: Option<Span>,
    ) -> Self {
        let property = property.into();
        let value = StyleValue::from_raw_with_span(&property, value.into(), span);
        Self {
            property,
            value,
            important,
            span,
        }
    }

    #[must_use]
    pub fn with_value(
        property: impl Into<StyleProperty>,
        value: StyleValue,
        important: bool,
        span: Option<Span>,
    ) -> Self {
        Self {
            property: property.into(),
            value,
            important,
            span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StyleProperty {
    Display,
    Color,
    Background,
    BackgroundColor,
    BackgroundClip,
    BackgroundImage,
    BackgroundSize,
    BoxSizing,
    Gap,
    Padding,
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    Margin,
    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,
    Width,
    Height,
    MinWidth,
    MinHeight,
    MaxWidth,
    MaxHeight,
    AspectRatio,
    Overflow,
    OverflowX,
    OverflowY,
    Position,
    Top,
    Right,
    Bottom,
    Left,
    Inset,
    ZIndex,
    Flex,
    FlexDirection,
    FlexWrap,
    FlexBasis,
    FlexGrow,
    FlexShrink,
    AlignItems,
    AlignSelf,
    AlignContent,
    JustifyContent,
    GridTemplateColumns,
    GridColumn,
    Border,
    BorderTop,
    BorderRight,
    BorderBottom,
    BorderLeft,
    BorderWidth,
    BorderStyle,
    BorderTopStyle,
    BorderRightStyle,
    BorderBottomStyle,
    BorderLeftStyle,
    BorderColor,
    BorderRadius,
    BoxShadow,
    BackdropFilter,
    FontSize,
    FontWeight,
    FontFamily,
    FontStyle,
    FontVariationSettings,
    LineHeight,
    LetterSpacing,
    TextAlign,
    TextDecoration,
    TextOverflow,
    TextTransform,
    WordBreak,
    WhiteSpace,
    VerticalAlign,
    Direction,
    UserSelect,
    Cursor,
    Opacity,
    Outline,
    PointerEvents,
    Filter,
    Transform,
    TransformOrigin,
    Animation,
    ClipPath,
    WebkitFontSmoothing,
    Content,
    Custom(CompactString),
    Unknown(CompactString),
}

impl StyleProperty {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Display => "display",
            Self::Color => "color",
            Self::Background => "background",
            Self::BackgroundColor => "background-color",
            Self::BackgroundClip => "background-clip",
            Self::BackgroundImage => "background-image",
            Self::BackgroundSize => "background-size",
            Self::BoxSizing => "box-sizing",
            Self::Gap => "gap",
            Self::Padding => "padding",
            Self::PaddingTop => "padding-top",
            Self::PaddingRight => "padding-right",
            Self::PaddingBottom => "padding-bottom",
            Self::PaddingLeft => "padding-left",
            Self::Margin => "margin",
            Self::MarginTop => "margin-top",
            Self::MarginRight => "margin-right",
            Self::MarginBottom => "margin-bottom",
            Self::MarginLeft => "margin-left",
            Self::Width => "width",
            Self::Height => "height",
            Self::MinWidth => "min-width",
            Self::MinHeight => "min-height",
            Self::MaxWidth => "max-width",
            Self::MaxHeight => "max-height",
            Self::AspectRatio => "aspect-ratio",
            Self::Overflow => "overflow",
            Self::OverflowX => "overflow-x",
            Self::OverflowY => "overflow-y",
            Self::Position => "position",
            Self::Top => "top",
            Self::Right => "right",
            Self::Bottom => "bottom",
            Self::Left => "left",
            Self::Inset => "inset",
            Self::ZIndex => "z-index",
            Self::Flex => "flex",
            Self::FlexDirection => "flex-direction",
            Self::FlexWrap => "flex-wrap",
            Self::FlexBasis => "flex-basis",
            Self::FlexGrow => "flex-grow",
            Self::FlexShrink => "flex-shrink",
            Self::AlignItems => "align-items",
            Self::AlignSelf => "align-self",
            Self::AlignContent => "align-content",
            Self::JustifyContent => "justify-content",
            Self::GridTemplateColumns => "grid-template-columns",
            Self::GridColumn => "grid-column",
            Self::Border => "border",
            Self::BorderTop => "border-top",
            Self::BorderRight => "border-right",
            Self::BorderBottom => "border-bottom",
            Self::BorderLeft => "border-left",
            Self::BorderWidth => "border-width",
            Self::BorderStyle => "border-style",
            Self::BorderTopStyle => "border-top-style",
            Self::BorderRightStyle => "border-right-style",
            Self::BorderBottomStyle => "border-bottom-style",
            Self::BorderLeftStyle => "border-left-style",
            Self::BorderColor => "border-color",
            Self::BorderRadius => "border-radius",
            Self::BoxShadow => "box-shadow",
            Self::BackdropFilter => "backdrop-filter",
            Self::FontSize => "font-size",
            Self::FontWeight => "font-weight",
            Self::FontFamily => "font-family",
            Self::FontStyle => "font-style",
            Self::FontVariationSettings => "font-variation-settings",
            Self::LineHeight => "line-height",
            Self::LetterSpacing => "letter-spacing",
            Self::TextAlign => "text-align",
            Self::TextDecoration => "text-decoration",
            Self::TextOverflow => "text-overflow",
            Self::TextTransform => "text-transform",
            Self::WordBreak => "word-break",
            Self::WhiteSpace => "white-space",
            Self::VerticalAlign => "vertical-align",
            Self::Direction => "direction",
            Self::UserSelect => "user-select",
            Self::Cursor => "cursor",
            Self::Opacity => "opacity",
            Self::Outline => "outline",
            Self::PointerEvents => "pointer-events",
            Self::Filter => "filter",
            Self::Transform => "transform",
            Self::TransformOrigin => "transform-origin",
            Self::Animation => "animation",
            Self::ClipPath => "clip-path",
            Self::WebkitFontSmoothing => "-webkit-font-smoothing",
            Self::Content => "content",
            Self::Custom(property) => property,
            Self::Unknown(property) => property,
        }
    }
}

impl From<CompactString> for StyleProperty {
    fn from(property: CompactString) -> Self {
        match property.as_str() {
            "display" => Self::Display,
            "color" => Self::Color,
            "background" => Self::Background,
            "background-color" => Self::BackgroundColor,
            "background-clip" => Self::BackgroundClip,
            "background-image" => Self::BackgroundImage,
            "background-size" => Self::BackgroundSize,
            "box-sizing" => Self::BoxSizing,
            "gap" => Self::Gap,
            "padding" => Self::Padding,
            "padding-top" => Self::PaddingTop,
            "padding-right" => Self::PaddingRight,
            "padding-bottom" => Self::PaddingBottom,
            "padding-left" => Self::PaddingLeft,
            "margin" => Self::Margin,
            "margin-top" => Self::MarginTop,
            "margin-right" => Self::MarginRight,
            "margin-bottom" => Self::MarginBottom,
            "margin-left" => Self::MarginLeft,
            "width" => Self::Width,
            "height" => Self::Height,
            "min-width" => Self::MinWidth,
            "min-height" => Self::MinHeight,
            "max-width" => Self::MaxWidth,
            "max-height" => Self::MaxHeight,
            "aspect-ratio" => Self::AspectRatio,
            "overflow" => Self::Overflow,
            "overflow-x" => Self::OverflowX,
            "overflow-y" => Self::OverflowY,
            "position" => Self::Position,
            "top" => Self::Top,
            "right" => Self::Right,
            "bottom" => Self::Bottom,
            "left" => Self::Left,
            "inset" => Self::Inset,
            "z-index" => Self::ZIndex,
            "flex" => Self::Flex,
            "flex-direction" => Self::FlexDirection,
            "flex-wrap" => Self::FlexWrap,
            "flex-basis" => Self::FlexBasis,
            "flex-grow" => Self::FlexGrow,
            "flex-shrink" => Self::FlexShrink,
            "align-items" => Self::AlignItems,
            "align-self" => Self::AlignSelf,
            "align-content" => Self::AlignContent,
            "justify-content" => Self::JustifyContent,
            "grid-template-columns" => Self::GridTemplateColumns,
            "grid-column" => Self::GridColumn,
            "border" => Self::Border,
            "border-top" => Self::BorderTop,
            "border-right" => Self::BorderRight,
            "border-bottom" => Self::BorderBottom,
            "border-left" => Self::BorderLeft,
            "border-width" => Self::BorderWidth,
            "border-style" => Self::BorderStyle,
            "border-top-style" => Self::BorderTopStyle,
            "border-right-style" => Self::BorderRightStyle,
            "border-bottom-style" => Self::BorderBottomStyle,
            "border-left-style" => Self::BorderLeftStyle,
            "border-color" => Self::BorderColor,
            "border-radius" => Self::BorderRadius,
            "box-shadow" => Self::BoxShadow,
            "backdrop-filter" => Self::BackdropFilter,
            "font-size" => Self::FontSize,
            "font-weight" => Self::FontWeight,
            "font-family" => Self::FontFamily,
            "font-style" => Self::FontStyle,
            "font-variation-settings" => Self::FontVariationSettings,
            "line-height" => Self::LineHeight,
            "letter-spacing" => Self::LetterSpacing,
            "text-align" => Self::TextAlign,
            "text-decoration" => Self::TextDecoration,
            "text-overflow" => Self::TextOverflow,
            "text-transform" => Self::TextTransform,
            "word-break" => Self::WordBreak,
            "white-space" => Self::WhiteSpace,
            "vertical-align" => Self::VerticalAlign,
            "direction" => Self::Direction,
            "user-select" => Self::UserSelect,
            "cursor" => Self::Cursor,
            "opacity" => Self::Opacity,
            "outline" => Self::Outline,
            "pointer-events" => Self::PointerEvents,
            "filter" => Self::Filter,
            "transform" => Self::Transform,
            "transform-origin" => Self::TransformOrigin,
            "animation" => Self::Animation,
            "clip-path" => Self::ClipPath,
            "-webkit-font-smoothing" => Self::WebkitFontSmoothing,
            "content" => Self::Content,
            _ if property.starts_with("--") => Self::Custom(property),
            _ => Self::Unknown(property),
        }
    }
}

impl From<String> for StyleProperty {
    fn from(property: String) -> Self {
        Self::from(CompactString::from(property))
    }
}

impl From<&str> for StyleProperty {
    fn from(property: &str) -> Self {
        Self::from(CompactString::from(property.to_ascii_lowercase()))
    }
}

impl fmt::Display for StyleProperty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StyleValue {
    Keyword(CompactString),
    Length(CompactString),
    Color(CompactString),
    Token(StyleToken),
    Raw(CompactString),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StyleToken {
    pub name: CompactString,
    pub raw: CompactString,
    pub fallback: Option<Box<StyleValue>>,
    pub span: Option<Span>,
}

impl StyleValue {
    #[must_use]
    pub fn from_raw(property: &StyleProperty, value: CompactString) -> Self {
        Self::from_raw_with_span(property, value, None)
    }

    #[must_use]
    pub fn from_raw_with_span(
        property: &StyleProperty,
        value: CompactString,
        span: Option<Span>,
    ) -> Self {
        if let Some(token) = parse_css_var_token(&value, span) {
            return Self::Token(token);
        }

        if matches!(property, StyleProperty::Custom(_)) || should_preserve_raw(&value) {
            return Self::Raw(value);
        }

        if matches!(
            property,
            StyleProperty::Color | StyleProperty::BackgroundColor | StyleProperty::BorderColor
        ) {
            return Self::Color(value);
        }

        if matches!(
            property,
            StyleProperty::Gap
                | StyleProperty::Padding
                | StyleProperty::PaddingTop
                | StyleProperty::PaddingRight
                | StyleProperty::PaddingBottom
                | StyleProperty::PaddingLeft
                | StyleProperty::Margin
                | StyleProperty::MarginTop
                | StyleProperty::MarginRight
                | StyleProperty::MarginBottom
                | StyleProperty::MarginLeft
                | StyleProperty::Width
                | StyleProperty::Height
                | StyleProperty::MinWidth
                | StyleProperty::MinHeight
                | StyleProperty::MaxWidth
                | StyleProperty::MaxHeight
                | StyleProperty::AspectRatio
                | StyleProperty::Top
                | StyleProperty::Right
                | StyleProperty::Bottom
                | StyleProperty::Left
                | StyleProperty::Inset
                | StyleProperty::FlexBasis
                | StyleProperty::BorderWidth
                | StyleProperty::BorderRadius
                | StyleProperty::FontSize
                | StyleProperty::LineHeight
                | StyleProperty::LetterSpacing
        ) {
            return Self::Length(value);
        }

        if matches!(
            property,
            StyleProperty::Display
                | StyleProperty::BoxSizing
                | StyleProperty::Overflow
                | StyleProperty::OverflowX
                | StyleProperty::OverflowY
                | StyleProperty::Position
                | StyleProperty::FlexDirection
                | StyleProperty::FlexWrap
                | StyleProperty::FlexGrow
                | StyleProperty::FlexShrink
                | StyleProperty::AlignItems
                | StyleProperty::AlignSelf
                | StyleProperty::AlignContent
                | StyleProperty::JustifyContent
                | StyleProperty::FontWeight
                | StyleProperty::FontStyle
                | StyleProperty::TextAlign
                | StyleProperty::TextDecoration
                | StyleProperty::TextOverflow
                | StyleProperty::TextTransform
                | StyleProperty::WordBreak
                | StyleProperty::WhiteSpace
                | StyleProperty::VerticalAlign
                | StyleProperty::Direction
                | StyleProperty::UserSelect
                | StyleProperty::Cursor
                | StyleProperty::Opacity
                | StyleProperty::Outline
                | StyleProperty::PointerEvents
                | StyleProperty::TransformOrigin
                | StyleProperty::BackgroundClip
                | StyleProperty::BackgroundSize
                | StyleProperty::WebkitFontSmoothing
        ) {
            return Self::Keyword(value);
        }

        Self::Raw(value)
    }

    #[must_use]
    pub fn from_theme_raw(value: impl Into<CompactString>, span: Option<Span>) -> Self {
        let value = value.into();
        if let Some(token) = parse_css_var_token(&value, span) {
            return Self::Token(token);
        }

        if looks_like_color(&value) {
            return Self::Color(value);
        }

        if looks_like_length(&value) {
            return Self::Length(value);
        }

        if value.split_whitespace().nth(1).is_none()
            && value
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        {
            return Self::Keyword(value);
        }

        Self::Raw(value)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Keyword(value) | Self::Length(value) | Self::Color(value) | Self::Raw(value) => {
                value
            }
            Self::Token(token) => &token.raw,
        }
    }

    #[must_use]
    pub fn tokens(&self) -> Vec<StyleToken> {
        self.tokens_with_span(None)
    }

    #[must_use]
    pub fn tokens_with_span(&self, span: Option<Span>) -> Vec<StyleToken> {
        match self {
            Self::Token(token) => {
                let mut tokens = vec![token.clone()];
                if let Some(fallback) = &token.fallback {
                    tokens.extend(fallback.tokens_with_span(span));
                }
                tokens
            }
            Self::Keyword(value) | Self::Length(value) | Self::Color(value) | Self::Raw(value) => {
                collect_css_var_tokens(value, span)
            }
        }
    }
}

fn should_preserve_raw(value: &str) -> bool {
    let value = value.trim();
    value.contains('(')
        || value.contains(',')
        || value.contains("var(")
        || value.contains("calc(")
        || value.contains("url(")
        || value.split_whitespace().nth(1).is_some()
}

fn parse_css_var_token(value: &str, span: Option<Span>) -> Option<StyleToken> {
    let raw = value.trim();
    let inner = raw.strip_prefix("var(")?.strip_suffix(')')?;
    parse_css_var_inner(raw, inner, span)
}

fn parse_css_var_inner(raw: &str, inner: &str, span: Option<Span>) -> Option<StyleToken> {
    let (name, fallback) = split_css_var_arguments(inner);
    let name = name.trim();
    if !name.starts_with("--") || name.split_whitespace().nth(1).is_some() {
        return None;
    }

    let fallback = fallback
        .map(str::trim)
        .filter(|fallback| !fallback.is_empty())
        .map(|fallback| Box::new(StyleValue::from_theme_raw(fallback, span)));

    Some(StyleToken {
        name: CompactString::from(name),
        raw: CompactString::from(raw),
        fallback,
        span,
    })
}

fn collect_css_var_tokens(value: &str, span: Option<Span>) -> Vec<StyleToken> {
    let mut tokens = Vec::new();
    let mut start = 0usize;

    while let Some(offset) = value[start..].find("var(") {
        let function_start = start + offset;
        let inner_start = function_start + "var(".len();
        let Some(function_end) = css_function_end(value, inner_start) else {
            break;
        };

        let raw = &value[function_start..function_end];
        let inner = &value[inner_start..function_end - 1];
        if let Some(token) = parse_css_var_inner(raw, inner, span) {
            if let Some(fallback) = &token.fallback {
                tokens.extend(fallback.tokens_with_span(span));
            }
            tokens.push(token);
        }

        start = function_end;
    }

    tokens
}

fn css_function_end(value: &str, inner_start: usize) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    let mut depth = 1usize;

    for (offset, ch) in value[inner_start..].char_indices() {
        if let Some(quote_ch) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == quote_ch {
                quote = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' => quote = Some(ch),
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(inner_start + offset + ch.len_utf8());
                }
            }
            _ => {}
        }
    }

    None
}

fn split_css_var_arguments(inner: &str) -> (&str, Option<&str>) {
    let mut quote = None;
    let mut escaped = false;
    let mut depth = 0usize;

    for (index, ch) in inner.char_indices() {
        if let Some(quote_ch) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == quote_ch {
                quote = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' => quote = Some(ch),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => return (&inner[..index], Some(&inner[index + 1..])),
            _ => {}
        }
    }

    (inner, None)
}

fn looks_like_color(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value.starts_with('#')
        || matches!(
            value.as_str(),
            "black" | "white" | "red" | "green" | "blue" | "transparent"
        )
        || value.starts_with("rgb(")
        || value.starts_with("rgba(")
        || value.starts_with("hsl(")
        || value.starts_with("hsla(")
        || value.starts_with("oklch(")
        || value.starts_with("oklab(")
        || value.starts_with("lab(")
        || value.starts_with("lch(")
}

fn looks_like_length(value: &str) -> bool {
    let value = value.trim();
    value == "0"
        || value.ends_with("px")
        || value.ends_with("rem")
        || value.ends_with("em")
        || value.ends_with("vh")
        || value.ends_with("vw")
        || value.ends_with('%')
}

impl fmt::Display for StyleValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Token(token) => {
                write!(f, "var({}", token.name)?;
                if let Some(fallback) = &token.fallback {
                    write!(f, ", {fallback}")?;
                }
                f.write_str(")")
            }
            _ => f.write_str(self.as_str()),
        }
    }
}
