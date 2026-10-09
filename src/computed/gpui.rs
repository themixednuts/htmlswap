//! Typed computed styles in GPUI's terms.
//!
//! [`plan`] decides, once, what each computed field becomes in GPUI: which
//! style a value maps to, what GPUI draws only approximately, and what it
//! cannot draw at all. A runtime renderer applies the plan to an element; a
//! code generator prints it as builder calls. Both read the same decisions,
//! so they cannot disagree about what is supported.
//!
//! The plan holds no GPUI types. Lengths keep their runtime parts (rem and
//! viewport) for the consumer to resolve against the window.

use super::style::{
    Align, BorderStyle, BoxSizing, ComputedStyle, Cursor, DecorationThickness, Display, Distribute,
    FlexDirection, FlexWrap, FontFamily, FontStyle, GridAutoFlow, GridLine, LineHeight, Overflow,
    Position, RepeatCount, Sides, Size, TextAlign, TextOverflow, TextWrap, Track, TrackBreadth,
    TrackSize, Visibility, WhiteSpace,
};
use super::values::{Length, LengthAuto, LengthPercentage};
use super::{Corners, DecorationStyle, Rgba};

/// GPUI features beyond its stock styles.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Features {
    /// CSS grid track lists (`grid_template_columns` and friends), which
    /// stock GPUI lacks; without them a template becomes `grid_cols(n)`.
    pub grid_tracks: bool,
    /// `BoxShadow::inset`, which GPUI 0.2 lacks.
    pub inset_shadows: bool,
}

/// An absolute length: pixels, rems and viewport parts, which the consumer
/// resolves against the window. Never a percentage.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Absolute(Length);

impl Absolute {
    #[must_use]
    pub const fn px(px: f32) -> Self {
        Self(Length::px(px))
    }

    /// The length, when it has no percentage part.
    #[must_use]
    pub fn new(length: Length) -> Option<Self> {
        (length.fraction == 0.0).then_some(Self(length))
    }

    #[must_use]
    pub const fn length(self) -> Length {
        self.0
    }

    /// Pixels, when that is all it is.
    #[must_use]
    pub fn as_px(self) -> Option<f32> {
        self.0.as_px()
    }

    /// Rems, when that is all it is.
    #[must_use]
    pub fn as_rems(self) -> Option<f32> {
        self.0.as_rem()
    }

    #[must_use]
    pub fn is_zero(self) -> bool {
        self.0.is_zero()
    }
}

impl std::ops::Add for Absolute {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

/// A length GPUI can hold: absolute, or a fraction of the containing block.
/// GPUI has no `calc()`, so the two never mix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Definite {
    Absolute(Absolute),
    Fraction(f32),
}

impl Definite {
    /// The length, unless it mixes absolute and relative parts.
    #[must_use]
    pub fn new(length: LengthPercentage) -> Option<Self> {
        if let Some(absolute) = Absolute::new(length) {
            Some(Self::Absolute(absolute))
        } else {
            length.as_fraction().map(Self::Fraction)
        }
    }
}

/// A definite length or `auto`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GpuiLength {
    Auto,
    Definite(Definite),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GpuiDisplay {
    Hidden,
    Block,
    Flex,
    Grid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GpuiPosition {
    Relative,
    Absolute,
}

/// `align-items` / `align-self` as GPUI has them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GpuiAlign {
    Start,
    End,
    FlexStart,
    FlexEnd,
    Center,
    Baseline,
    Stretch,
}

/// `align-content` / `justify-content` as GPUI has them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GpuiDistribute {
    Start,
    End,
    FlexStart,
    FlexEnd,
    Center,
    Stretch,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GpuiOverflow {
    Visible,
    Hidden,
    Clip,
    Scroll,
}

/// GPUI's cursor styles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GpuiCursor {
    Arrow,
    PointingHand,
    IBeam,
    IBeamVertical,
    Crosshair,
    OpenHand,
    ClosedHand,
    NotAllowed,
    DragLink,
    DragCopy,
    ResizeLeftRight,
    ResizeUpDown,
    ResizeUpRightDownLeft,
    ResizeUpLeftDownRight,
    ResizeColumn,
    ResizeRow,
}

/// A corner radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Radius {
    Length(Absolute),
    /// A percentage of at least 50%: a pill or circle, which GPUI draws by
    /// clamping a very large radius.
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuiShadow {
    pub color: Rgba,
    pub x: Absolute,
    pub y: Absolute,
    pub blur: Absolute,
    pub spread: Absolute,
    pub inset: bool,
}

