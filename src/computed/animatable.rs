//! Animatable longhands of the typed computed style: which properties
//! interpolate, how to read and write them, and how their values blend
//! (CSS Values 4 §3, CSS Color 4 §12).
//!
//! Renderers drive time and state themselves (the live renderer and
//! generated code use GPUI Kit's motion runtime); this table tells them what
//! a `transition` or `@keyframes` rule animates.

use super::{ComputedStyle, LengthAuto, LengthPercentage, Rgba, Size};

/// A longhand that interpolates when it transitions or animates.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AnimatableProperty {
    Opacity,
    Color,
    BackgroundColor,
    BorderTopColor,
    BorderRightColor,
    BorderBottomColor,
    BorderLeftColor,
    BorderTopWidth,
    BorderRightWidth,
    BorderBottomWidth,
    BorderLeftWidth,
    BorderTopLeftRadius,
    BorderTopRightRadius,
    BorderBottomRightRadius,
    BorderBottomLeftRadius,
    Width,
    Height,
    MinWidth,
    MinHeight,
    MaxWidth,
    MaxHeight,
    Top,
    Right,
    Bottom,
    Left,
    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    RowGap,
    ColumnGap,
    Translate,
    FontSize,
    LetterSpacing,
    FlexGrow,
    FlexShrink,
    TextDecorationColor,
}

const COUNT: usize = 41;

