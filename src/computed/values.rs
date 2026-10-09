//! Lengths: every CSS unit and math function, resolved against the
//! element's font sizes and the viewport.

use lightningcss::values::calc::{Calc, MathFunction};
use lightningcss::values::length::{
    Length as CssLength, LengthPercentage as CssLengthPercentage, LengthValue,
};
use lightningcss::values::percentage::DimensionPercentage;

/// What relative units resolve against. A base that is `None` is decided
/// at runtime (by the window), so lengths keep it as a symbolic part.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValueContext {
    /// The element's computed `font-size`, for `em` and friends.
    pub font_size: LengthPercentage,
    /// What `rem` is: the root element's `font-size`, or the window's rem
    /// size (`Length::rem(1.0)`) when the window takes the root's size.
    pub root_font_size: Length,
    /// The viewport size in pixels, when known; otherwise viewport units
    /// stay symbolic.
    pub viewport: Option<(f32, f32)>,
}

impl Default for ValueContext {
    fn default() -> Self {
        Self {
            font_size: LengthPercentage::rem(1.0),
            root_font_size: LengthPercentage::rem(1.0),
            viewport: None,
        }
    }
}

/// The runtime bases a symbolic length resolves against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bases {
    /// The rem size in pixels.
    pub rem: f32,
    pub viewport_width: f32,
    pub viewport_height: f32,
}

impl Default for Bases {
    fn default() -> Self {
        Self {
            rem: 16.0,
            viewport_width: 1024.0,
            viewport_height: 768.0,
        }
    }
}

/// A length-percentage after resolving units: a sum of pixels, parts that
/// depend on runtime bases (`rem`, the viewport), and a fraction of whatever
/// the property's percentages refer to. `calc()` keeps every part.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LengthPercentage {
    pub px: f32,
    /// Multiples of the rem size.
    pub rem: f32,
    /// Fractions of the viewport width, height, smaller and larger side.
    pub vw: f32,
    pub vh: f32,
    pub vmin: f32,
    pub vmax: f32,
    /// A fraction of the percentage basis.
    pub fraction: f32,
}

/// A length-percentage with no percentage part.
pub type Length = LengthPercentage;

impl LengthPercentage {
    pub const ZERO: Self = Self {
        px: 0.0,
        rem: 0.0,
        vw: 0.0,
        vh: 0.0,
        vmin: 0.0,
        vmax: 0.0,
        fraction: 0.0,
    };

    #[must_use]
    pub const fn px(px: f32) -> Self {
        Self { px, ..Self::ZERO }
    }

    #[must_use]
    pub const fn rem(rem: f32) -> Self {
        Self { rem, ..Self::ZERO }
    }

    #[must_use]
    pub const fn fraction(fraction: f32) -> Self {
        Self {
            fraction,
            ..Self::ZERO
        }
    }

    const fn parts(self) -> [f32; 7] {
        [
            self.px,
            self.rem,
            self.vw,
            self.vh,
            self.vmin,
            self.vmax,
            self.fraction,
        ]
    }

    const fn from_parts(parts: [f32; 7]) -> Self {
        let [px, rem, vw, vh, vmin, vmax, fraction] = parts;
        Self {
            px,
            rem,
            vw,
            vh,
            vmin,
            vmax,
            fraction,
        }
    }

    fn only(self, index: usize) -> Option<f32> {
        let parts = self.parts();
        parts
            .iter()
            .enumerate()
            .all(|(other, value)| other == index || *value == 0.0)
            .then_some(parts[index])
    }

    /// The pixel length when nothing else is involved.
    #[must_use]
    pub fn as_px(self) -> Option<f32> {
        self.only(0)
    }

    /// The rem multiple when nothing else is involved.
    #[must_use]
    pub fn as_rem(self) -> Option<f32> {
        self.only(1)
    }

    /// The fraction when nothing else is involved.
    #[must_use]
    pub fn as_fraction(self) -> Option<f32> {
        self.only(6)
    }

    /// Whether any part depends on the viewport.
    #[must_use]
    pub fn uses_viewport(self) -> bool {
        self.vw != 0.0 || self.vh != 0.0 || self.vmin != 0.0 || self.vmax != 0.0
    }