/// A grid track list as GPUI holds it.
#[derive(Clone, Debug, PartialEq)]
pub enum GpuiTracks {
    /// The full track list, with [`Features::grid_tracks`].
    Tracks(Vec<Track>),
    /// `n` equal flexible tracks: stock GPUI's `grid_cols` / `grid_rows`.
    Count(u16),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GpuiTextAlign {
    Left,
    Center,
    Right,
}

/// A text decoration line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuiDecoration {
    pub thickness: Absolute,
    pub color: Option<Rgba>,
    pub wavy: bool,
}

/// The text decorations `text-decoration-line` sets: `None` in a slot
/// clears that line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuiDecorations {
    pub underline: Option<GpuiDecoration>,
    pub strikethrough: Option<GpuiDecoration>,
}

/// A note on a declaration GPUI draws differently than CSS describes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Note {
    /// The CSS property.
    pub property: &'static str,
    pub reason: &'static str,
}

/// A computed style in GPUI's terms. Each field is `None` when the style
/// does not set it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GpuiStyle {
    pub display: Option<GpuiDisplay>,
    pub position: Option<GpuiPosition>,
    pub inset: Sides<Option<GpuiLength>>,
    pub width: Option<GpuiLength>,
    pub height: Option<GpuiLength>,
    pub min_width: Option<GpuiLength>,
    pub min_height: Option<GpuiLength>,
    pub max_width: Option<GpuiLength>,
    pub max_height: Option<GpuiLength>,
    pub aspect_ratio: Option<f32>,
    pub margin: Sides<Option<GpuiLength>>,
    pub padding: Sides<Option<Definite>>,
    pub flex_direction: Option<FlexDirection>,
    pub flex_wrap: Option<FlexWrap>,
    pub flex_grow: Option<f32>,
    pub flex_shrink: Option<f32>,
    pub flex_basis: Option<GpuiLength>,
    pub align_items: Option<GpuiAlign>,
    pub align_self: Option<GpuiAlign>,
    pub align_content: Option<GpuiDistribute>,
    pub justify_content: Option<GpuiDistribute>,
    pub column_gap: Option<Definite>,
    pub row_gap: Option<Definite>,
    pub grid_template_columns: Option<GpuiTracks>,
    pub grid_template_rows: Option<GpuiTracks>,
    pub grid_auto_columns: Option<Vec<TrackSize>>,
    pub grid_auto_rows: Option<Vec<TrackSize>>,
    pub grid_auto_flow: Option<GridAutoFlow>,
    pub grid_column_start: Option<GridLine>,
    pub grid_column_end: Option<GridLine>,
    pub grid_row_start: Option<GridLine>,
    pub grid_row_end: Option<GridLine>,
    pub overflow_x: Option<GpuiOverflow>,
    pub overflow_y: Option<GpuiOverflow>,
    /// `Some(false)` hides the element without removing it from layout.
    pub visible: Option<bool>,
    pub opacity: Option<f32>,
    pub cursor: Option<GpuiCursor>,
    pub background: Option<Rgba>,
    pub border_widths: Sides<Option<Absolute>>,
    pub border_dashed: bool,
    pub border_color: Option<Rgba>,
    pub corner_radii: Corners<Option<Radius>>,
    pub box_shadow: Option<Vec<GpuiShadow>>,
    pub text_color: Option<Rgba>,
    pub font_family: Option<Vec<FontFamily>>,
    pub font_size: Option<Absolute>,
    pub font_weight: Option<f32>,
    pub font_style: Option<FontStyle>,
    pub line_height: Option<Definite>,
    pub text_align: Option<GpuiTextAlign>,
    /// `Some(true)` keeps text on one line.
    pub no_wrap: Option<bool>,
    pub text_ellipsis: bool,
    pub line_clamp: Option<Option<u32>>,
    pub decorations: Option<GpuiDecorations>,
    /// Values GPUI cannot draw; they are left unset.
    pub limits: Vec<Note>,
    /// Values GPUI draws differently than CSS describes.
    pub approximations: Vec<Note>,
}

const MIXED: &str =
    "GPUI lengths are either absolute or relative; calc() mixing both is not supported";

impl GpuiStyle {
    fn limit(&mut self, property: &'static str, reason: &'static str) {
        let note = Note { property, reason };
        if !self.limits.contains(&note) {
            self.limits.push(note);
        }
    }

    fn approximate(&mut self, property: &'static str, reason: &'static str) {
        let note = Note { property, reason };
        if !self.approximations.contains(&note) {
            self.approximations.push(note);
        }
    }

    fn definite(&mut self, property: &'static str, value: LengthPercentage) -> Option<Definite> {
        let definite = Definite::new(value);
        if definite.is_none() {
            self.limit(property, MIXED);
        }
        definite
    }