impl AnimatableProperty {
    pub const ALL: [Self; COUNT] = [
        Self::Opacity,
        Self::Color,
        Self::BackgroundColor,
        Self::BorderTopColor,
        Self::BorderRightColor,
        Self::BorderBottomColor,
        Self::BorderLeftColor,
        Self::BorderTopWidth,
        Self::BorderRightWidth,
        Self::BorderBottomWidth,
        Self::BorderLeftWidth,
        Self::BorderTopLeftRadius,
        Self::BorderTopRightRadius,
        Self::BorderBottomRightRadius,
        Self::BorderBottomLeftRadius,
        Self::Width,
        Self::Height,
        Self::MinWidth,
        Self::MinHeight,
        Self::MaxWidth,
        Self::MaxHeight,
        Self::Top,
        Self::Right,
        Self::Bottom,
        Self::Left,
        Self::MarginTop,
        Self::MarginRight,
        Self::MarginBottom,
        Self::MarginLeft,
        Self::PaddingTop,
        Self::PaddingRight,
        Self::PaddingBottom,
        Self::PaddingLeft,
        Self::RowGap,
        Self::ColumnGap,
        Self::Translate,
        Self::FontSize,
        Self::LetterSpacing,
        Self::FlexGrow,
        Self::FlexShrink,
        Self::TextDecorationColor,
    ];

    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    #[must_use]
    pub const fn css_name(self) -> &'static str {
        match self {
            Self::Opacity => "opacity",
            Self::Color => "color",
            Self::BackgroundColor => "background-color",
            Self::BorderTopColor => "border-top-color",
            Self::BorderRightColor => "border-right-color",
            Self::BorderBottomColor => "border-bottom-color",
            Self::BorderLeftColor => "border-left-color",
            Self::BorderTopWidth => "border-top-width",
            Self::BorderRightWidth => "border-right-width",
            Self::BorderBottomWidth => "border-bottom-width",
            Self::BorderLeftWidth => "border-left-width",
            Self::BorderTopLeftRadius => "border-top-left-radius",
            Self::BorderTopRightRadius => "border-top-right-radius",
            Self::BorderBottomRightRadius => "border-bottom-right-radius",
            Self::BorderBottomLeftRadius => "border-bottom-left-radius",
            Self::Width => "width",
            Self::Height => "height",
            Self::MinWidth => "min-width",
            Self::MinHeight => "min-height",
            Self::MaxWidth => "max-width",
            Self::MaxHeight => "max-height",
            Self::Top => "top",
            Self::Right => "right",
            Self::Bottom => "bottom",
            Self::Left => "left",
            Self::MarginTop => "margin-top",
            Self::MarginRight => "margin-right",
            Self::MarginBottom => "margin-bottom",
            Self::MarginLeft => "margin-left",
            Self::PaddingTop => "padding-top",
            Self::PaddingRight => "padding-right",
            Self::PaddingBottom => "padding-bottom",
            Self::PaddingLeft => "padding-left",
            Self::RowGap => "row-gap",
            Self::ColumnGap => "column-gap",
            Self::Translate => "translate",
            Self::FontSize => "font-size",
            Self::LetterSpacing => "letter-spacing",
            Self::FlexGrow => "flex-grow",
            Self::FlexShrink => "flex-shrink",
            Self::TextDecorationColor => "text-decoration-color",
        }
    }

    /// The property's interpolable computed value in `style`, if it sets one.
    #[must_use]
    pub fn get(self, style: &ComputedStyle) -> Option<AnimatedValue> {
        use AnimatedValue::{Length, Number};
        let size = |size: Option<Size>| match size? {
            Size::Length(length) => Some(Length(length)),
            _ => None,
        };
        let auto = |value: Option<LengthAuto>| match value? {
            LengthAuto::Length(length) => Some(Length(length)),
            LengthAuto::Auto => None,
        };
        let color = |value: Option<Rgba>| value.map(AnimatedValue::color);
        match self {
            Self::Opacity => style.opacity.map(Number),
            Self::Color => color(style.color),
            Self::BackgroundColor => color(style.background_color),
            Self::BorderTopColor => color(style.border_color.top),
            Self::BorderRightColor => color(style.border_color.right),
            Self::BorderBottomColor => color(style.border_color.bottom),
            Self::BorderLeftColor => color(style.border_color.left),
            Self::BorderTopWidth => style.border_width.top.map(Length),
            Self::BorderRightWidth => style.border_width.right.map(Length),
            Self::BorderBottomWidth => style.border_width.bottom.map(Length),
            Self::BorderLeftWidth => style.border_width.left.map(Length),
            Self::BorderTopLeftRadius => style.border_radius.top_left.map(Length),
            Self::BorderTopRightRadius => style.border_radius.top_right.map(Length),
            Self::BorderBottomRightRadius => style.border_radius.bottom_right.map(Length),
            Self::BorderBottomLeftRadius => style.border_radius.bottom_left.map(Length),
            Self::Width => size(style.width),
            Self::Height => size(style.height),
            Self::MinWidth => size(style.min_width),
            Self::MinHeight => size(style.min_height),
            Self::MaxWidth => size(style.max_width),
            Self::MaxHeight => size(style.max_height),
            Self::Top => auto(style.inset.top),
            Self::Right => auto(style.inset.right),
            Self::Bottom => auto(style.inset.bottom),
            Self::Left => auto(style.inset.left),
            Self::MarginTop => auto(style.margin.top),
            Self::MarginRight => auto(style.margin.right),
            Self::MarginBottom => auto(style.margin.bottom),
            Self::MarginLeft => auto(style.margin.left),
            Self::PaddingTop => style.padding.top.map(Length),
            Self::PaddingRight => style.padding.right.map(Length),
            Self::PaddingBottom => style.padding.bottom.map(Length),
            Self::PaddingLeft => style.padding.left.map(Length),
            Self::RowGap => style.row_gap.map(Length),
            Self::ColumnGap => style.column_gap.map(Length),
            Self::Translate => style.translate.map(|(x, y)| AnimatedValue::Translate(x, y)),
            Self::FontSize => style.font_size.map(Length),
            Self::LetterSpacing => style.letter_spacing.map(Length),
            Self::FlexGrow => style.flex_grow.map(Number),
            Self::FlexShrink => style.flex_shrink.map(Number),
            Self::TextDecorationColor => color(style.text_decoration_color),
        }
    }

    /// Write a value of this property into `style`.
    pub fn set(self, style: &mut ComputedStyle, value: AnimatedValue) {
        let length = value.length();
        let number = value.number();
        let rgba = value.rgba();
        // Values outside a property's range clamp to it (CSS Values 4 §10),
        // which a length with runtime parts can only do once resolved.
        let non_negative = length.map(|length| match length.as_px() {
            Some(px) => LengthPercentage::px(px.max(0.0)),
            None => length,
        });
        match self {
            Self::Opacity => style.opacity = number.map(|value| value.clamp(0.0, 1.0)),
            Self::Color => style.color = rgba,
            Self::BackgroundColor => style.background_color = rgba,
            Self::BorderTopColor => style.border_color.top = rgba,
            Self::BorderRightColor => style.border_color.right = rgba,
            Self::BorderBottomColor => style.border_color.bottom = rgba,
            Self::BorderLeftColor => style.border_color.left = rgba,
            Self::BorderTopWidth => style.border_width.top = non_negative,
            Self::BorderRightWidth => style.border_width.right = non_negative,
            Self::BorderBottomWidth => style.border_width.bottom = non_negative,
            Self::BorderLeftWidth => style.border_width.left = non_negative,
            Self::BorderTopLeftRadius => style.border_radius.top_left = length,
            Self::BorderTopRightRadius => style.border_radius.top_right = length,
            Self::BorderBottomRightRadius => style.border_radius.bottom_right = length,
            Self::BorderBottomLeftRadius => style.border_radius.bottom_left = length,
            Self::Width => style.width = length.map(Size::Length),
            Self::Height => style.height = length.map(Size::Length),
            Self::MinWidth => style.min_width = length.map(Size::Length),
            Self::MinHeight => style.min_height = length.map(Size::Length),
            Self::MaxWidth => style.max_width = length.map(Size::Length),
            Self::MaxHeight => style.max_height = length.map(Size::Length),
            Self::Top => style.inset.top = length.map(LengthAuto::Length),
            Self::Right => style.inset.right = length.map(LengthAuto::Length),
            Self::Bottom => style.inset.bottom = length.map(LengthAuto::Length),
            Self::Left => style.inset.left = length.map(LengthAuto::Length),
            Self::MarginTop => style.margin.top = length.map(LengthAuto::Length),
            Self::MarginRight => style.margin.right = length.map(LengthAuto::Length),
            Self::MarginBottom => style.margin.bottom = length.map(LengthAuto::Length),
            Self::MarginLeft => style.margin.left = length.map(LengthAuto::Length),
            Self::PaddingTop => style.padding.top = length,
            Self::PaddingRight => style.padding.right = length,
            Self::PaddingBottom => style.padding.bottom = length,
            Self::PaddingLeft => style.padding.left = length,
            Self::RowGap => style.row_gap = length,
            Self::ColumnGap => style.column_gap = length,
            Self::Translate => style.translate = value.translate(),
            Self::FontSize => style.font_size = non_negative,
            Self::LetterSpacing => style.letter_spacing = length,
            Self::FlexGrow => style.flex_grow = number.map(|value| value.max(0.0)),
            Self::FlexShrink => style.flex_shrink = number.map(|value| value.max(0.0)),
            Self::TextDecorationColor => style.text_decoration_color = rgba,
        }
    }

    /// The computed value when no declaration sets the property, where it
    /// is interpolable. Inherited properties take the inherited value.
    #[must_use]
    pub fn initial(self, underlying: &Underlying) -> Option<AnimatedValue> {
        use AnimatedValue::{Length, Number};
        Some(match self {
            Self::Opacity => Number(1.0),
            Self::Color | Self::TextDecorationColor => {
                AnimatedValue::color(underlying.current_color)
            }
            Self::BackgroundColor => AnimatedValue::Color([0.0; 4]),
            Self::BorderTopColor
            | Self::BorderRightColor
            | Self::BorderBottomColor
            | Self::BorderLeftColor => AnimatedValue::color(underlying.current_color),
            Self::BorderTopWidth
            | Self::BorderRightWidth
            | Self::BorderBottomWidth
            | Self::BorderLeftWidth
            | Self::BorderTopLeftRadius
            | Self::BorderTopRightRadius
            | Self::BorderBottomRightRadius
            | Self::BorderBottomLeftRadius
            | Self::MarginTop
            | Self::MarginRight
            | Self::MarginBottom
            | Self::MarginLeft
            | Self::PaddingTop
            | Self::PaddingRight
            | Self::PaddingBottom
            | Self::PaddingLeft
            | Self::RowGap
            | Self::ColumnGap
            | Self::LetterSpacing => Length(LengthPercentage::ZERO),
            Self::Translate => {
                AnimatedValue::Translate(LengthPercentage::ZERO, LengthPercentage::ZERO)
            }
            Self::FontSize => Length(underlying.font_size),
            Self::FlexGrow => Number(0.0),
            Self::FlexShrink => Number(1.0),
            // `auto` and `none` do not interpolate.
            Self::Width
            | Self::Height
            | Self::MinWidth
            | Self::MinHeight
            | Self::MaxWidth
            | Self::MaxHeight
            | Self::Top
            | Self::Right
            | Self::Bottom
            | Self::Left => return None,
        })
    }
}