    /// The parts' bit patterns, for hashing.
    #[must_use]
    pub fn bits(self) -> [u32; 7] {
        self.parts().map(f32::to_bits)
    }

    /// Whether the length is zero in every part.
    #[must_use]
    pub fn is_zero(self) -> bool {
        self.parts().iter().all(|part| *part == 0.0)
    }

    /// Resolve against runtime bases and a percentage basis.
    #[must_use]
    pub fn resolve(self, bases: &Bases, basis: f32) -> f32 {
        let (width, height) = (bases.viewport_width, bases.viewport_height);
        self.px
            + self.rem * bases.rem
            + self.vw * width
            + self.vh * height
            + self.vmin * width.min(height)
            + self.vmax * width.max(height)
            + self.fraction * basis
    }

    /// Fold the viewport parts into pixels when the context knows the
    /// viewport.
    #[must_use]
    pub fn fold(self, context: &ValueContext) -> Self {
        let mut folded = self;
        if let Some((width, height)) = context.viewport {
            folded.px += folded.vw * width
                + folded.vh * height
                + folded.vmin * width.min(height)
                + folded.vmax * width.max(height);
            folded.vw = 0.0;
            folded.vh = 0.0;
            folded.vmin = 0.0;
            folded.vmax = 0.0;
        }
        folded
    }

    #[must_use]
    pub fn lerp(self, to: Self, t: f32) -> Self {
        let (from, to) = (self.parts(), to.parts());
        Self::from_parts(std::array::from_fn(|index| {
            from[index] + (to[index] - from[index]) * t
        }))
    }

    #[must_use]
    pub fn scale(self, factor: f32) -> Self {
        Self::from_parts(self.parts().map(|part| part * factor))
    }
}

impl std::ops::Add for LengthPercentage {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        let (a, b) = (self.parts(), other.parts());
        Self::from_parts(std::array::from_fn(|index| a[index] + b[index]))
    }
}

impl LengthPercentage {
    /// The length with its percentage taken of `basis`, for properties
    /// whose percentages refer to a length (such as the font size).
    #[must_use]
    pub fn percent_of(self, basis: Length) -> Length {
        Self {
            fraction: 0.0,
            ..self
        } + basis.scale(self.fraction)
    }
}

/// A length-percentage or `auto`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LengthAuto {
    Auto,
    Length(LengthPercentage),
}

/// One length unit.
pub(crate) fn length_value(value: &LengthValue, context: &ValueContext) -> Option<Length> {
    use LengthValue::{
        Cap, Ch, Cqb, Cqh, Cqi, Cqmax, Cqmin, Cqw, Dvb, Dvh, Dvi, Dvmax, Dvmin, Dvw, Em, Ex, Ic,
        Lh, Lvb, Lvh, Lvi, Lvmax, Lvmin, Lvw, Rcap, Rch, Rem, Rex, Ric, Rlh, Svb, Svh, Svi, Svmax,
        Svmin, Svw, Vb, Vh, Vi, Vmax, Vmin, Vw,
    };
    if let Some(px) = value.to_px() {
        return px.is_finite().then(|| Length::px(px));
    }
    let em = context.font_size;
    let rem = context.root_font_size;
    let percent = |part: fn(f32) -> Length, v: f32| part(v / 100.0);
    let vw = |v| Length {
        vw: v,
        ..Length::ZERO
    };
    let vh = |v| Length {
        vh: v,
        ..Length::ZERO
    };
    let vmin = |v| Length {
        vmin: v,
        ..Length::ZERO
    };
    let vmax = |v| Length {
        vmax: v,
        ..Length::ZERO
    };
    // Without font metrics, CSS Values 4 §6.1.1 assigns these fallbacks.
    let length =
        match value {
            Em(v) => em.scale(*v),
            Rem(v) => rem.scale(*v),
            Ex(v) | Cap(v) | Ch(v) => em.scale(v * 0.5),
            Rex(v) | Rcap(v) | Rch(v) => rem.scale(v * 0.5),
            Ic(v) => em.scale(*v),
            Ric(v) => rem.scale(*v),
            Lh(v) => em.scale(v * 1.2),
            Rlh(v) => rem.scale(v * 1.2),
            // The window is the viewport: large, small and dynamic agree, and
            // there are no size containers, so container units use it too.
            Vw(v) | Lvw(v) | Svw(v) | Dvw(v) | Cqw(v) | Vi(v) | Lvi(v) | Svi(v) | Dvi(v)
            | Cqi(v) => percent(vw, *v),
            Vh(v) | Lvh(v) | Svh(v) | Dvh(v) | Cqh(v) | Vb(v) | Lvb(v) | Svb(v) | Dvb(v)
            | Cqb(v) => percent(vh, *v),
            Vmin(v) | Lvmin(v) | Svmin(v) | Dvmin(v) | Cqmin(v) => percent(vmin, *v),
            Vmax(v) | Lvmax(v) | Svmax(v) | Dvmax(v) | Cqmax(v) => percent(vmax, *v),
            _ => return None,
        };
    let length = length.fold(context);
    length
        .parts()
        .iter()
        .all(|part| part.is_finite())
        .then_some(length)
}