    fn length(&mut self, property: &'static str, value: LengthAuto) -> Option<GpuiLength> {
        match value {
            LengthAuto::Auto => Some(GpuiLength::Auto),
            LengthAuto::Length(length) => self.definite(property, length).map(GpuiLength::Definite),
        }
    }

    fn absolute(
        &mut self,
        property: &'static str,
        value: Length,
        reason: &'static str,
    ) -> Option<Absolute> {
        let absolute = Absolute::new(value);
        if absolute.is_none() {
            self.limit(property, reason);
        }
        absolute
    }
}

/// What `content-box` sizing adds to a size on one axis: the padding and
/// drawn border widths, which GPUI (always border-box) needs included.
/// `None` when the padding is relative, so no absolute size can include it.
fn content_box_extra(style: &ComputedStyle, horizontal: bool) -> Option<Absolute> {
    if matches!(style.box_sizing, Some(BoxSizing::BorderBox)) {
        return Some(Absolute::default());
    }
    let (padding, sides) = if horizontal {
        (
            [style.padding.left, style.padding.right],
            [
                (style.border_width.left, style.border_style.left),
                (style.border_width.right, style.border_style.right),
            ],
        )
    } else {
        (
            [style.padding.top, style.padding.bottom],
            [
                (style.border_width.top, style.border_style.top),
                (style.border_width.bottom, style.border_style.bottom),
            ],
        )
    };
    let mut extra = Absolute::default();
    for padding in padding.into_iter().flatten() {
        extra = extra + Absolute::new(padding)?;
    }
    for (width, line) in sides {
        if !matches!(line, None | Some(BorderStyle::None | BorderStyle::Hidden)) {
            extra = extra + width.and_then(Absolute::new).unwrap_or_default();
        }
    }
    Some(extra)
}

const fn align(value: Align) -> Option<GpuiAlign> {
    Some(match value {
        Align::Start | Align::SelfStart | Align::Left => GpuiAlign::Start,
        Align::End | Align::SelfEnd | Align::Right => GpuiAlign::End,
        Align::FlexStart => GpuiAlign::FlexStart,
        Align::FlexEnd => GpuiAlign::FlexEnd,
        Align::Center => GpuiAlign::Center,
        Align::Baseline => GpuiAlign::Baseline,
        Align::Stretch | Align::Normal => GpuiAlign::Stretch,
        Align::Auto | Align::LastBaseline => return None,
    })
}

const fn distribute(value: Distribute) -> Option<GpuiDistribute> {
    Some(match value {
        Distribute::Start | Distribute::Left => GpuiDistribute::Start,
        Distribute::End | Distribute::Right => GpuiDistribute::End,
        Distribute::FlexStart => GpuiDistribute::FlexStart,
        Distribute::FlexEnd => GpuiDistribute::FlexEnd,
        Distribute::Center => GpuiDistribute::Center,
        Distribute::Stretch => GpuiDistribute::Stretch,
        Distribute::SpaceBetween => GpuiDistribute::SpaceBetween,
        Distribute::SpaceAround => GpuiDistribute::SpaceAround,
        Distribute::SpaceEvenly => GpuiDistribute::SpaceEvenly,
        Distribute::Normal => return None,
    })
}

const fn overflow(value: Overflow) -> GpuiOverflow {
    match value {
        Overflow::Visible => GpuiOverflow::Visible,
        Overflow::Hidden => GpuiOverflow::Hidden,
        Overflow::Clip => GpuiOverflow::Clip,
        Overflow::Scroll | Overflow::Auto => GpuiOverflow::Scroll,
    }
}

const fn cursor(value: Cursor) -> Option<GpuiCursor> {
    Some(match value {
        Cursor::Auto | Cursor::Default => GpuiCursor::Arrow,
        Cursor::Pointer => GpuiCursor::PointingHand,
        Cursor::Text => GpuiCursor::IBeam,
        Cursor::VerticalText => GpuiCursor::IBeamVertical,
        Cursor::Crosshair | Cursor::Cell => GpuiCursor::Crosshair,
        Cursor::Grab => GpuiCursor::OpenHand,
        Cursor::Grabbing | Cursor::Move | Cursor::AllScroll => GpuiCursor::ClosedHand,
        Cursor::NotAllowed | Cursor::NoDrop => GpuiCursor::NotAllowed,
        Cursor::Alias => GpuiCursor::DragLink,
        Cursor::Copy => GpuiCursor::DragCopy,
        Cursor::EwResize => GpuiCursor::ResizeLeftRight,
        Cursor::NsResize => GpuiCursor::ResizeUpDown,
        Cursor::NeswResize => GpuiCursor::ResizeUpRightDownLeft,
        Cursor::NwseResize => GpuiCursor::ResizeUpLeftDownRight,
        Cursor::ColResize => GpuiCursor::ResizeColumn,
        Cursor::RowResize => GpuiCursor::ResizeRow,
        Cursor::ContextMenu
        | Cursor::Help
        | Cursor::Progress
        | Cursor::Wait
        | Cursor::None
        | Cursor::ZoomIn
        | Cursor::ZoomOut => return None,
    })
}