/// An interpolable computed value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnimatedValue {
    /// Premultiplied RGBA in `0..=1`, as CSS Color 4 §12.3 interpolates, so
    /// fades through transparent do not darken.
    Color([f32; 4]),
    Number(f32),
    /// Lengths and percentages interpolate into `calc()` mixes.
    Length(LengthPercentage),
    Translate(LengthPercentage, LengthPercentage),
}

impl AnimatedValue {
    #[must_use]
    pub fn color(color: Rgba) -> Self {
        let [r, g, b, a] = [color.r, color.g, color.b, color.a].map(|byte| f32::from(byte) / 255.0);
        Self::Color([r * a, g * a, b * a, a])
    }

    #[must_use]
    pub fn rgba(self) -> Option<Rgba> {
        let Self::Color([r, g, b, a]) = self else {
            return None;
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        let unpremultiply = |channel: f32| if a > 0.0 { channel / a } else { 0.0 };
        Some(Rgba {
            r: byte(unpremultiply(r)),
            g: byte(unpremultiply(g)),
            b: byte(unpremultiply(b)),
            a: byte(a),
        })
    }

    #[must_use]
    pub const fn number(self) -> Option<f32> {
        match self {
            Self::Number(value) => Some(value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn length(self) -> Option<LengthPercentage> {
        match self {
            Self::Length(value) => Some(value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn translate(self) -> Option<(LengthPercentage, LengthPercentage)> {
        match self {
            Self::Translate(x, y) => Some((x, y)),
            _ => None,
        }
    }

    /// Interpolate, or `None` when the values have no common form, in which
    /// case CSS switches between them at the midpoint.
    #[must_use]
    pub fn lerp(self, to: Self, t: f32) -> Option<Self> {
        let lerp = |a: f32, b: f32| a + (b - a) * t;
        Some(match (self, to) {
            (Self::Color(from), Self::Color(to)) => {
                Self::Color(std::array::from_fn(|index| lerp(from[index], to[index])))
            }
            (Self::Number(from), Self::Number(to)) => Self::Number(lerp(from, to)),
            (Self::Length(from), Self::Length(to)) => Self::Length(from.lerp(to, t)),
            (Self::Translate(fx, fy), Self::Translate(tx, ty)) => {
                Self::Translate(fx.lerp(tx, t), fy.lerp(ty, t))
            }
            _ => return None,
        })
    }

    /// Interpolate, switching at the midpoint when the values have no
    /// common form (CSS discrete interpolation).
    #[must_use]
    pub fn mix(self, to: Self, t: f32) -> Self {
        self.lerp(to, t).unwrap_or(if t < 0.5 { self } else { to })
    }
}

/// Inherited values that initial values of some properties refer to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Underlying {
    /// The inherited `color`, for `color` and `currentColor` initials.
    pub current_color: Rgba,
    /// The inherited `font-size`.
    pub font_size: LengthPercentage,
}

#[cfg(test)]
mod tests {
    use super::{AnimatableProperty, AnimatedValue, Underlying};
    use crate::computed::{ComputedStyle, LengthPercentage, Rgba, Size};

    #[test]
    fn lengths_and_percentages_interpolate_through_calc() {
        let from = AnimatedValue::Length(LengthPercentage::px(100.0));
        let to = AnimatedValue::Length(LengthPercentage::fraction(0.5));
        assert_eq!(
            from.lerp(to, 0.5),
            Some(AnimatedValue::Length(LengthPercentage {
                px: 50.0,
                fraction: 0.25,
                ..LengthPercentage::ZERO
            }))
        );
    }

    #[test]
    fn colors_fade_through_transparent_without_darkening() {
        let red = AnimatedValue::color(Rgba::opaque(0xff0000));
        let clear = AnimatedValue::color(Rgba {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        });
        assert_eq!(
            red.lerp(clear, 0.5).and_then(AnimatedValue::rgba),
            Some(Rgba {
                r: 255,
                g: 0,
                b: 0,
                a: 128
            })
        );
    }

    #[test]
    fn every_property_round_trips_through_the_style() {
        let underlying = Underlying {
            current_color: Rgba::opaque(0x123456),
            font_size: LengthPercentage::rem(1.0),
        };
        for property in AnimatableProperty::ALL {
            let value = property
                .initial(&underlying)
                .unwrap_or_else(|| match property {
                    AnimatableProperty::Width
                    | AnimatableProperty::Height
                    | AnimatableProperty::MinWidth
                    | AnimatableProperty::MinHeight
                    | AnimatableProperty::MaxWidth
                    | AnimatableProperty::MaxHeight
                    | AnimatableProperty::Top
                    | AnimatableProperty::Right
                    | AnimatableProperty::Bottom
                    | AnimatableProperty::Left => AnimatedValue::Length(LengthPercentage::px(7.0)),
                    _ => unreachable!("{property:?} has an initial value"),
                });
            let mut style = ComputedStyle::default();
            property.set(&mut style, value);
            assert_eq!(property.get(&style), Some(value), "{property:?}");
        }
        let style = ComputedStyle {
            width: Some(Size::Auto),
            ..ComputedStyle::default()
        };
        assert_eq!(
            AnimatableProperty::Width.get(&style),
            None,
            "auto does not interpolate"
        );
    }
}
