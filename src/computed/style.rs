//! Typed computed styles: what a renderer needs from an element's
//! declarations, parsed by lightningcss's property parsers and resolved to
//! absolute units.
//!
//! [`ComputedStyle`] holds only what the declarations set (every field is an
//! `Option`), so it doubles as a partial style for interaction states such as
//! `:hover`. Declarations should first pass through
//! [`ComputedScope::resolve`](super::ComputedScope::resolve) so that `var()`,
//! `light-dark()` and `currentColor` are already substituted.

use compact_str::CompactString;
use lightningcss::properties::align::{
    AlignContent as CssAlignContent, AlignItems as CssAlignItems, AlignSelf as CssAlignSelf,
    BaselinePosition, ContentDistribution, ContentPosition, GapValue,
    JustifyContent as CssJustifyContent, JustifyItems as CssJustifyItems,
    JustifySelf as CssJustifySelf, SelfPosition,
};
use lightningcss::properties::border::{BorderSideWidth, LineStyle};
use lightningcss::properties::box_shadow::BoxShadow as CssBoxShadow;
use lightningcss::properties::display::{
    Display as CssDisplay, DisplayInside, DisplayKeyword, DisplayOutside,
    Visibility as CssVisibility,
};
use lightningcss::properties::flex::{FlexDirection as CssFlexDirection, FlexWrap as CssFlexWrap};
use lightningcss::properties::font::{
    AbsoluteFontSize, AbsoluteFontWeight, FontFamily as CssFontFamily, FontSize,
    FontStyle as CssFontStyle, FontWeight as CssFontWeight, GenericFontFamily,
    LineHeight as CssLineHeight, RelativeFontSize,
};
use lightningcss::properties::grid::{
    GridAutoFlow as CssGridAutoFlow, GridLine as CssGridLine, RepeatCount as CssRepeatCount,
    TrackBreadth as CssTrackBreadth, TrackListItem, TrackSize as CssTrackSize, TrackSizing,
};
use lightningcss::properties::overflow::{OverflowKeyword, TextOverflow as CssTextOverflow};
use lightningcss::properties::position::Position as CssPosition;
use lightningcss::properties::size::{BoxSizing as CssBoxSizing, MaxSize, Size as CssSize};
use lightningcss::properties::text::{
    Spacing, TextAlign as CssTextAlign, TextDecorationLine,
    TextDecorationStyle as CssDecorationStyle, TextDecorationThickness,
    WhiteSpace as CssWhiteSpace,
};
use lightningcss::properties::transform::Translate;
use lightningcss::properties::ui::{Cursor as CssCursor, CursorKeyword};
use lightningcss::properties::{Property, PropertyId};
use lightningcss::stylesheet::ParserOptions;
use lightningcss::values::color::CssColor;
use lightningcss::values::length::{
    LengthPercentage as CssLengthPercentage, LengthPercentageOrAuto,
};

use super::values::{LengthAuto, LengthPercentage, ValueContext, length, length_percentage};
use super::{ColorContext, DecorationLines, DecorationStyle, Rgba};
use crate::style::StyleDeclaration;

/// A value per box side.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sides<T> {
    pub top: T,
    pub right: T,
    pub bottom: T,
    pub left: T,
}

impl<T: Copy> Sides<T> {
    #[must_use]
    pub const fn all(value: T) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }
}