fn breadth_is_definite(value: TrackBreadth) -> bool {
    match value {
        TrackBreadth::Length(length) => Definite::new(length).is_some(),
        TrackBreadth::Flex(_)
        | TrackBreadth::MinContent
        | TrackBreadth::MaxContent
        | TrackBreadth::Auto => true,
    }
}

fn size_is_definite(value: TrackSize) -> bool {
    match value {
        TrackSize::Breadth(value) => breadth_is_definite(value),
        TrackSize::MinMax(min, max) => breadth_is_definite(min) && breadth_is_definite(max),
        TrackSize::FitContent(limit) => Definite::new(limit).is_some(),
    }
}

/// The number of equal `1fr` tracks a list describes, which stock GPUI's
/// `grid_cols(n)` lays out exactly.
fn equal_fraction_count(tracks: &[Track]) -> Option<u16> {
    let one_fr = |size: &TrackSize| matches!(size, TrackSize::Breadth(TrackBreadth::Flex(fr)) if (*fr - 1.0).abs() < f32::EPSILON);
    let mut count = 0_u16;
    for track in tracks {
        match track {
            Track::Size(size) if one_fr(size) => count = count.checked_add(1)?,
            Track::Repeat {
                count: RepeatCount::Count(times),
                tracks,
            } if tracks.iter().all(one_fr) => {
                let added = times.checked_mul(u16::try_from(tracks.len()).ok()?)?;
                count = count.checked_add(added)?;
            }
            _ => return None,
        }
    }
    (count > 0).then_some(count)
}

/// The number of tracks a list describes, when it is fixed.
fn track_count(tracks: &[Track]) -> Option<u16> {
    let mut count = 0_u16;
    for track in tracks {
        match track {
            Track::Size(_) => count = count.checked_add(1)?,
            Track::Repeat {
                count: RepeatCount::Count(times),
                tracks,
            } => {
                let added = times.checked_mul(u16::try_from(tracks.len()).ok()?)?;
                count = count.checked_add(added)?;
            }
            Track::Repeat { .. } => return None,
        }
    }
    (count > 0).then_some(count)
}

fn tracks(
    plan: &mut GpuiStyle,
    property: &'static str,
    tracks: &[Track],
    features: Features,
) -> Option<GpuiTracks> {
    if features.grid_tracks {
        let definite = tracks.iter().all(|track| match track {
            Track::Size(size) => size_is_definite(*size),
            Track::Repeat { tracks, .. } => tracks.iter().copied().all(size_is_definite),
        });
        if !definite {
            plan.limit(property, MIXED);
            return None;
        }
        return Some(GpuiTracks::Tracks(tracks.to_vec()));
    }
    if tracks.is_empty() {
        return None;
    }
    if let Some(count) = equal_fraction_count(tracks) {
        return Some(GpuiTracks::Count(count));
    }
    if let Some(count) = track_count(tracks) {
        plan.approximate(
            property,
            "stock GPUI grids have equal tracks; the track sizes are not kept",
        );
        return Some(GpuiTracks::Count(count));
    }
    plan.limit(
        property,
        "stock GPUI grids have a fixed number of equal tracks; auto-fill and auto-fit need grid track support",
    );
    None
}

/// The plan for an element's computed style.
#[must_use]
pub fn plan(style: &ComputedStyle, features: Features) -> GpuiStyle {
    plan_within(style, style, features)
}