/// A `<length>`.
pub(crate) fn length(value: &CssLength, context: &ValueContext) -> Option<Length> {
    match value {
        CssLength::Value(value) => length_value(value, context),
        CssLength::Calc(calc) => evaluate(calc, &|value: &CssLength| length(value, context)),
    }
}

/// A `<length-percentage>`.
pub(crate) fn length_percentage(
    value: &CssLengthPercentage,
    context: &ValueContext,
) -> Option<LengthPercentage> {
    match value {
        DimensionPercentage::Dimension(value) => length_value(value, context),
        DimensionPercentage::Percentage(percentage) => {
            Some(LengthPercentage::fraction(percentage.0))
        }
        DimensionPercentage::Calc(calc) => evaluate(calc, &|value: &CssLengthPercentage| {
            length_percentage(value, context)
        }),
    }
}

/// Evaluate a math expression. Mixed lengths and percentages add up into
/// both parts; comparisons and stepped functions need operands of one kind.
fn evaluate<V>(
    calc: &Calc<V>,
    leaf: &dyn Fn(&V) -> Option<LengthPercentage>,
) -> Option<LengthPercentage> {
    match calc {
        Calc::Value(value) => leaf(value),
        // A bare number is not a length.
        Calc::Number(_) => None,
        Calc::Sum(a, b) => Some(evaluate(a, leaf)? + evaluate(b, leaf)?),
        Calc::Product(factor, value) => Some(evaluate(value, leaf)?.scale(*factor)),
        Calc::Function(function) => match function.as_ref() {
            MathFunction::Calc(calc) => evaluate(calc, leaf),
            MathFunction::Min(values) => compare(values, leaf, f32::min),
            MathFunction::Max(values) => compare(values, leaf, f32::max),
            MathFunction::Clamp(min, value, max) => {
                let [min, value, max] = same_kind([
                    evaluate(min, leaf)?,
                    evaluate(value, leaf)?,
                    evaluate(max, leaf)?,
                ])?;
                // clamp(MIN, VAL, MAX) = max(MIN, min(VAL, MAX)).
                Some(rebuild(min.0.max(value.0.min(max.0)), value.1))
            }
            MathFunction::Abs(value) => {
                let [value] = same_kind([evaluate(value, leaf)?])?;
                Some(rebuild(value.0.abs(), value.1))
            }
            MathFunction::Hypot(values) => {
                let values = values
                    .iter()
                    .map(|value| evaluate(value, leaf))
                    .collect::<Option<Vec<_>>>()?;
                let kind = kind_of(*values.first()?)?.1;
                let mut sum = 0.0_f32;
                for value in values {
                    let (number, value_kind) = kind_of(value)?;
                    if value_kind != kind {
                        return None;
                    }
                    sum += number * number;
                }
                Some(rebuild(sum.sqrt(), kind))
            }
            MathFunction::Round(strategy, value, step) => {
                use lightningcss::values::calc::RoundingStrategy;
                let [value, step] = same_kind([evaluate(value, leaf)?, evaluate(step, leaf)?])?;
                if step.0 == 0.0 {
                    return None;
                }
                let quotient = value.0 / step.0;
                let rounded = match strategy {
                    RoundingStrategy::Nearest => quotient.round(),
                    RoundingStrategy::Up => quotient.ceil(),
                    RoundingStrategy::Down => quotient.floor(),
                    RoundingStrategy::ToZero => quotient.trunc(),
                };
                Some(rebuild(rounded * step.0, value.1))
            }
            MathFunction::Rem(value, divisor) => {
                let [value, divisor] =
                    same_kind([evaluate(value, leaf)?, evaluate(divisor, leaf)?])?;
                (divisor.0 != 0.0).then(|| rebuild(value.0 % divisor.0, value.1))
            }
            MathFunction::Mod(value, divisor) => {
                let [value, divisor] =
                    same_kind([evaluate(value, leaf)?, evaluate(divisor, leaf)?])?;
                (divisor.0 != 0.0).then(|| rebuild(value.0.rem_euclid(divisor.0), value.1))
            }
            // A sign is a number, not a length.
            MathFunction::Sign(_) => None,
        },
    }
}