/// A value per box corner.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Corners<T> {
    pub top_left: T,
    pub top_right: T,
    pub bottom_right: T,
    pub bottom_left: T,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Display {
    None,
    Contents,
    Block,
    Inline,
    InlineBlock,
    FlowRoot,
    Flex,
    InlineFlex,
    Grid,
    InlineGrid,
    /// Tables and ruby, which have no layout of their own here.
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Position {
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

/// A box dimension.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    Auto,
    /// `max-width: none`.
    None,
    Length(LengthPercentage),
    MinContent,
    MaxContent,
    FitContent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlexDirection {
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlexWrap {
    NoWrap,
    Wrap,
    WrapReverse,
}

/// Alignment of items within their area (`align-items`, `align-self`,
/// `justify-items`, `justify-self`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Align {
    Auto,
    Normal,
    Stretch,
    Baseline,
    LastBaseline,
    Center,
    Start,
    End,
    SelfStart,
    SelfEnd,
    FlexStart,
    FlexEnd,
    Left,
    Right,
}

/// Distribution of content (`justify-content`, `align-content`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Distribute {
    Normal,
    Start,
    End,
    FlexStart,
    FlexEnd,
    Center,
    Left,
    Right,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
    Stretch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Overflow {
    Visible,
    Hidden,
    Clip,
    Scroll,
    Auto,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BorderStyle {
    None,
    Hidden,
    Solid,
    Dashed,
    Dotted,
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxShadow {
    pub color: Rgba,
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    pub inset: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoxSizing {
    ContentBox,
    BorderBox,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Visibility {
    Visible,
    Hidden,
    Collapse,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FontFamily {
    Named(CompactString),
    Serif,
    SansSerif,
    Monospace,
    Cursive,
    Fantasy,
    SystemUi,
    UiSerif,
    UiSansSerif,
    UiMonospace,
    UiRounded,
    Emoji,
    Math,
    Fangsong,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontStyle {
    Normal,
    Italic,
    Oblique,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineHeight {
    Normal,
    /// A multiple of the font size.
    Number(f32),
    /// Lengths and percentages compute to pixels.
    Px(f32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextAlign {
    Start,
    End,
    Left,
    Right,
    Center,
    Justify,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WhiteSpace {
    Normal,
    Pre,
    NoWrap,
    PreWrap,
    PreLine,
    BreakSpaces,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextOverflow {
    Clip,
    Ellipsis,
}

/// `text-wrap-mode` and `text-wrap-style`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextWrap {
    Wrap,
    NoWrap,
    Balance,
    Pretty,
    Stable,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DecorationThickness {
    Auto,
    FromFont,
    Px(f32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cursor {
    Auto,
    Default,
    None,
    ContextMenu,
    Help,
    Pointer,
    Progress,
    Wait,
    Cell,
    Crosshair,
    Text,
    VerticalText,
    Alias,
    Copy,
    Move,
    NoDrop,
    NotAllowed,
    Grab,
    Grabbing,
    EwResize,
    NsResize,
    NeswResize,
    NwseResize,
    ColResize,
    RowResize,
    AllScroll,
    ZoomIn,
    ZoomOut,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrackBreadth {
    Length(LengthPercentage),
    Flex(f32),
    MinContent,
    MaxContent,
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrackSize {
    Breadth(TrackBreadth),
    MinMax(TrackBreadth, TrackBreadth),
    FitContent(LengthPercentage),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepeatCount {
    Count(u16),
    AutoFill,
    AutoFit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Track {
    Size(TrackSize),
    Repeat {
        count: RepeatCount,
        tracks: Vec<TrackSize>,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GridAutoFlow {
    pub column: bool,
    pub dense: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GridLine {
    Auto,
    Line(i16),
    Span(u16),
}

/// Why a declaration contributed nothing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Unsupported {
    /// The property is not part of the computed model.
    Property,
    /// The value does not parse for the property.
    Invalid,
    /// The value parses but uses a feature the model does not represent,
    /// described by the message.
    Value(&'static str),
}

/// The typed styles an element's declarations set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ComputedStyle {
    pub display: Option<Display>,
    pub position: Option<Position>,
    pub inset: Sides<Option<LengthAuto>>,
    pub width: Option<Size>,
    pub height: Option<Size>,
    pub min_width: Option<Size>,
    pub min_height: Option<Size>,
    pub max_width: Option<Size>,
    pub max_height: Option<Size>,
    pub aspect_ratio: Option<f32>,
    pub box_sizing: Option<BoxSizing>,
    pub margin: Sides<Option<LengthAuto>>,
    pub padding: Sides<Option<LengthPercentage>>,
    pub flex_direction: Option<FlexDirection>,
    pub flex_wrap: Option<FlexWrap>,
    pub flex_grow: Option<f32>,
    pub flex_shrink: Option<f32>,
    pub flex_basis: Option<Size>,
    pub align_items: Option<Align>,
    pub align_self: Option<Align>,
    pub justify_items: Option<Align>,
    pub justify_self: Option<Align>,
    pub align_content: Option<Distribute>,
    pub justify_content: Option<Distribute>,
    pub row_gap: Option<LengthPercentage>,
    pub column_gap: Option<LengthPercentage>,
    pub grid_template_columns: Option<Vec<Track>>,
    pub grid_template_rows: Option<Vec<Track>>,
    pub grid_auto_columns: Option<Vec<TrackSize>>,
    pub grid_auto_rows: Option<Vec<TrackSize>>,
    pub grid_auto_flow: Option<GridAutoFlow>,
    pub grid_column_start: Option<GridLine>,
    pub grid_column_end: Option<GridLine>,
    pub grid_row_start: Option<GridLine>,
    pub grid_row_end: Option<GridLine>,
    pub overflow_x: Option<Overflow>,
    pub overflow_y: Option<Overflow>,
    pub visibility: Option<Visibility>,
    pub opacity: Option<f32>,
    pub color: Option<Rgba>,
    pub background_color: Option<Rgba>,
    pub border_width: Sides<Option<f32>>,
    pub border_style: Sides<Option<BorderStyle>>,
    pub border_color: Sides<Option<Rgba>>,
    pub border_radius: Corners<Option<LengthPercentage>>,
    pub box_shadow: Option<Vec<BoxShadow>>,
    pub cursor: Option<Cursor>,
    pub translate: Option<(LengthPercentage, LengthPercentage)>,
    pub font_family: Option<Vec<FontFamily>>,
    pub font_size: Option<f32>,
    pub font_weight: Option<f32>,
    pub font_style: Option<FontStyle>,
    pub line_height: Option<LineHeight>,
    pub letter_spacing: Option<f32>,
    pub word_spacing: Option<f32>,
    pub text_align: Option<TextAlign>,
    pub white_space: Option<WhiteSpace>,
    pub text_wrap: Option<TextWrap>,
    pub text_overflow: Option<TextOverflow>,
    /// `line-clamp` / `-webkit-line-clamp`; `Some(None)` is `none`.
    pub line_clamp: Option<Option<u32>>,
    pub text_decoration_line: Option<DecorationLines>,
    pub text_decoration_style: Option<DecorationStyle>,
    pub text_decoration_color: Option<Rgba>,
    pub text_decoration_thickness: Option<DecorationThickness>,
}

/// What a computation resolves against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StyleContext {
    pub values: ValueContext,
    /// The parent's computed `font-size`, which `em` in `font-size` uses.
    pub parent_font_size: f32,
    pub colors: ColorContext,
}

impl ComputedStyle {
    /// Compute every declaration in cascade order, reporting those that
    /// contributed nothing.
    pub fn compute<'a>(
        declarations: impl IntoIterator<Item = &'a StyleDeclaration>,
        context: &StyleContext,
        mut unsupported: impl FnMut(&'a StyleDeclaration, Unsupported),
    ) -> Self {
        let mut style = Self::default();
        let mut important = Vec::new();
        for declaration in declarations {
            if declaration.important {
                important.push(declaration);
                continue;
            }
            if let Err(reason) = style.apply(declaration, context) {
                unsupported(declaration, reason);
            }
        }
        for declaration in important {
            if let Err(reason) = style.apply(declaration, context) {
                unsupported(declaration, reason);
            }
        }
        style
    }

    /// Overlay another style's set fields, as a later state (such as
    /// `:hover`) does.
    pub fn overlay(&mut self, other: &Self) {
        macro_rules! take {
            ($($field:ident),* $(,)?) => {
                $(if other.$field.is_some() { self.$field.clone_from(&other.$field); })*
            };
        }
        macro_rules! sides {
            ($($field:ident),* $(,)?) => {
                $(
                    if other.$field.top.is_some() { self.$field.top = other.$field.top; }
                    if other.$field.right.is_some() { self.$field.right = other.$field.right; }
                    if other.$field.bottom.is_some() { self.$field.bottom = other.$field.bottom; }
                    if other.$field.left.is_some() { self.$field.left = other.$field.left; }
                )*
            };
        }
        take!(
            display,
            position,
            width,
            height,
            min_width,
            min_height,
            max_width,
            max_height,
            aspect_ratio,
            box_sizing,
            flex_direction,
            flex_wrap,
            flex_grow,
            flex_shrink,
            flex_basis,
            align_items,
            align_self,
            justify_items,
            justify_self,
            align_content,
            justify_content,
            row_gap,
            column_gap,
            grid_template_columns,
            grid_template_rows,
            grid_auto_columns,
            grid_auto_rows,
            grid_auto_flow,
            grid_column_start,
            grid_column_end,
            grid_row_start,
            grid_row_end,
            overflow_x,
            overflow_y,
            visibility,
            opacity,
            color,
            background_color,
            box_shadow,
            cursor,
            translate,
            font_family,
            font_size,
            font_weight,
            font_style,
            line_height,
            letter_spacing,
            word_spacing,
            text_align,
            white_space,
            text_wrap,
            text_overflow,
            line_clamp,
            text_decoration_line,
            text_decoration_style,
            text_decoration_color,
            text_decoration_thickness,
        );
        sides!(
            inset,
            margin,
            padding,
            border_width,
            border_style,
            border_color
        );
        let corners = other.border_radius;
        if corners.top_left.is_some() {
            self.border_radius.top_left = corners.top_left;
        }
        if corners.top_right.is_some() {
            self.border_radius.top_right = corners.top_right;
        }
        if corners.bottom_right.is_some() {
            self.border_radius.bottom_right = corners.bottom_right;
        }
        if corners.bottom_left.is_some() {
            self.border_radius.bottom_left = corners.bottom_left;
        }
    }

    /// Apply one declaration.
    ///
    /// # Errors
    ///
    /// Reports why the declaration contributed nothing.
    #[allow(clippy::too_many_lines)]
    pub fn apply(
        &mut self,
        declaration: &StyleDeclaration,
        context: &StyleContext,
    ) -> Result<(), Unsupported> {
        let name = declaration.property.as_str();
        let value = declaration.value.as_str().trim();
        if is_css_wide(value) {
            // An unset field means "inherit" for inherited properties and
            // "initial" for the rest, so each keyword clears what earlier
            // declarations set. Only `inherit` on a non-inherited property
            // has no such representation.
            let inherits = INHERITED.contains(&name);
            if !self.reset(name) {
                return Err(Unsupported::Property);
            }
            if value.eq_ignore_ascii_case("inherit") && !inherits {
                return Err(Unsupported::Value(
                    "inherit on a non-inherited property is treated as initial",
                ));
            }
            return Ok(());
        }
        // Properties lightningcss leaves untyped.
        match name {
            "text-wrap" | "text-wrap-mode" | "text-wrap-style" => {
                self.text_wrap = Some(text_wrap(value).ok_or(Unsupported::Invalid)?);
                return Ok(());
            }
            "line-clamp" | "-webkit-line-clamp" => {
                self.line_clamp = Some(if value.eq_ignore_ascii_case("none") {
                    None
                } else {
                    Some(
                        value
                            .parse::<u32>()
                            .ok()
                            .filter(|lines| *lines > 0)
                            .ok_or(Unsupported::Invalid)?,
                    )
                });
                return Ok(());
            }
            _ => {}
        }
        let values = &context.values;
        let lp = |value: &CssLengthPercentage| {
            length_percentage(value, values).ok_or(Unsupported::Value("unresolvable length"))
        };
        let lpa = |value: &LengthPercentageOrAuto| match value {
            LengthPercentageOrAuto::Auto => Ok(LengthAuto::Auto),
            LengthPercentageOrAuto::LengthPercentage(value) => lp(value).map(LengthAuto::Length),
        };
        let color = |value: &CssColor| {
            super::css_color(value, &context.colors).ok_or(Unsupported::Value("unresolvable color"))
        };
        let property =
            Property::parse_string(PropertyId::from(name), value, ParserOptions::default())
                .map_err(|_| Unsupported::Invalid)?;
        match &property {
            Property::Display(value) => self.display = Some(display(value)),
            Property::Position(value) => self.position = Some(position(value)),
            Property::Top(value) => self.inset.top = Some(lpa(value)?),
            Property::Right(value) => self.inset.right = Some(lpa(value)?),
            Property::Bottom(value) => self.inset.bottom = Some(lpa(value)?),
            Property::Left(value) => self.inset.left = Some(lpa(value)?),
            Property::Inset(value) => {
                self.inset = Sides {
                    top: Some(lpa(&value.top)?),
                    right: Some(lpa(&value.right)?),
                    bottom: Some(lpa(&value.bottom)?),
                    left: Some(lpa(&value.left)?),
                };
            }
            Property::Width(value) => self.width = Some(size(value, values)?),
            Property::Height(value) => self.height = Some(size(value, values)?),
            Property::MinWidth(value) => self.min_width = Some(size(value, values)?),
            Property::MinHeight(value) => self.min_height = Some(size(value, values)?),
            Property::MaxWidth(value) => self.max_width = Some(max_size(value, values)?),
            Property::MaxHeight(value) => self.max_height = Some(max_size(value, values)?),
            Property::AspectRatio(value) => {
                self.aspect_ratio = value.ratio.as_ref().map(|ratio| ratio.0 / ratio.1);
            }
            Property::BoxSizing(value, _) => {
                self.box_sizing = Some(match value {
                    CssBoxSizing::ContentBox => BoxSizing::ContentBox,
                    CssBoxSizing::BorderBox => BoxSizing::BorderBox,
                });
            }
            Property::Margin(value) => {
                self.margin = Sides {
                    top: Some(lpa(&value.top)?),
                    right: Some(lpa(&value.right)?),
                    bottom: Some(lpa(&value.bottom)?),
                    left: Some(lpa(&value.left)?),
                };
            }
            Property::MarginTop(value) => self.margin.top = Some(lpa(value)?),
            Property::MarginRight(value) => self.margin.right = Some(lpa(value)?),
            Property::MarginBottom(value) => self.margin.bottom = Some(lpa(value)?),
            Property::MarginLeft(value) => self.margin.left = Some(lpa(value)?),
            Property::Padding(value) => {
                self.padding = Sides {
                    top: Some(padding(&value.top, values)?),
                    right: Some(padding(&value.right, values)?),
                    bottom: Some(padding(&value.bottom, values)?),
                    left: Some(padding(&value.left, values)?),
                };
            }
            Property::PaddingTop(value) => self.padding.top = Some(padding(value, values)?),
            Property::PaddingRight(value) => self.padding.right = Some(padding(value, values)?),
            Property::PaddingBottom(value) => self.padding.bottom = Some(padding(value, values)?),
            Property::PaddingLeft(value) => self.padding.left = Some(padding(value, values)?),
            Property::FlexDirection(value, _) => {
                self.flex_direction = Some(match value {
                    CssFlexDirection::Row => FlexDirection::Row,
                    CssFlexDirection::RowReverse => FlexDirection::RowReverse,
                    CssFlexDirection::Column => FlexDirection::Column,
                    CssFlexDirection::ColumnReverse => FlexDirection::ColumnReverse,
                });
            }
            Property::FlexWrap(value, _) => self.flex_wrap = Some(flex_wrap(*value)),
            Property::FlexFlow(value, _) => {
                self.flex_direction = Some(match value.direction {
                    CssFlexDirection::Row => FlexDirection::Row,
                    CssFlexDirection::RowReverse => FlexDirection::RowReverse,
                    CssFlexDirection::Column => FlexDirection::Column,
                    CssFlexDirection::ColumnReverse => FlexDirection::ColumnReverse,
                });
                self.flex_wrap = Some(flex_wrap(value.wrap));
            }
            Property::Flex(value, _) => {
                self.flex_grow = Some(value.grow);
                self.flex_shrink = Some(value.shrink);
                self.flex_basis = Some(match &value.basis {
                    LengthPercentageOrAuto::Auto => Size::Auto,
                    LengthPercentageOrAuto::LengthPercentage(value) => Size::Length(lp(value)?),
                });
            }
            Property::FlexGrow(value, _) => self.flex_grow = Some(*value),
            Property::FlexShrink(value, _) => self.flex_shrink = Some(*value),
            Property::FlexBasis(value, _) => {
                self.flex_basis = Some(match value {
                    LengthPercentageOrAuto::Auto => Size::Auto,
                    LengthPercentageOrAuto::LengthPercentage(value) => Size::Length(lp(value)?),
                });
            }
            Property::AlignItems(value, _) => self.align_items = Some(align_items(value)),
            Property::AlignSelf(value, _) => self.align_self = Some(align_self(value)),
            Property::JustifyItems(value) => self.justify_items = Some(justify_items(value)),
            Property::JustifySelf(value) => self.justify_self = Some(justify_self(value)),
            Property::AlignContent(value, _) => self.align_content = Some(align_content(value)),
            Property::JustifyContent(value, _) => {
                self.justify_content = Some(justify_content(value));
            }
            Property::PlaceItems(value) => {
                self.align_items = Some(align_items(&value.align));
                self.justify_items = Some(justify_items(&value.justify));
            }
            Property::PlaceSelf(value) => {
                self.align_self = Some(align_self(&value.align));
                self.justify_self = Some(justify_self(&value.justify));
            }
            Property::PlaceContent(value) => {
                self.align_content = Some(align_content(&value.align));
                self.justify_content = Some(justify_content(&value.justify));
            }
            Property::Gap(value) => {
                self.row_gap = Some(gap(&value.row, values)?);
                self.column_gap = Some(gap(&value.column, values)?);
            }
            Property::RowGap(value) => self.row_gap = Some(gap(value, values)?),
            Property::ColumnGap(value) => self.column_gap = Some(gap(value, values)?),
            Property::GridTemplateColumns(value) => {
                self.grid_template_columns = Some(track_sizing(value, values)?);
            }
            Property::GridTemplateRows(value) => {
                self.grid_template_rows = Some(track_sizing(value, values)?);
            }
            Property::GridAutoColumns(value) => {
                self.grid_auto_columns = Some(track_list(&value.0, values)?);
            }
            Property::GridAutoRows(value) => {
                self.grid_auto_rows = Some(track_list(&value.0, values)?);
            }
            Property::GridAutoFlow(value) => {
                self.grid_auto_flow = Some(GridAutoFlow {
                    column: value.contains(CssGridAutoFlow::Column),
                    dense: value.contains(CssGridAutoFlow::Dense),
                });
            }
            Property::GridColumn(value) => {
                self.grid_column_start = Some(grid_line(&value.start)?);
                self.grid_column_end = Some(grid_line(&value.end)?);
            }
            Property::GridRow(value) => {
                self.grid_row_start = Some(grid_line(&value.start)?);
                self.grid_row_end = Some(grid_line(&value.end)?);
            }
            Property::GridColumnStart(value) => self.grid_column_start = Some(grid_line(value)?),
            Property::GridColumnEnd(value) => self.grid_column_end = Some(grid_line(value)?),
            Property::GridRowStart(value) => self.grid_row_start = Some(grid_line(value)?),
            Property::GridRowEnd(value) => self.grid_row_end = Some(grid_line(value)?),
            Property::Overflow(value) => {
                self.overflow_x = Some(overflow(value.x));
                self.overflow_y = Some(overflow(value.y));
            }
            Property::OverflowX(value) => self.overflow_x = Some(overflow(*value)),
            Property::OverflowY(value) => self.overflow_y = Some(overflow(*value)),
            Property::Visibility(value) => {
                self.visibility = Some(match value {
                    CssVisibility::Visible => Visibility::Visible,
                    CssVisibility::Hidden => Visibility::Hidden,
                    CssVisibility::Collapse => Visibility::Collapse,
                });
            }
            Property::Opacity(value) => self.opacity = Some(value.0.clamp(0.0, 1.0)),
            Property::Color(value) => self.color = Some(color(value)?),
            Property::BackgroundColor(value) => self.background_color = Some(color(value)?),
            Property::Background(layers) => {
                // Only the bottom layer has a color; images are not modeled.
                let has_image = layers
                    .iter()
                    .any(|layer| !matches!(layer.image, lightningcss::values::image::Image::None));
                if has_image {
                    return Err(Unsupported::Value(
                        "background images and gradients are not supported",
                    ));
                }
                let layer = layers.last().ok_or(Unsupported::Invalid)?;
                self.background_color = Some(color(&layer.color)?);
            }
            Property::BorderWidth(value) => {
                self.border_width = Sides {
                    top: Some(border_width(&value.top, values)?),
                    right: Some(border_width(&value.right, values)?),
                    bottom: Some(border_width(&value.bottom, values)?),
                    left: Some(border_width(&value.left, values)?),
                };
            }
            Property::BorderTopWidth(value) => {
                self.border_width.top = Some(border_width(value, values)?)
            }
            Property::BorderRightWidth(value) => {
                self.border_width.right = Some(border_width(value, values)?)
            }
            Property::BorderBottomWidth(value) => {
                self.border_width.bottom = Some(border_width(value, values)?)
            }
            Property::BorderLeftWidth(value) => {
                self.border_width.left = Some(border_width(value, values)?)
            }
            Property::BorderStyle(value) => {
                self.border_style = Sides {
                    top: Some(border_style(value.top)),
                    right: Some(border_style(value.right)),
                    bottom: Some(border_style(value.bottom)),
                    left: Some(border_style(value.left)),
                };
            }
            Property::BorderTopStyle(value) => self.border_style.top = Some(border_style(*value)),
            Property::BorderRightStyle(value) => {
                self.border_style.right = Some(border_style(*value))
            }
            Property::BorderBottomStyle(value) => {
                self.border_style.bottom = Some(border_style(*value))
            }
            Property::BorderLeftStyle(value) => self.border_style.left = Some(border_style(*value)),
            Property::BorderColor(value) => {
                self.border_color = Sides {
                    top: Some(color(&value.top)?),
                    right: Some(color(&value.right)?),
                    bottom: Some(color(&value.bottom)?),
                    left: Some(color(&value.left)?),
                };
            }
            Property::BorderTopColor(value) => self.border_color.top = Some(color(value)?),
            Property::BorderRightColor(value) => self.border_color.right = Some(color(value)?),
            Property::BorderBottomColor(value) => self.border_color.bottom = Some(color(value)?),
            Property::BorderLeftColor(value) => self.border_color.left = Some(color(value)?),
            Property::Border(value) => {
                let width = border_width(&value.width, values)?;
                let style = border_style(value.style);
                let paint = color(&value.color)?;
                self.border_width = Sides::all(Some(width));
                self.border_style = Sides::all(Some(style));
                self.border_color = Sides::all(Some(paint));
            }
            Property::BorderTop(value) => {
                self.border_width.top = Some(border_width(&value.width, values)?);
                self.border_style.top = Some(border_style(value.style));
                self.border_color.top = Some(color(&value.color)?);
            }
            Property::BorderRight(value) => {
                self.border_width.right = Some(border_width(&value.width, values)?);
                self.border_style.right = Some(border_style(value.style));
                self.border_color.right = Some(color(&value.color)?);
            }
            Property::BorderBottom(value) => {
                self.border_width.bottom = Some(border_width(&value.width, values)?);
                self.border_style.bottom = Some(border_style(value.style));
                self.border_color.bottom = Some(color(&value.color)?);
            }
            Property::BorderLeft(value) => {
                self.border_width.left = Some(border_width(&value.width, values)?);
                self.border_style.left = Some(border_style(value.style));
                self.border_color.left = Some(color(&value.color)?);
            }
            Property::BorderRadius(value, _) => {
                self.border_radius = Corners {
                    top_left: Some(radius(&value.top_left, values)?),
                    top_right: Some(radius(&value.top_right, values)?),
                    bottom_right: Some(radius(&value.bottom_right, values)?),
                    bottom_left: Some(radius(&value.bottom_left, values)?),
                };
            }
            Property::BorderTopLeftRadius(value, _) => {
                self.border_radius.top_left = Some(radius(value, values)?);
            }
            Property::BorderTopRightRadius(value, _) => {
                self.border_radius.top_right = Some(radius(value, values)?);
            }
            Property::BorderBottomRightRadius(value, _) => {
                self.border_radius.bottom_right = Some(radius(value, values)?);
            }
            Property::BorderBottomLeftRadius(value, _) => {
                self.border_radius.bottom_left = Some(radius(value, values)?);
            }
            Property::BoxShadow(shadows, _) => {
                self.box_shadow = Some(
                    shadows
                        .iter()
                        .map(|shadow| box_shadow(shadow, context))
                        .collect::<Result<_, _>>()?,
                );
            }
            Property::Cursor(value) => self.cursor = Some(cursor(value)?),
            Property::Translate(value) => {
                self.translate = Some(match value {
                    Translate::None => (LengthPercentage::ZERO, LengthPercentage::ZERO),
                    Translate::XYZ { x, y, z } => {
                        if length(z, values).is_some_and(|z| z != 0.0) {
                            return Err(Unsupported::Value("3D translation is not supported"));
                        }
                        (lp(x)?, lp(y)?)
                    }
                });
            }
            Property::Font(font) => {
                // The shorthand resets each longhand it covers.
                self.font_family = Some(font.family.iter().map(font_family).collect());
                self.font_size = Some(font_size(&font.size, context.parent_font_size, values)?);
                self.font_style = Some(font_style(&font.style));
                self.font_weight = Some(font_weight(&font.weight)?);
                self.line_height = Some(line_height(&font.line_height, values)?);
            }
            Property::FontFamily(families) => {
                self.font_family = Some(families.iter().map(font_family).collect());
            }
            Property::FontSize(value) => {
                self.font_size = Some(font_size(value, context.parent_font_size, values)?);
            }
            Property::FontWeight(value) => self.font_weight = Some(font_weight(value)?),
            Property::FontStyle(value) => self.font_style = Some(font_style(value)),
            Property::LineHeight(value) => self.line_height = Some(line_height(value, values)?),
            Property::LetterSpacing(value) => self.letter_spacing = Some(spacing(value, values)?),
            Property::WordSpacing(value) => self.word_spacing = Some(spacing(value, values)?),
            Property::TextAlign(value) => {
                self.text_align = Some(match value {
                    CssTextAlign::Start | CssTextAlign::MatchParent => TextAlign::Start,
                    CssTextAlign::End => TextAlign::End,
                    CssTextAlign::Left => TextAlign::Left,
                    CssTextAlign::Right => TextAlign::Right,
                    CssTextAlign::Center => TextAlign::Center,
                    CssTextAlign::Justify | CssTextAlign::JustifyAll => TextAlign::Justify,
                });
            }
            Property::WhiteSpace(value) => {
                self.white_space = Some(match value {
                    CssWhiteSpace::Normal => WhiteSpace::Normal,
                    CssWhiteSpace::Pre => WhiteSpace::Pre,
                    CssWhiteSpace::NoWrap => WhiteSpace::NoWrap,
                    CssWhiteSpace::PreWrap => WhiteSpace::PreWrap,
                    CssWhiteSpace::PreLine => WhiteSpace::PreLine,
                    CssWhiteSpace::BreakSpaces => WhiteSpace::BreakSpaces,
                });
            }
            Property::TextOverflow(value, _) => {
                self.text_overflow = Some(match value {
                    CssTextOverflow::Clip => TextOverflow::Clip,
                    CssTextOverflow::Ellipsis => TextOverflow::Ellipsis,
                });
            }
            Property::TextDecoration(value, _) => {
                self.text_decoration_line = Some(decoration_lines(value.line));
                self.text_decoration_style = Some(decoration_style(value.style));
                self.text_decoration_color = Some(color(&value.color)?);
                self.text_decoration_thickness = Some(thickness(&value.thickness, values)?);
            }
            Property::TextDecorationLine(value, _) => {
                self.text_decoration_line = Some(decoration_lines(*value));
            }
            Property::TextDecorationStyle(value, _) => {
                self.text_decoration_style = Some(decoration_style(*value));
            }
            Property::TextDecorationColor(value, _) => {
                self.text_decoration_color = Some(color(value)?);
            }
            Property::TextDecorationThickness(value) => {
                self.text_decoration_thickness = Some(thickness(value, values)?);
            }
            Property::Unparsed(_) | Property::Custom(_) => return Err(Unsupported::Property),
            _ => return Err(Unsupported::Property),
        }
        Ok(())
    }
}

/// Inherited properties in the model.
const INHERITED: &[&str] = &[
    "color",
    "cursor",
    "visibility",
    "font",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "line-height",
    "letter-spacing",
    "word-spacing",
    "text-align",
    "white-space",
    "text-wrap",
    "text-wrap-mode",
    "text-wrap-style",
];

impl ComputedStyle {
    /// Clear what a property sets, returning whether it is in the model.
    #[allow(clippy::too_many_lines)]
    fn reset(&mut self, property: &str) -> bool {
        match property {
            "display" => self.display = None,
            "position" => self.position = None,
            "inset" => self.inset = Sides::all(None),
            "top" => self.inset.top = None,
            "right" => self.inset.right = None,
            "bottom" => self.inset.bottom = None,
            "left" => self.inset.left = None,
            "width" => self.width = None,
            "height" => self.height = None,
            "min-width" => self.min_width = None,
            "min-height" => self.min_height = None,
            "max-width" => self.max_width = None,
            "max-height" => self.max_height = None,
            "aspect-ratio" => self.aspect_ratio = None,
            "box-sizing" => self.box_sizing = None,
            "margin" => self.margin = Sides::all(None),
            "margin-top" => self.margin.top = None,
            "margin-right" => self.margin.right = None,
            "margin-bottom" => self.margin.bottom = None,
            "margin-left" => self.margin.left = None,
            "padding" => self.padding = Sides::all(None),
            "padding-top" => self.padding.top = None,
            "padding-right" => self.padding.right = None,
            "padding-bottom" => self.padding.bottom = None,
            "padding-left" => self.padding.left = None,
            "flex-direction" => self.flex_direction = None,
            "flex-wrap" => self.flex_wrap = None,
            "flex-flow" => {
                self.flex_direction = None;
                self.flex_wrap = None;
            }
            "flex" => {
                self.flex_grow = None;
                self.flex_shrink = None;
                self.flex_basis = None;
            }
            "flex-grow" => self.flex_grow = None,
            "flex-shrink" => self.flex_shrink = None,
            "flex-basis" => self.flex_basis = None,
            "align-items" => self.align_items = None,
            "align-self" => self.align_self = None,
            "justify-items" => self.justify_items = None,
            "justify-self" => self.justify_self = None,
            "align-content" => self.align_content = None,
            "justify-content" => self.justify_content = None,
            "place-items" => {
                self.align_items = None;
                self.justify_items = None;
            }
            "place-self" => {
                self.align_self = None;
                self.justify_self = None;
            }
            "place-content" => {
                self.align_content = None;
                self.justify_content = None;
            }
            "gap" => {
                self.row_gap = None;
                self.column_gap = None;
            }
            "row-gap" => self.row_gap = None,
            "column-gap" => self.column_gap = None,
            "grid-template-columns" => self.grid_template_columns = None,
            "grid-template-rows" => self.grid_template_rows = None,
            "grid-auto-columns" => self.grid_auto_columns = None,
            "grid-auto-rows" => self.grid_auto_rows = None,
            "grid-auto-flow" => self.grid_auto_flow = None,
            "grid-column" => {
                self.grid_column_start = None;
                self.grid_column_end = None;
            }
            "grid-row" => {
                self.grid_row_start = None;
                self.grid_row_end = None;
            }
            "grid-column-start" => self.grid_column_start = None,
            "grid-column-end" => self.grid_column_end = None,
            "grid-row-start" => self.grid_row_start = None,
            "grid-row-end" => self.grid_row_end = None,
            "overflow" => {
                self.overflow_x = None;
                self.overflow_y = None;
            }
            "overflow-x" => self.overflow_x = None,
            "overflow-y" => self.overflow_y = None,
            "visibility" => self.visibility = None,
            "opacity" => self.opacity = None,
            "color" => self.color = None,
            "background" | "background-color" => self.background_color = None,
            "border" => {
                self.border_width = Sides::all(None);
                self.border_style = Sides::all(None);
                self.border_color = Sides::all(None);
            }
            "border-width" => self.border_width = Sides::all(None),
            "border-style" => self.border_style = Sides::all(None),
            "border-color" => self.border_color = Sides::all(None),
            "border-top" | "border-right" | "border-bottom" | "border-left" => {
                match &property["border-".len()..] {
                    "top" => {
                        self.border_width.top = None;
                        self.border_style.top = None;
                        self.border_color.top = None;
                    }
                    "right" => {
                        self.border_width.right = None;
                        self.border_style.right = None;
                        self.border_color.right = None;
                    }
                    "bottom" => {
                        self.border_width.bottom = None;
                        self.border_style.bottom = None;
                        self.border_color.bottom = None;
                    }
                    _ => {
                        self.border_width.left = None;
                        self.border_style.left = None;
                        self.border_color.left = None;
                    }
                }
            }
            "border-top-width" => self.border_width.top = None,
            "border-right-width" => self.border_width.right = None,
            "border-bottom-width" => self.border_width.bottom = None,
            "border-left-width" => self.border_width.left = None,
            "border-top-style" => self.border_style.top = None,
            "border-right-style" => self.border_style.right = None,
            "border-bottom-style" => self.border_style.bottom = None,
            "border-left-style" => self.border_style.left = None,
            "border-top-color" => self.border_color.top = None,
            "border-right-color" => self.border_color.right = None,
            "border-bottom-color" => self.border_color.bottom = None,
            "border-left-color" => self.border_color.left = None,
            "border-radius" => self.border_radius = Corners::default(),
            "border-top-left-radius" => self.border_radius.top_left = None,
            "border-top-right-radius" => self.border_radius.top_right = None,
            "border-bottom-right-radius" => self.border_radius.bottom_right = None,
            "border-bottom-left-radius" => self.border_radius.bottom_left = None,
            "box-shadow" => self.box_shadow = None,
            "cursor" => self.cursor = None,
            "translate" => self.translate = None,
            "font" => {
                self.font_family = None;
                self.font_size = None;
                self.font_weight = None;
                self.font_style = None;
                self.line_height = None;
            }
            "font-family" => self.font_family = None,
            "font-size" => self.font_size = None,
            "font-weight" => self.font_weight = None,
            "font-style" => self.font_style = None,
            "line-height" => self.line_height = None,
            "letter-spacing" => self.letter_spacing = None,
            "word-spacing" => self.word_spacing = None,
            "text-align" => self.text_align = None,
            "white-space" => self.white_space = None,
            "text-wrap" | "text-wrap-mode" | "text-wrap-style" => self.text_wrap = None,
            "text-overflow" => self.text_overflow = None,
            "line-clamp" | "-webkit-line-clamp" => self.line_clamp = None,
            "text-decoration" => {
                self.text_decoration_line = None;
                self.text_decoration_style = None;
                self.text_decoration_color = None;
                self.text_decoration_thickness = None;
            }
            "text-decoration-line" => self.text_decoration_line = None,
            "text-decoration-style" => self.text_decoration_style = None,
            "text-decoration-color" => self.text_decoration_color = None,
            "text-decoration-thickness" => self.text_decoration_thickness = None,
            _ => return false,
        }
        true
    }
}

fn is_css_wide(value: &str) -> bool {
    ["initial", "inherit", "unset", "revert", "revert-layer"]
        .iter()
        .any(|keyword| value.eq_ignore_ascii_case(keyword))
}

fn text_wrap(value: &str) -> Option<TextWrap> {
    let mut result = None;
    for word in value.split_whitespace() {
        result = Some(match word.to_ascii_lowercase().as_str() {
            "wrap" => TextWrap::Wrap,
            "nowrap" => TextWrap::NoWrap,
            "balance" => TextWrap::Balance,
            "pretty" => TextWrap::Pretty,
            "stable" => TextWrap::Stable,
            "auto" => result.unwrap_or(TextWrap::Wrap),
            _ => return None,
        });
    }
    result
}

fn display(value: &CssDisplay) -> Display {
    match value {
        CssDisplay::Keyword(DisplayKeyword::None) => Display::None,
        CssDisplay::Keyword(DisplayKeyword::Contents) => Display::Contents,
        CssDisplay::Keyword(_) => Display::Other,
        CssDisplay::Pair(pair) => match (&pair.outside, &pair.inside) {
            (DisplayOutside::Block, DisplayInside::Flow) => Display::Block,
            (DisplayOutside::Inline, DisplayInside::Flow) => Display::Inline,
            (DisplayOutside::Inline, DisplayInside::FlowRoot) => Display::InlineBlock,
            (_, DisplayInside::FlowRoot) => Display::FlowRoot,
            (DisplayOutside::Inline, DisplayInside::Flex(_) | DisplayInside::Box(_)) => {
                Display::InlineFlex
            }
            (_, DisplayInside::Flex(_) | DisplayInside::Box(_)) => Display::Flex,
            (DisplayOutside::Inline, DisplayInside::Grid) => Display::InlineGrid,
            (_, DisplayInside::Grid) => Display::Grid,
            _ => Display::Other,
        },
    }
}

fn position(value: &CssPosition) -> Position {
    match value {
        CssPosition::Static => Position::Static,
        CssPosition::Relative => Position::Relative,
        CssPosition::Absolute => Position::Absolute,
        CssPosition::Fixed => Position::Fixed,
        CssPosition::Sticky(_) => Position::Sticky,
    }
}

fn size(value: &CssSize, context: &ValueContext) -> Result<Size, Unsupported> {
    Ok(match value {
        CssSize::Auto => Size::Auto,
        CssSize::LengthPercentage(value) => Size::Length(
            length_percentage(value, context).ok_or(Unsupported::Value("unresolvable length"))?,
        ),
        CssSize::MinContent(_) => Size::MinContent,
        CssSize::MaxContent(_) => Size::MaxContent,
        CssSize::FitContent(_) => Size::FitContent,
        CssSize::FitContentFunction(_) | CssSize::Stretch(_) | CssSize::Contain => {
            return Err(Unsupported::Value(
                "fit-content(), stretch and contain sizes are not supported",
            ));
        }
    })
}

fn max_size(value: &MaxSize, context: &ValueContext) -> Result<Size, Unsupported> {
    Ok(match value {
        MaxSize::None => Size::None,
        MaxSize::LengthPercentage(value) => Size::Length(
            length_percentage(value, context).ok_or(Unsupported::Value("unresolvable length"))?,
        ),
        MaxSize::MinContent(_) => Size::MinContent,
        MaxSize::MaxContent(_) => Size::MaxContent,
        MaxSize::FitContent(_) => Size::FitContent,
        MaxSize::FitContentFunction(_) | MaxSize::Stretch(_) | MaxSize::Contain => {
            return Err(Unsupported::Value(
                "fit-content(), stretch and contain sizes are not supported",
            ));
        }
    })
}

fn padding(
    value: &LengthPercentageOrAuto,
    context: &ValueContext,
) -> Result<LengthPercentage, Unsupported> {
    match value {
        LengthPercentageOrAuto::Auto => Err(Unsupported::Invalid),
        LengthPercentageOrAuto::LengthPercentage(value) => {
            length_percentage(value, context).ok_or(Unsupported::Value("unresolvable length"))
        }
    }
}

const fn flex_wrap(value: CssFlexWrap) -> FlexWrap {
    match value {
        CssFlexWrap::NoWrap => FlexWrap::NoWrap,
        CssFlexWrap::Wrap => FlexWrap::Wrap,
        CssFlexWrap::WrapReverse => FlexWrap::WrapReverse,
    }
}

const fn self_position(value: &SelfPosition) -> Align {
    match value {
        SelfPosition::Center => Align::Center,
        SelfPosition::Start => Align::Start,
        SelfPosition::End => Align::End,
        SelfPosition::SelfStart => Align::SelfStart,
        SelfPosition::SelfEnd => Align::SelfEnd,
        SelfPosition::FlexStart => Align::FlexStart,
        SelfPosition::FlexEnd => Align::FlexEnd,
    }
}

const fn baseline(value: &BaselinePosition) -> Align {
    match value {
        BaselinePosition::First => Align::Baseline,
        BaselinePosition::Last => Align::LastBaseline,
    }
}

fn align_items(value: &CssAlignItems) -> Align {
    match value {
        CssAlignItems::Normal => Align::Normal,
        CssAlignItems::Stretch => Align::Stretch,
        CssAlignItems::BaselinePosition(value) => baseline(value),
        CssAlignItems::SelfPosition { value, .. } => self_position(value),
    }
}

fn align_self(value: &CssAlignSelf) -> Align {
    match value {
        CssAlignSelf::Auto => Align::Auto,
        CssAlignSelf::Normal => Align::Normal,
        CssAlignSelf::Stretch => Align::Stretch,
        CssAlignSelf::BaselinePosition(value) => baseline(value),
        CssAlignSelf::SelfPosition { value, .. } => self_position(value),
    }
}

fn justify_items(value: &CssJustifyItems) -> Align {
    match value {
        CssJustifyItems::Normal => Align::Normal,
        CssJustifyItems::Stretch => Align::Stretch,
        CssJustifyItems::BaselinePosition(value) => baseline(value),
        CssJustifyItems::SelfPosition { value, .. } => self_position(value),
        CssJustifyItems::Left { .. } => Align::Left,
        CssJustifyItems::Right { .. } => Align::Right,
        CssJustifyItems::Legacy(_) => Align::Normal,
    }
}

fn justify_self(value: &CssJustifySelf) -> Align {
    match value {
        CssJustifySelf::Auto => Align::Auto,
        CssJustifySelf::Normal => Align::Normal,
        CssJustifySelf::Stretch => Align::Stretch,
        CssJustifySelf::BaselinePosition(value) => baseline(value),
        CssJustifySelf::SelfPosition { value, .. } => self_position(value),
        CssJustifySelf::Left { .. } => Align::Left,
        CssJustifySelf::Right { .. } => Align::Right,
    }
}

const fn distribution(value: &ContentDistribution) -> Distribute {
    match value {
        ContentDistribution::SpaceBetween => Distribute::SpaceBetween,
        ContentDistribution::SpaceAround => Distribute::SpaceAround,
        ContentDistribution::SpaceEvenly => Distribute::SpaceEvenly,
        ContentDistribution::Stretch => Distribute::Stretch,
    }
}

const fn content_position(value: &ContentPosition) -> Distribute {
    match value {
        ContentPosition::Center => Distribute::Center,
        ContentPosition::Start => Distribute::Start,
        ContentPosition::End => Distribute::End,
        ContentPosition::FlexStart => Distribute::FlexStart,
        ContentPosition::FlexEnd => Distribute::FlexEnd,
    }
}

fn align_content(value: &CssAlignContent) -> Distribute {
    match value {
        CssAlignContent::Normal => Distribute::Normal,
        // Baseline content alignment falls back to start.
        CssAlignContent::BaselinePosition(_) => Distribute::Start,
        CssAlignContent::ContentDistribution(value) => distribution(value),
        CssAlignContent::ContentPosition { value, .. } => content_position(value),
    }
}

fn justify_content(value: &CssJustifyContent) -> Distribute {
    match value {
        CssJustifyContent::Normal => Distribute::Normal,
        CssJustifyContent::ContentDistribution(value) => distribution(value),
        CssJustifyContent::ContentPosition { value, .. } => content_position(value),
        CssJustifyContent::Left { .. } => Distribute::Left,
        CssJustifyContent::Right { .. } => Distribute::Right,
    }
}

fn gap(value: &GapValue, context: &ValueContext) -> Result<LengthPercentage, Unsupported> {
    match value {
        // `normal` is 0 in flex and grid layout.
        GapValue::Normal => Ok(LengthPercentage::ZERO),
        GapValue::LengthPercentage(value) => {
            length_percentage(value, context).ok_or(Unsupported::Value("unresolvable length"))
        }
    }
}

fn track_breadth(
    value: &CssTrackBreadth,
    context: &ValueContext,
) -> Result<TrackBreadth, Unsupported> {
    Ok(match value {
        CssTrackBreadth::Length(value) => TrackBreadth::Length(
            length_percentage(value, context).ok_or(Unsupported::Value("unresolvable length"))?,
        ),
        CssTrackBreadth::Flex(value) => TrackBreadth::Flex(*value),
        CssTrackBreadth::MinContent => TrackBreadth::MinContent,
        CssTrackBreadth::MaxContent => TrackBreadth::MaxContent,
        CssTrackBreadth::Auto => TrackBreadth::Auto,
    })
}

fn track_size(value: &CssTrackSize, context: &ValueContext) -> Result<TrackSize, Unsupported> {
    Ok(match value {
        CssTrackSize::TrackBreadth(value) => TrackSize::Breadth(track_breadth(value, context)?),
        CssTrackSize::MinMax { min, max } => {
            TrackSize::MinMax(track_breadth(min, context)?, track_breadth(max, context)?)
        }
        CssTrackSize::FitContent(value) => TrackSize::FitContent(
            length_percentage(value, context).ok_or(Unsupported::Value("unresolvable length"))?,
        ),
    })
}

fn track_list(
    values: &[CssTrackSize],
    context: &ValueContext,
) -> Result<Vec<TrackSize>, Unsupported> {
    values
        .iter()
        .map(|value| track_size(value, context))
        .collect()
}

fn track_sizing(
    value: &TrackSizing<'_>,
    context: &ValueContext,
) -> Result<Vec<Track>, Unsupported> {
    let TrackSizing::TrackList(list) = value else {
        return Ok(Vec::new());
    };
    if list.line_names.iter().any(|names| !names.is_empty()) {
        return Err(Unsupported::Value("named grid lines are not supported"));
    }
    list.items
        .iter()
        .map(|item| {
            Ok(match item {
                TrackListItem::TrackSize(size) => Track::Size(track_size(size, context)?),
                TrackListItem::TrackRepeat(repeat) => {
                    if repeat.line_names.iter().any(|names| !names.is_empty()) {
                        return Err(Unsupported::Value("named grid lines are not supported"));
                    }
                    Track::Repeat {
                        count: match repeat.count {
                            CssRepeatCount::Number(count) => RepeatCount::Count(
                                u16::try_from(count).map_err(|_| Unsupported::Invalid)?,
                            ),
                            CssRepeatCount::AutoFill => RepeatCount::AutoFill,
                            CssRepeatCount::AutoFit => RepeatCount::AutoFit,
                        },
                        tracks: track_list(&repeat.track_sizes, context)?,
                    }
                }
            })
        })
        .collect()
}

fn grid_line(value: &CssGridLine<'_>) -> Result<GridLine, Unsupported> {
    Ok(match value {
        CssGridLine::Auto => GridLine::Auto,
        CssGridLine::Line { index, name: None } => {
            GridLine::Line(i16::try_from(*index).map_err(|_| Unsupported::Invalid)?)
        }
        CssGridLine::Span { index, name: None } => {
            GridLine::Span(u16::try_from(*index).map_err(|_| Unsupported::Invalid)?)
        }
        _ => {
            return Err(Unsupported::Value(
                "named grid lines and areas are not supported",
            ));
        }
    })
}

const fn overflow(value: OverflowKeyword) -> Overflow {
    match value {
        OverflowKeyword::Visible => Overflow::Visible,
        OverflowKeyword::Hidden => Overflow::Hidden,
        OverflowKeyword::Clip => Overflow::Clip,
        OverflowKeyword::Scroll => Overflow::Scroll,
        OverflowKeyword::Auto => Overflow::Auto,
    }
}

fn border_width(value: &BorderSideWidth, context: &ValueContext) -> Result<f32, Unsupported> {
    Ok(match value {
        BorderSideWidth::Thin => 1.0,
        BorderSideWidth::Medium => 3.0,
        BorderSideWidth::Thick => 5.0,
        BorderSideWidth::Length(value) => {
            length(value, context).ok_or(Unsupported::Value("unresolvable length"))?
        }
    })
}

const fn border_style(value: LineStyle) -> BorderStyle {
    match value {
        LineStyle::None => BorderStyle::None,
        LineStyle::Hidden => BorderStyle::Hidden,
        LineStyle::Solid => BorderStyle::Solid,
        LineStyle::Dashed => BorderStyle::Dashed,
        LineStyle::Dotted => BorderStyle::Dotted,
        LineStyle::Double => BorderStyle::Double,
        LineStyle::Groove => BorderStyle::Groove,
        LineStyle::Ridge => BorderStyle::Ridge,
        LineStyle::Inset => BorderStyle::Inset,
        LineStyle::Outset => BorderStyle::Outset,
    }
}

fn radius(
    value: &lightningcss::values::size::Size2D<CssLengthPercentage>,
    context: &ValueContext,
) -> Result<LengthPercentage, Unsupported> {
    let horizontal =
        length_percentage(&value.0, context).ok_or(Unsupported::Value("unresolvable length"))?;
    let vertical =
        length_percentage(&value.1, context).ok_or(Unsupported::Value("unresolvable length"))?;
    if horizontal != vertical {
        return Err(Unsupported::Value("elliptical corners are not supported"));
    }
    Ok(horizontal)
}

fn box_shadow(value: &CssBoxShadow, context: &StyleContext) -> Result<BoxShadow, Unsupported> {
    let px =
        |value| length(value, &context.values).ok_or(Unsupported::Value("unresolvable length"));
    Ok(BoxShadow {
        color: super::css_color(&value.color, &context.colors)
            .ok_or(Unsupported::Value("unresolvable color"))?,
        x: px(&value.x_offset)?,
        y: px(&value.y_offset)?,
        blur: px(&value.blur)?,
        spread: px(&value.spread)?,
        inset: value.inset,
    })
}

fn cursor(value: &CssCursor<'_>) -> Result<Cursor, Unsupported> {
    use CursorKeyword as K;
    if !value.images.is_empty() {
        return Err(Unsupported::Value("cursor images are not supported"));
    }
    Ok(match value.keyword {
        K::Auto => Cursor::Auto,
        K::Default => Cursor::Default,
        K::None => Cursor::None,
        K::ContextMenu => Cursor::ContextMenu,
        K::Help => Cursor::Help,
        K::Pointer => Cursor::Pointer,
        K::Progress => Cursor::Progress,
        K::Wait => Cursor::Wait,
        K::Cell => Cursor::Cell,
        K::Crosshair => Cursor::Crosshair,
        K::Text => Cursor::Text,
        K::VerticalText => Cursor::VerticalText,
        K::Alias => Cursor::Alias,
        K::Copy => Cursor::Copy,
        K::Move => Cursor::Move,
        K::NoDrop => Cursor::NoDrop,
        K::NotAllowed => Cursor::NotAllowed,
        K::Grab => Cursor::Grab,
        K::Grabbing => Cursor::Grabbing,
        K::EResize | K::WResize | K::EwResize => Cursor::EwResize,
        K::NResize | K::SResize | K::NsResize => Cursor::NsResize,
        K::NeResize | K::SwResize | K::NeswResize => Cursor::NeswResize,
        K::NwResize | K::SeResize | K::NwseResize => Cursor::NwseResize,
        K::ColResize => Cursor::ColResize,
        K::RowResize => Cursor::RowResize,
        K::AllScroll => Cursor::AllScroll,
        K::ZoomIn => Cursor::ZoomIn,
        K::ZoomOut => Cursor::ZoomOut,
    })
}

fn font_family(value: &CssFontFamily<'_>) -> FontFamily {
    match value {
        CssFontFamily::FamilyName(name) => {
            use lightningcss::traits::ToCss;
            let name = name
                .to_css_string(lightningcss::printer::PrinterOptions::default())
                .unwrap_or_default();
            FontFamily::Named(name.trim_matches('"').into())
        }
        CssFontFamily::Generic(generic) => match generic {
            GenericFontFamily::Serif => FontFamily::Serif,
            GenericFontFamily::Monospace => FontFamily::Monospace,
            GenericFontFamily::Cursive => FontFamily::Cursive,
            GenericFontFamily::Fantasy => FontFamily::Fantasy,
            GenericFontFamily::SystemUI => FontFamily::SystemUi,
            GenericFontFamily::UISerif => FontFamily::UiSerif,
            GenericFontFamily::UISansSerif => FontFamily::UiSansSerif,
            GenericFontFamily::UIMonospace => FontFamily::UiMonospace,
            GenericFontFamily::UIRounded => FontFamily::UiRounded,
            GenericFontFamily::Emoji => FontFamily::Emoji,
            GenericFontFamily::Math => FontFamily::Math,
            GenericFontFamily::FangSong => FontFamily::Fangsong,
            _ => FontFamily::SansSerif,
        },
    }
}

/// A computed `font-size` in pixels.
pub(crate) fn font_size(
    value: &FontSize,
    parent: f32,
    context: &ValueContext,
) -> Result<f32, Unsupported> {
    Ok(match value {
        FontSize::Length(value) => {
            // `em` and percentages in font-size refer to the parent.
            let parent_context = ValueContext {
                font_size: parent,
                ..*context
            };
            length_percentage(value, &parent_context)
                .ok_or(Unsupported::Value("unresolvable length"))?
                .resolve(parent)
        }
        FontSize::Absolute(size) => {
            // CSS Fonts 4 §2.5 scale, from a 16px `medium`.
            16.0 * match size {
                AbsoluteFontSize::XXSmall => 0.6,
                AbsoluteFontSize::XSmall => 0.75,
                AbsoluteFontSize::Small => 0.889,
                AbsoluteFontSize::Medium => 1.0,
                AbsoluteFontSize::Large => 1.2,
                AbsoluteFontSize::XLarge => 1.5,
                AbsoluteFontSize::XXLarge => 2.0,
                AbsoluteFontSize::XXXLarge => 3.0,
            }
        }
        FontSize::Relative(RelativeFontSize::Larger) => parent * 1.2,
        FontSize::Relative(RelativeFontSize::Smaller) => parent / 1.2,
    })
}

fn spacing(value: &Spacing, context: &ValueContext) -> Result<f32, Unsupported> {
    match value {
        Spacing::Normal => Ok(0.0),
        Spacing::Length(value) => {
            length(value, context).ok_or(Unsupported::Value("unresolvable length"))
        }
    }
}

fn decoration_lines(value: TextDecorationLine) -> DecorationLines {
    DecorationLines {
        underline: value.contains(TextDecorationLine::Underline),
        overline: value.contains(TextDecorationLine::Overline),
        line_through: value.contains(TextDecorationLine::LineThrough),
    }
}

const fn decoration_style(value: CssDecorationStyle) -> DecorationStyle {
    match value {
        CssDecorationStyle::Solid => DecorationStyle::Solid,
        CssDecorationStyle::Double => DecorationStyle::Double,
        CssDecorationStyle::Dotted => DecorationStyle::Dotted,
        CssDecorationStyle::Dashed => DecorationStyle::Dashed,
        CssDecorationStyle::Wavy => DecorationStyle::Wavy,
    }
}

fn thickness(
    value: &TextDecorationThickness,
    context: &ValueContext,
) -> Result<DecorationThickness, Unsupported> {
    Ok(match value {
        TextDecorationThickness::Auto => DecorationThickness::Auto,
        TextDecorationThickness::FromFont => DecorationThickness::FromFont,
        // Percentages refer to 1em.
        TextDecorationThickness::LengthPercentage(value) => DecorationThickness::Px(
            length_percentage(value, context)
                .ok_or(Unsupported::Value("unresolvable length"))?
                .resolve(context.font_size),
        ),
    })
}

fn font_weight(value: &CssFontWeight) -> Result<f32, Unsupported> {
    Ok(match value {
        CssFontWeight::Absolute(AbsoluteFontWeight::Weight(weight)) => *weight,
        CssFontWeight::Absolute(AbsoluteFontWeight::Normal) => 400.0,
        CssFontWeight::Absolute(AbsoluteFontWeight::Bold) => 700.0,
        CssFontWeight::Bolder | CssFontWeight::Lighter => {
            return Err(Unsupported::Value(
                "relative font weights are not supported",
            ));
        }
    })
}

const fn font_style(value: &CssFontStyle) -> FontStyle {
    match value {
        CssFontStyle::Normal => FontStyle::Normal,
        CssFontStyle::Italic => FontStyle::Italic,
        CssFontStyle::Oblique(_) => FontStyle::Oblique,
    }
}

fn line_height(value: &CssLineHeight, context: &ValueContext) -> Result<LineHeight, Unsupported> {
    Ok(match value {
        CssLineHeight::Normal => LineHeight::Normal,
        CssLineHeight::Number(number) => LineHeight::Number(*number),
        // Lengths and percentages compute to an absolute length.
        CssLineHeight::Length(value) => LineHeight::Px(
            length_percentage(value, context)
                .ok_or(Unsupported::Value("unresolvable length"))?
                .resolve(context.font_size),
        ),
    })
}

/// The computed `font-size` a `font-size` or `font` declaration sets.
pub(crate) fn font_size_of(
    property: &str,
    value: &str,
    parent: f32,
    context: &ValueContext,
) -> Option<f32> {
    let parsed =
        Property::parse_string(PropertyId::from(property), value, ParserOptions::default()).ok()?;
    match parsed {
        Property::FontSize(size) => font_size(&size, parent, context).ok(),
        Property::Font(font) => font_size(&font.size, parent, context).ok(),
        _ => None,
    }
}