/// The plan for some of an element's declarations, computed into `style`.
/// `element` is the element's whole computed style, whose padding, borders
/// and `box-sizing` decide how `content-box` sizes become GPUI's border-box
/// sizes.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn plan_within(
    style: &ComputedStyle,
    element: &ComputedStyle,
    features: Features,
) -> GpuiStyle {
    let mut plan = GpuiStyle::default();
    let content_size = "GPUI has no intrinsic (min-content, max-content, fit-content) sizes";

    plan.display = match style.display {
        Some(Display::None) => Some(GpuiDisplay::Hidden),
        Some(Display::Flex) => Some(GpuiDisplay::Flex),
        Some(Display::InlineFlex) => {
            plan.approximate("display", "GPUI has no inline layout; laid out as flex");
            Some(GpuiDisplay::Flex)
        }
        Some(Display::Grid) => Some(GpuiDisplay::Grid),
        Some(Display::InlineGrid) => {
            plan.approximate("display", "GPUI has no inline layout; laid out as grid");
            Some(GpuiDisplay::Grid)
        }
        Some(Display::Block | Display::FlowRoot) => Some(GpuiDisplay::Block),
        Some(Display::Inline | Display::InlineBlock) => {
            plan.approximate("display", "GPUI has no inline layout; laid out as block");
            Some(GpuiDisplay::Block)
        }
        Some(Display::Contents | Display::Other) => {
            plan.limit("display", "GPUI has no contents, table or ruby display");
            None
        }
        None => None,
    };
    plan.position = match style.position {
        Some(Position::Absolute) => Some(GpuiPosition::Absolute),
        Some(Position::Relative | Position::Static) => Some(GpuiPosition::Relative),
        Some(Position::Fixed) => {
            plan.approximate(
                "position",
                "GPUI has no fixed positioning; positioned absolutely",
            );
            Some(GpuiPosition::Absolute)
        }
        Some(Position::Sticky) => {
            plan.approximate(
                "position",
                "GPUI has no sticky positioning; positioned relatively",
            );
            Some(GpuiPosition::Relative)
        }
        None => None,
    };
    let inset = [
        style.inset.top,
        style.inset.right,
        style.inset.bottom,
        style.inset.left,
    ]
    .map(|value| value.and_then(|value| plan.length("inset", value)));
    [
        plan.inset.top,
        plan.inset.right,
        plan.inset.bottom,
        plan.inset.left,
    ] = inset;

    let horizontal = content_box_extra(element, true);
    let vertical = content_box_extra(element, false);
    let sizes = [
        ("width", style.width, horizontal),
        ("height", style.height, vertical),
        ("min-width", style.min_width, horizontal),
        ("min-height", style.min_height, vertical),
        ("max-width", style.max_width, horizontal),
        ("max-height", style.max_height, vertical),
    ];
    let mut planned = [None; 6];
    for (index, (property, value, extra)) in sizes.into_iter().enumerate() {
        planned[index] = match value {
            None | Some(Size::None) => None,
            Some(Size::Auto) => Some(GpuiLength::Auto),
            Some(Size::MinContent | Size::MaxContent | Size::FitContent) => {
                plan.limit(property, content_size);
                None
            }
            Some(Size::Length(length)) => match (plan.definite(property, length), extra) {
                (None, _) => None,
                (Some(Definite::Absolute(size)), Some(extra)) => {
                    Some(GpuiLength::Definite(Definite::Absolute(size + extra)))
                }
                (Some(definite @ Definite::Fraction(_)), Some(extra)) if extra.is_zero() => {
                    Some(GpuiLength::Definite(definite))
                }
                (Some(Definite::Absolute(size)), None) => {
                    plan.approximate(
                        property,
                        "GPUI sizes are border-box; the relative padding of this content-box size is not added",
                    );
                    Some(GpuiLength::Definite(Definite::Absolute(size)))
                }
                (Some(definite @ Definite::Fraction(_)), _) => {
                    plan.approximate(
                        property,
                        "GPUI sizes are border-box; padding and borders fall inside this relative content-box size",
                    );
                    Some(GpuiLength::Definite(definite))
                }
            },
        };
    }
    [
        plan.width,
        plan.height,
        plan.min_width,
        plan.min_height,
        plan.max_width,
        plan.max_height,
    ] = planned;
    plan.aspect_ratio = style.aspect_ratio;

    let margin = [
        style.margin.top,
        style.margin.right,
        style.margin.bottom,
        style.margin.left,
    ]
    .map(|value| value.and_then(|value| plan.length("margin", value)));
    [
        plan.margin.top,
        plan.margin.right,
        plan.margin.bottom,
        plan.margin.left,
    ] = margin;
    let padding = [
        style.padding.top,
        style.padding.right,
        style.padding.bottom,
        style.padding.left,
    ]
    .map(|value| value.and_then(|value| plan.definite("padding", value)));
    [
        plan.padding.top,
        plan.padding.right,
        plan.padding.bottom,
        plan.padding.left,
    ] = padding;

    plan.flex_direction = style.flex_direction;
    plan.flex_wrap = style.flex_wrap;
    plan.flex_grow = style.flex_grow;
    plan.flex_shrink = style.flex_shrink;
    plan.flex_basis = match style.flex_basis {
        None | Some(Size::None) => None,
        Some(Size::Auto) => Some(GpuiLength::Auto),
        Some(Size::MinContent | Size::MaxContent | Size::FitContent) => {
            plan.limit("flex-basis", content_size);
            None
        }
        Some(Size::Length(length)) => plan
            .definite("flex-basis", length)
            .map(GpuiLength::Definite),
    };
    if matches!(style.align_items, Some(Align::LastBaseline))
        || matches!(style.align_self, Some(Align::LastBaseline))
    {
        plan.limit("align-items", "GPUI has no last-baseline alignment");
    }
    plan.align_items = style.align_items.and_then(align);
    plan.align_self = style.align_self.and_then(align);
    plan.align_content = style.align_content.and_then(distribute);
    plan.justify_content = style.justify_content.and_then(distribute);
    if style.justify_items.is_some() || style.justify_self.is_some() {
        plan.limit("justify-items", "GPUI has no justify-items or justify-self");
    }
    plan.column_gap = style
        .column_gap
        .and_then(|value| plan.definite("gap", value));
    plan.row_gap = style.row_gap.and_then(|value| plan.definite("gap", value));

    plan.grid_template_columns = style
        .grid_template_columns
        .as_deref()
        .and_then(|list| tracks(&mut plan, "grid-template-columns", list, features));
    plan.grid_template_rows = style
        .grid_template_rows
        .as_deref()
        .and_then(|list| tracks(&mut plan, "grid-template-rows", list, features));
    if features.grid_tracks {
        plan.grid_auto_columns = style.grid_auto_columns.clone();
        plan.grid_auto_rows = style.grid_auto_rows.clone();
        plan.grid_auto_flow = style.grid_auto_flow;
        plan.grid_column_start = style.grid_column_start;
        plan.grid_column_end = style.grid_column_end;
        plan.grid_row_start = style.grid_row_start;
        plan.grid_row_end = style.grid_row_end;
    } else {
        let placement = [
            style.grid_column_start,
            style.grid_column_end,
            style.grid_row_start,
            style.grid_row_end,
        ];
        if style.grid_auto_columns.is_some()
            || style.grid_auto_rows.is_some()
            || style.grid_auto_flow.is_some()
            || placement
                .iter()
                .flatten()
                .any(|line| *line != GridLine::Auto)
        {
            plan.limit(
                "grid",
                "stock GPUI grids have no implicit track sizes, auto-flow or item placement",
            );
        }
    }

    plan.overflow_x = style.overflow_x.map(overflow);
    plan.overflow_y = style.overflow_y.map(overflow);
    plan.visible = style
        .visibility
        .map(|visibility| visibility == Visibility::Visible);
    plan.opacity = style.opacity;
    plan.cursor = style.cursor.and_then(|value| {
        let mapped = cursor(value);
        if mapped.is_none() {
            plan.limit("cursor", "GPUI has no cursor of this kind");
        }
        mapped
    });
    plan.background = style.background_color;

    borders(&mut plan, style);
    if let Some(shadows) = &style.box_shadow {
        let drawn = shadows
            .iter()
            .filter(|shadow| features.inset_shadows || !shadow.inset)
            .collect::<Vec<_>>();
        if drawn.len() < shadows.len() {
            if drawn.is_empty() {
                plan.limit("box-shadow", "this GPUI draws no inset shadows");
            } else {
                plan.approximate(
                    "box-shadow",
                    "this GPUI draws no inset shadows; they are left out",
                );
            }
        }
        let planned = (!(drawn.is_empty() && !shadows.is_empty()))
            .then(|| {
                drawn
                    .into_iter()
                    .map(|shadow| {
                        Some(GpuiShadow {
                            color: shadow.color,
                            x: Absolute::new(shadow.x)?,
                            y: Absolute::new(shadow.y)?,
                            blur: Absolute::new(shadow.blur)?,
                            spread: Absolute::new(shadow.spread)?,
                            inset: shadow.inset,
                        })
                    })
                    .collect::<Option<Vec<_>>>()
            })
            .flatten();
        if planned.is_none() && !drawn_empty_or_limited(&plan) {
            plan.limit("box-shadow", MIXED);
        }
        plan.box_shadow = planned;
    }
    text(&mut plan, style);
    plan
}

