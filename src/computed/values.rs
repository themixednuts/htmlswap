//! Lengths: every CSS unit and math function, resolved against the
//! element's font sizes and the viewport.

use lightningcss::values::calc::{Calc, MathFunction};
use lightningcss::values::length::{
    Length as CssLength, LengthPercentage as CssLengthPercentage, LengthValue,
};
use lightningcss::values::percentage::DimensionPercentage;

/// What relative units resolve against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValueContext {
    /// The element's computed `font-size`, for `em` and friends.
    pub font_size: f32,
    /// The root element's computed `font-size`, for `rem` and friends.
    pub root_font_size: f32,
    pub viewport_width: f32,
    pub viewport_height: f32,
}

impl Default for ValueContext {
    fn default() -> Self {
        Self {
            font_size: 16.0,
            root_font_size: 16.0,
            viewport_width: 1024.0,
            viewport_height: 768.0,
        }
    }
}

/// A length-percentage after resolving units: `px + fraction × basis`,
/// where the basis is whatever the property's percentages refer to. `calc()`
/// mixing both keeps both parts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LengthPercentage {
    pub px: f32,
    pub fraction: f32,
}

impl LengthPercentage {
    pub const ZERO: Self = Self {
        px: 0.0,
        fraction: 0.0,
    };

    #[must_use]
    pub const fn px(px: f32) -> Self {
        Self { px, fraction: 0.0 }
    }

    #[must_use]
    pub const fn fraction(fraction: f32) -> Self {
        Self { px: 0.0, fraction }
    }

    /// The pixel length when no percentage is involved.
    #[must_use]
    pub fn as_px(self) -> Option<f32> {
        (self.fraction == 0.0).then_some(self.px)
    }

    /// The fraction when no absolute length is involved.
    #[must_use]
    pub fn as_fraction(self) -> Option<f32> {
        (self.px == 0.0).then_some(self.fraction)
    }

    /// Resolve against a basis.
    #[must_use]
    pub fn resolve(self, basis: f32) -> f32 {
        self.px + self.fraction * basis
    }

    #[must_use]
    pub fn lerp(self, to: Self, t: f32) -> Self {
        Self {
            px: self.px + (to.px - self.px) * t,
            fraction: self.fraction + (to.fraction - self.fraction) * t,
        }
    }

    fn add(self, other: Self) -> Self {
        Self {
            px: self.px + other.px,
            fraction: self.fraction + other.fraction,
        }
    }

    fn scale(self, factor: f32) -> Self {
        Self {
            px: self.px * factor,
            fraction: self.fraction * factor,
        }
    }
}

/// A length-percentage or `auto`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LengthAuto {
    Auto,
    Length(LengthPercentage),
}

/// One length unit in pixels.
pub(crate) fn length_value(value: &LengthValue, context: &ValueContext) -> Option<f32> {
    use LengthValue::{
        Cap, Ch, Cqb, Cqh, Cqi, Cqmax, Cqmin, Cqw, Dvb, Dvh, Dvi, Dvmax, Dvmin, Dvw, Em, Ex, Ic,
        Lh, Lvb, Lvh, Lvi, Lvmax, Lvmin, Lvw, Rcap, Rch, Rem, Rex, Ric, Rlh, Svb, Svh, Svi, Svmax,
        Svmin, Svw, Vb, Vh, Vi, Vmax, Vmin, Vw,
    };
    if let Some(px) = value.to_px() {
        return Some(px);
    }
    let em = context.font_size;
    let rem = context.root_font_size;
    let width = context.viewport_width / 100.0;
    let height = context.viewport_height / 100.0;
    // Without font metrics, CSS Values 4 §6.1.1 assigns these fallbacks.
    Some(match value {
        Em(v) => v * em,
        Rem(v) => v * rem,
        Ex(v) | Cap(v) => v * em * 0.5,
        Rex(v) | Rcap(v) => v * rem * 0.5,
        Ch(v) => v * em * 0.5,
        Rch(v) => v * rem * 0.5,
        Ic(v) => v * em,
        Ric(v) => v * rem,
        Lh(v) => v * em * 1.2,
        Rlh(v) => v * rem * 1.2,
        // The window is the viewport: large, small and dynamic agree, and
        // there are no size containers, so container units use it too.
        Vw(v) | Lvw(v) | Svw(v) | Dvw(v) | Cqw(v) | Vi(v) | Lvi(v) | Svi(v) | Dvi(v) | Cqi(v) => {
            v * width
        }
        Vh(v) | Lvh(v) | Svh(v) | Dvh(v) | Cqh(v) | Vb(v) | Lvb(v) | Svb(v) | Dvb(v) | Cqb(v) => {
            v * height
        }
        Vmin(v) | Lvmin(v) | Svmin(v) | Dvmin(v) | Cqmin(v) => v * width.min(height),
        Vmax(v) | Lvmax(v) | Svmax(v) | Dvmax(v) | Cqmax(v) => v * width.max(height),
        _ => return None,
    })
    .filter(|px| px.is_finite())
}

/// A `<length>` in pixels.
pub(crate) fn length(value: &CssLength, context: &ValueContext) -> Option<f32> {
    match value {
        CssLength::Value(value) => length_value(value, context),
        CssLength::Calc(calc) => calc_length(calc, context)?.as_px(),
    }
}

fn calc_length(calc: &Calc<CssLength>, context: &ValueContext) -> Option<LengthPercentage> {
    evaluate(calc, &|value: &CssLength| {
        length(value, context).map(LengthPercentage::px)
    })
}

/// A `<length-percentage>`.
pub(crate) fn length_percentage(
    value: &CssLengthPercentage,
    context: &ValueContext,
) -> Option<LengthPercentage> {
    match value {
        DimensionPercentage::Dimension(value) => {
            length_value(value, context).map(LengthPercentage::px)
        }
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
        Calc::Sum(a, b) => Some(evaluate(a, leaf)?.add(evaluate(b, leaf)?)),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Px,
    Fraction,
}

fn kind_of(value: LengthPercentage) -> Option<(f32, Kind)> {
    match (value.px == 0.0, value.fraction == 0.0) {
        (_, true) => Some((value.px, Kind::Px)),
        (true, false) => Some((value.fraction, Kind::Fraction)),
        (false, false) => None,
    }
}

fn same_kind<const N: usize>(values: [LengthPercentage; N]) -> Option<[(f32, Kind); N]> {
    let mut out = [(0.0, Kind::Px); N];
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
    let shared = shared.unwrap_or(Kind::Px);
    Some(out.map(|(number, _)| (number, shared)))
}

fn rebuild(number: f32, kind: Kind) -> LengthPercentage {
    match kind {
        Kind::Px => LengthPercentage::px(number),
        Kind::Fraction => LengthPercentage::fraction(number),
    }
}

fn compare<V>(
    values: &[Calc<V>],
    leaf: &dyn Fn(&V) -> Option<LengthPercentage>,
    pick: fn(f32, f32) -> f32,
) -> Option<LengthPercentage> {
    let mut shared: Option<Kind> = None;
    let mut result: Option<f32> = None;
    for value in values {
        let (number, kind) = kind_of(evaluate(value, leaf)?)?;
        if number != 0.0 {
            if shared.is_some_and(|shared| shared != kind) {
                return None;
            }
            shared = Some(kind);
        }
        result = Some(result.map_or(number, |current| pick(current, number)));
    }
    Some(rebuild(result?, shared.unwrap_or(Kind::Px)))
}