/// Which part a single-part length uses (an index into its parts).
type Kind = usize;

/// The value and part of a length that uses at most one part. Comparisons
/// and stepped functions need operands of one kind.
fn kind_of(value: LengthPercentage) -> Option<(f32, Kind)> {
    let parts = value.parts();
    let mut used = parts.iter().enumerate().filter(|(_, part)| **part != 0.0);
    match (used.next(), used.next()) {
        (None, _) => Some((0.0, 0)),
        (Some((kind, number)), None) => Some((*number, kind)),
        (Some(_), Some(_)) => None,
    }
}

/// Operands of mixed kinds with no percentage part, settled into pixels at
/// the default runtime bases. GPUI has no math functions, so `min(10rem,
/// 300px)` must be decided statically, as at the initial rem size.
fn settled<const N: usize>(values: [LengthPercentage; N]) -> Option<[LengthPercentage; N]> {
    values
        .iter()
        .all(|value| value.fraction == 0.0)
        .then(|| values.map(|value| LengthPercentage::px(value.resolve(&Bases::default(), 0.0))))
}

fn same_kind<const N: usize>(values: [LengthPercentage; N]) -> Option<[(f32, Kind); N]> {
    exact_kind(values).or_else(|| exact_kind(settled(values)?))
}

fn exact_kind<const N: usize>(values: [LengthPercentage; N]) -> Option<[(f32, Kind); N]> {
    let mut out = [(0.0, 0); N];
    let mut shared: Option<Kind> = None;
    for (index, value) in values.into_iter().enumerate() {
        let (number, kind) = kind_of(value)?;
        // Zero fits either kind.
        if number != 0.0 {
            if shared.is_some_and(|shared| shared != kind) {
                return None;
            }
            shared = Some(kind);
        }
        out[index] = (number, kind);
    }
    let shared = shared.unwrap_or(0);
    Some(out.map(|(number, _)| (number, shared)))
}

fn rebuild(number: f32, kind: Kind) -> LengthPercentage {
    let mut parts = [0.0; 7];
    parts[kind] = number;
    LengthPercentage::from_parts(parts)
}

fn compare<V>(
    values: &[Calc<V>],
    leaf: &dyn Fn(&V) -> Option<LengthPercentage>,
    pick: fn(f32, f32) -> f32,
) -> Option<LengthPercentage> {
    let values = values
        .iter()
        .map(|value| evaluate(value, leaf))
        .collect::<Option<Vec<_>>>()?;
    let mut kinds = values.iter().map(|value| kind_of(*value));
    let mixed = kinds.clone().any(|kind| kind.is_none())
        || kinds
            .by_ref()
            .flatten()
            .filter(|(number, _)| *number != 0.0)
            .map(|(_, kind)| kind)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            > 1;
    let values = if mixed {
        values
            .iter()
            .map(|value| settled([*value]).map(|[value]| value))
            .collect::<Option<Vec<_>>>()?
    } else {
        values
    };
    let mut shared: Option<Kind> = None;
    let mut result: Option<f32> = None;
    for value in values {
        let (number, kind) = kind_of(value)?;
        if number != 0.0 {
            if shared.is_some_and(|shared| shared != kind) {
                return None;
            }
            shared = Some(kind);
        }
        result = Some(result.map_or(number, |current| pick(current, number)));
    }
    Some(rebuild(result?, shared.unwrap_or(0)))
}