fn drawn_empty_or_limited(plan: &GpuiStyle) -> bool {
    plan.limits.iter().any(|note| note.property == "box-shadow")
}

fn borders(plan: &mut GpuiStyle, style: &ComputedStyle) {
    let sides = [
        (style.border_width.top, style.border_style.top),
        (style.border_width.right, style.border_style.right),
        (style.border_width.bottom, style.border_style.bottom),
        (style.border_width.left, style.border_style.left),
    ];
    let mut widths = [None; 4];
    for (slot, (width, line)) in widths.iter_mut().zip(sides) {
        // A border is drawn only with a visible style; its width alone does
        // not draw it (the initial style is `none`).
        match (width, line) {
            (_, Some(BorderStyle::None | BorderStyle::Hidden)) => {
                *slot = Some(Absolute::default());
            }
            (Some(width), Some(_)) => {
                *slot = plan.absolute(
                    "border-width",
                    width,
                    "GPUI border widths are absolute lengths",
                );
            }
            (Some(_), None) | (None, _) => {}
        }
        if matches!(line, Some(BorderStyle::Dashed | BorderStyle::Dotted)) {
            plan.border_dashed = true;
        }
    }
    [
        plan.border_widths.top,
        plan.border_widths.right,
        plan.border_widths.bottom,
        plan.border_widths.left,
    ] = widths;
    let styles = sides.map(|(_, line)| line);
    if styles.iter().flatten().any(|line| {
        matches!(
            line,
            BorderStyle::Double
                | BorderStyle::Groove
                | BorderStyle::Ridge
                | BorderStyle::Inset
                | BorderStyle::Outset
        )
    }) {
        plan.limit("border-style", "GPUI draws solid and dashed borders only");
    }
    if styles
        .iter()
        .flatten()
        .any(|line| matches!(line, BorderStyle::Dotted))
    {
        plan.approximate("border-style", "dotted borders are drawn dashed");
    }
    let colors = [
        style.border_color.top,
        style.border_color.right,
        style.border_color.bottom,
        style.border_color.left,
    ];
    let mut set = colors.iter().flatten();
    if let Some(first) = set.next() {
        plan.border_color = Some(*first);
        if set.any(|other| other != first) {
            plan.limit("border-color", "GPUI draws one border color for all sides");
        }
    }
    let corners = [
        style.border_radius.top_left,
        style.border_radius.top_right,
        style.border_radius.bottom_right,
        style.border_radius.bottom_left,
    ];
    let mut radii = [None; 4];
    for (slot, value) in radii.iter_mut().zip(corners) {
        let Some(value) = value else { continue };
        *slot = if let Some(absolute) = Absolute::new(value) {
            Some(Radius::Length(absolute))
        } else if value.as_fraction().is_some_and(|fraction| fraction >= 0.5) {
            Some(Radius::Full)
        } else {
            plan.limit("border-radius", "GPUI corner radii are absolute lengths");
            None
        };
    }
    plan.corner_radii = Corners {
        top_left: radii[0],
        top_right: radii[1],
        bottom_right: radii[2],
        bottom_left: radii[3],
    };
}

fn text(plan: &mut GpuiStyle, style: &ComputedStyle) {
    plan.text_color = style.color;
    plan.font_family.clone_from(&style.font_family);
    plan.font_size = style
        .font_size
        .and_then(|size| plan.absolute("font-size", size, MIXED));
    plan.font_weight = style.font_weight;
    plan.font_style = style.font_style;
    plan.line_height = match style.line_height {
        Some(LineHeight::Number(factor)) => Some(Definite::Fraction(factor)),
        Some(LineHeight::Length(height)) => plan
            .absolute("line-height", height, MIXED)
            .map(Definite::Absolute),
        Some(LineHeight::Normal) | None => None,
    };
    plan.text_align = match style.text_align {
        Some(TextAlign::Start | TextAlign::Left) => Some(GpuiTextAlign::Left),
        Some(TextAlign::End | TextAlign::Right) => Some(GpuiTextAlign::Right),
        Some(TextAlign::Center) => Some(GpuiTextAlign::Center),
        Some(TextAlign::Justify) => {
            plan.limit("text-align", "GPUI text cannot be justified");
            None
        }
        None => None,
    };
    let no_wrap = matches!(
        style.white_space,
        Some(WhiteSpace::NoWrap | WhiteSpace::Pre)
    ) || style.text_wrap == Some(TextWrap::NoWrap);
    if matches!(
        style.white_space,
        Some(WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::PreLine | WhiteSpace::BreakSpaces)
    ) {
        plan.approximate(
            "white-space",
            "GPUI keeps the text's white space as given; collapsing rules are not applied",
        );
    }
    if no_wrap {
        plan.no_wrap = Some(true);
    } else if style.white_space.is_some() || style.text_wrap.is_some() {
        plan.no_wrap = Some(false);
    }
    if matches!(
        style.text_wrap,
        Some(TextWrap::Balance | TextWrap::Pretty | TextWrap::Stable)
    ) {
        plan.limit(
            "text-wrap",
            "GPUI wraps greedily; balance and pretty wrap normally",
        );
    }
    plan.text_ellipsis = style.text_overflow == Some(TextOverflow::Ellipsis);
    plan.line_clamp = style.line_clamp;
    if let Some(lines) = style.text_decoration_line {
        let wavy = style.text_decoration_style == Some(DecorationStyle::Wavy);
        let thickness = match style.text_decoration_thickness {
            Some(DecorationThickness::Length(thickness)) => plan
                .absolute("text-decoration-thickness", thickness, MIXED)
                .unwrap_or(Absolute::px(1.0)),
            _ => Absolute::px(1.0),
        };
        let line = GpuiDecoration {
            thickness,
            color: style.text_decoration_color,
            wavy,
        };
        plan.decorations = Some(GpuiDecorations {
            underline: lines.underline.then_some(line),
            strikethrough: lines.line_through.then_some(GpuiDecoration {
                wavy: false,
                ..line
            }),
        });
        if lines.overline {
            plan.limit("text-decoration", "GPUI text has no overline");
        }
    }
    if matches!(
        style.text_decoration_style,
        Some(DecorationStyle::Double | DecorationStyle::Dotted | DecorationStyle::Dashed)
    ) {
        plan.limit(
            "text-decoration-style",
            "GPUI draws solid and wavy text decorations only",
        );
    }
    if style
        .letter_spacing
        .is_some_and(|spacing| !spacing.is_zero())
    {
        plan.limit("letter-spacing", "GPUI text has no letter spacing");
    }
    if style.word_spacing.is_some_and(|spacing| !spacing.is_zero()) {
        plan.limit("word-spacing", "GPUI text has no word spacing");
    }
}

#[cfg(test)]
mod tests {
    use super::{Absolute, Definite, Features, GpuiLength, GpuiTracks, Radius, plan};
    use crate::computed::{
        BorderStyle, ComputedStyle, Display, LengthPercentage, RepeatCount, Sides, Size, Track,
        TrackBreadth, TrackSize,
    };

    #[test]
    fn content_box_sizes_include_absolute_padding_and_borders() {
        let style = ComputedStyle {
            width: Some(Size::Length(LengthPercentage::rem(10.0))),
            padding: Sides::all(Some(LengthPercentage::px(4.0))),
            border_width: Sides::all(Some(LengthPercentage::px(1.0))),
            border_style: Sides::all(Some(BorderStyle::Solid)),
            ..ComputedStyle::default()
        };
        let planned = plan(&style, Features::default());
        let Some(GpuiLength::Definite(Definite::Absolute(width))) = planned.width else {
            panic!("{:?}", planned.width);
        };
        assert_eq!(width.length().rem, 10.0);
        assert_eq!(width.length().px, 10.0);
    }

    #[test]
    fn stock_grids_count_tracks_and_note_what_they_lose() {
        let fr = TrackSize::Breadth(TrackBreadth::Flex(1.0));
        let style = ComputedStyle {
            display: Some(Display::Grid),
            grid_template_columns: Some(vec![Track::Repeat {
                count: RepeatCount::Count(3),
                tracks: vec![fr],
            }]),
            grid_template_rows: Some(vec![
                Track::Size(TrackSize::Breadth(TrackBreadth::Length(
                    LengthPercentage::px(20.0),
                ))),
                Track::Size(fr),
            ]),
            ..ComputedStyle::default()
        };
        let planned = plan(&style, Features::default());
        assert_eq!(planned.grid_template_columns, Some(GpuiTracks::Count(3)));
        assert_eq!(planned.grid_template_rows, Some(GpuiTracks::Count(2)));
        assert_eq!(planned.approximations.len(), 1);
        let planned = plan(
            &style,
            Features {
                grid_tracks: true,
                ..Features::default()
            },
        );
        assert!(matches!(
            planned.grid_template_rows,
            Some(GpuiTracks::Tracks(_))
        ));
        assert!(planned.approximations.is_empty());
    }

    #[test]
    fn mixed_lengths_and_half_radii() {
        let style = ComputedStyle {
            padding: Sides {
                left: Some(LengthPercentage {
                    px: 2.0,
                    fraction: 0.5,
                    ..LengthPercentage::ZERO
                }),
                ..Sides::default()
            },
            border_radius: crate::computed::Corners {
                top_left: Some(LengthPercentage::fraction(0.5)),
                ..Default::default()
            },
            ..ComputedStyle::default()
        };
        let planned = plan(&style, Features::default());
        assert_eq!(planned.padding.left, None);
        assert_eq!(planned.limits.len(), 1);
        assert_eq!(planned.corner_radii.top_left, Some(Radius::Full));
        assert_eq!(Absolute::px(3.0).as_px(), Some(3.0));
    }
}
