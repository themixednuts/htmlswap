//! Target-neutral CSS motion: easing functions, transitions, animations and
//! their timing model.
//!
//! Values are parsed with Lightning CSS, so shorthands, longhands, list
//! repetition and keywords follow the same rules a browser applies. Renderers
//! read [`transitions`] and [`animations`] from an element's cascaded
//! declarations and drive them with their own clock.

use compact_str::CompactString;
use lightningcss::properties::animation::{
    AnimationDirection as CssAnimationDirection, AnimationFillMode as CssFillMode,
    AnimationIterationCount, AnimationName,
};
use lightningcss::properties::{Property, PropertyId};
use lightningcss::stylesheet::{ParserOptions, PrinterOptions};
use lightningcss::traits::ToCss;
use lightningcss::values::easing::{EasingFunction, StepPosition as CssStepPosition};
use lightningcss::values::time::Time;

use crate::style::StyleDeclaration;

/// A CSS easing function.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Easing {
    /// `linear`.
    Linear,
    /// `cubic-bezier(x1, y1, x2, y2)`, which also expresses `ease`,
    /// `ease-in`, `ease-out` and `ease-in-out`.
    CubicBezier(f32, f32, f32, f32),
    /// `steps(count, position)`.
    Steps(u32, StepPosition),
}

/// Where the jumps of a `steps()` easing fall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepPosition {
    /// `jump-start` / `start`.
    JumpStart,
    /// `jump-end` / `end`.
    JumpEnd,
    /// `jump-none`.
    JumpNone,
    /// `jump-both`.
    JumpBoth,
}

impl Easing {
    /// `ease`, the initial value of every timing-function property.
    pub const EASE: Self = Self::CubicBezier(0.25, 0.1, 0.25, 1.0);
    /// `ease-in`.
    pub const EASE_IN: Self = Self::CubicBezier(0.42, 0.0, 1.0, 1.0);
    /// `ease-out`.
    pub const EASE_OUT: Self = Self::CubicBezier(0.0, 0.0, 0.58, 1.0);
    /// `ease-in-out`.
    pub const EASE_IN_OUT: Self = Self::CubicBezier(0.42, 0.0, 0.58, 1.0);

    /// Parse one `<easing-function>`.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match Property::parse_string(
            PropertyId::from("transition-timing-function"),
            value,
            ParserOptions::default(),
        )
        .ok()?
        {
            Property::TransitionTimingFunction(list, _) if list.len() == 1 => {
                Some(Self::from_css(&list[0]))
            }
            _ => None,
        }
    }

    fn from_css(easing: &EasingFunction) -> Self {
        match easing {
            EasingFunction::Linear => Self::Linear,
            EasingFunction::Ease => Self::EASE,
            EasingFunction::EaseIn => Self::EASE_IN,
            EasingFunction::EaseOut => Self::EASE_OUT,
            EasingFunction::EaseInOut => Self::EASE_IN_OUT,
            EasingFunction::CubicBezier { x1, y1, x2, y2 } => Self::CubicBezier(*x1, *y1, *x2, *y2),
            EasingFunction::Steps { count, position } => Self::Steps(
                u32::try_from(*count).unwrap_or(1).max(1),
                match position {
                    CssStepPosition::Start => StepPosition::JumpStart,
                    CssStepPosition::End => StepPosition::JumpEnd,
                    CssStepPosition::JumpNone => StepPosition::JumpNone,
                    CssStepPosition::JumpBoth => StepPosition::JumpBoth,
                },
            ),
        }
    }

    /// The eased output for an input progress, which is clamped to `0..=1`.
    ///
    /// The output may leave `0..=1` for cubic Béziers that overshoot.
    #[must_use]
    pub fn sample(self, progress: f32) -> f32 {
        let t = progress.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::CubicBezier(x1, y1, x2, y2) => cubic_bezier(x1, y1, x2, y2, t),
            Self::Steps(count, position) => steps(count, position, t),
        }
    }
}

impl Default for Easing {
    fn default() -> Self {
        Self::EASE
    }
}

/// Solve `x(s) = t` on the curve through (0,0), (x1,y1), (x2,y2), (1,1) and
/// return `y(s)`: Newton's method, falling back to bisection.
fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    if t <= 0.0 || t >= 1.0 {
        return t;
    }
    let curve = |a: f32, b: f32, s: f32| {
        let inverse = 1.0 - s;
        3.0 * inverse * inverse * s * a + 3.0 * inverse * s * s * b + s * s * s
    };
    let slope = |a: f32, b: f32, s: f32| {
        let inverse = 1.0 - s;
        3.0 * inverse * inverse * a + 6.0 * inverse * s * (b - a) + 3.0 * s * s * (1.0 - b)
    };
    let mut s = t;
    for _ in 0..8 {
        let error = curve(x1, x2, s) - t;
        if error.abs() < 1e-6 {
            return curve(y1, y2, s);
        }
        let derivative = slope(x1, x2, s);
        if derivative.abs() < 1e-6 {
            break;
        }
        s -= error / derivative;
    }
    let (mut low, mut high) = (0.0_f32, 1.0_f32);
    s = t;
    for _ in 0..32 {
        let x = curve(x1, x2, s);
        if (x - t).abs() < 1e-6 {
            break;
        }
        if x < t {
            low = s;
        } else {
            high = s;
        }
        s = (low + high) / 2.0;
    }
    curve(y1, y2, s)
}

#[allow(clippy::cast_precision_loss)]
fn steps(count: u32, position: StepPosition, t: f32) -> f32 {
    let count = count as f32;
    let (jumps, offset) = match position {
        StepPosition::JumpStart => (count, 1.0),
        StepPosition::JumpEnd => (count, 0.0),
        StepPosition::JumpNone => ((count - 1.0).max(1.0), 0.0),
        StepPosition::JumpBoth => (count + 1.0, 1.0),
    };
    let step = (t * count).floor() + offset;
    let step = if t >= 1.0 { jumps } else { step };
    (step / jumps).clamp(0.0, 1.0)
}

/// Which property a transition applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionProperty {
    /// `all`.
    All,
    /// One property, by its CSS name.
    Property(CompactString),
}

impl TransitionProperty {
    /// Whether this transition covers `property`, a CSS property name.
    #[must_use]
    pub fn covers(&self, property: &str) -> bool {
        match self {
            Self::All => true,
            Self::Property(name) => name == property || shorthand_covers(name, property),
        }
    }
}

fn shorthand_covers(shorthand: &str, longhand: &str) -> bool {
    const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];
    const CORNERS: [&str; 4] = ["top-left", "top-right", "bottom-right", "bottom-left"];
    let side = |prefix: &str, suffix: &str| {
        SIDES.iter().any(|side| {
            longhand
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_suffix(suffix))
                == Some(side)
        })
    };
    match shorthand {
        "background" => longhand == "background-color",
        "border" => {
            side("border-", "-color") || side("border-", "-width") || side("border-", "-style")
        }
        "border-color" => side("border-", "-color"),
        "border-width" => side("border-", "-width"),
        "border-style" => side("border-", "-style"),
        "border-top" | "border-right" | "border-bottom" | "border-left" => longhand
            .strip_prefix(shorthand)
            .is_some_and(|rest| matches!(rest, "-color" | "-width" | "-style")),
        "border-radius" => CORNERS.iter().any(|corner| {
            longhand
                .strip_prefix("border-")
                .and_then(|rest| rest.strip_suffix("-radius"))
                == Some(corner)
        }),
        "inset" => SIDES.contains(&longhand),
        "margin" => side("margin-", ""),
        "padding" => side("padding-", ""),
        "gap" => matches!(longhand, "row-gap" | "column-gap"),
        "flex" => matches!(longhand, "flex-grow" | "flex-shrink" | "flex-basis"),
        "font" => matches!(
            longhand,
            "font-size" | "font-weight" | "font-style" | "line-height" | "font-family"
        ),
        "text-decoration" => matches!(
            longhand,
            "text-decoration-color"
                | "text-decoration-thickness"
                | "text-decoration-line"
                | "text-decoration-style"
        ),
        _ => false,
    }
}

/// One entry of an element's `transition`.
#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    /// The property it applies to.
    pub property: TransitionProperty,
    /// `transition-duration`, in milliseconds.
    pub duration_ms: f32,
    /// `transition-delay`, in milliseconds; negative delays start part-way.
    pub delay_ms: f32,
    /// `transition-timing-function`.
    pub easing: Easing,
}

impl Transition {
    /// Whether this transition runs at all: CSS ignores entries whose
    /// combined duration is zero.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.duration_ms + self.delay_ms.max(0.0) > 0.0
    }

    /// Eased progress `elapsed_ms` after the transition started, or `None`
    /// once it has finished.
    #[must_use]
    pub fn progress(&self, elapsed_ms: f32) -> Option<f32> {
        let local = elapsed_ms - self.delay_ms;
        if self.duration_ms <= 0.0 {
            return (local < 0.0).then_some(0.0);
        }
        if local >= self.duration_ms {
            return None;
        }
        Some(self.easing.sample((local / self.duration_ms).max(0.0)))
    }

    /// The transition for `property`, if one covers it. Like a browser, the
    /// last matching entry wins.
    #[must_use]
    pub fn for_property<'a>(transitions: &'a [Self], property: &str) -> Option<&'a Self> {
        transitions
            .iter()
            .rev()
            .find(|transition| transition.property.covers(property))
    }
}

/// An element's transitions from its cascaded declarations, in the order a
/// browser applies them: later declarations win, and each longhand list is
/// repeated or truncated to the length of `transition-property`.
#[must_use]
pub fn transitions<'a>(
    declarations: impl IntoIterator<Item = &'a StyleDeclaration>,
) -> Vec<Transition> {
    let mut properties: Option<Vec<Option<TransitionProperty>>> = None;
    let mut durations = vec![0.0_f32];
    let mut delays = vec![0.0_f32];
    let mut easings = vec![Easing::EASE];
    for declaration in declarations {
        let name = declaration.property.as_str();
        if !name.starts_with("transition") {
            continue;
        }
        let Ok(property) = Property::parse_string(
            PropertyId::from(name),
            declaration.value.as_str(),
            ParserOptions::default(),
        ) else {
            continue;
        };
        match property {
            Property::Transition(list, _) => {
                properties = Some(
                    list.iter()
                        .map(|t| transition_property(&t.property))
                        .collect(),
                );
                durations = list.iter().map(|t| t.duration.to_ms()).collect();
                delays = list.iter().map(|t| t.delay.to_ms()).collect();
                easings = list
                    .iter()
                    .map(|t| Easing::from_css(&t.timing_function))
                    .collect();
            }
            Property::TransitionProperty(list, _) => {
                properties = Some(list.iter().map(transition_property).collect());
            }
            Property::TransitionDuration(list, _) => durations = times(&list),
            Property::TransitionDelay(list, _) => delays = times(&list),
            Property::TransitionTimingFunction(list, _) => {
                easings = list.iter().map(Easing::from_css).collect();
            }
            _ => {}
        }
    }
    let Some(properties) = properties else {
        // `transition-property` defaults to `all`.
        return single_all(&durations, &delays, &easings);
    };
    properties
        .into_iter()
        .enumerate()
        .filter_map(|(index, property)| {
            Some(Transition {
                property: property?,
                duration_ms: repeat(&durations, index, 0.0),
                delay_ms: repeat(&delays, index, 0.0),
                easing: repeat(&easings, index, Easing::EASE),
            })
        })
        .filter(Transition::is_active)
        .collect()
}

fn single_all(durations: &[f32], delays: &[f32], easings: &[Easing]) -> Vec<Transition> {
    let transition = Transition {
        property: TransitionProperty::All,
        duration_ms: repeat(durations, 0, 0.0),
        delay_ms: repeat(delays, 0, 0.0),
        easing: repeat(easings, 0, Easing::EASE),
    };
    if transition.is_active() {
        vec![transition]
    } else {
        Vec::new()
    }
}

fn transition_property(property: &PropertyId<'_>) -> Option<TransitionProperty> {
    let name = property.to_css_string(PrinterOptions::default()).ok()?;
    match name.as_str() {
        "none" => None,
        "all" => Some(TransitionProperty::All),
        _ => Some(TransitionProperty::Property(name.into())),
    }
}

fn times(list: &[Time]) -> Vec<f32> {
    list.iter().map(Time::to_ms).collect()
}

fn repeat<T: Copy>(list: &[T], index: usize, default: T) -> T {
    if list.is_empty() {
        default
    } else {
        list[index % list.len()]
    }
}

/// How many times an animation runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Iterations {
    /// A finite, possibly fractional, count.
    Count(f32),
    /// `infinite`.
    Infinite,
}

/// `animation-direction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnimationDirection {
    /// `normal`.
    #[default]
    Normal,
    /// `reverse`.
    Reverse,
    /// `alternate`.
    Alternate,
    /// `alternate-reverse`.
    AlternateReverse,
}

/// `animation-fill-mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillMode {
    /// `none`.
    #[default]
    None,
    /// `forwards`.
    Forwards,
    /// `backwards`.
    Backwards,
    /// `both`.
    Both,
}

/// One entry of an element's `animation`.
#[derive(Debug, Clone, PartialEq)]
pub struct Animation {
    /// The `@keyframes` name, or `None` for `animation-name: none`.
    pub name: Option<CompactString>,
    /// `animation-duration`, in milliseconds.
    pub duration_ms: f32,
    /// `animation-delay`, in milliseconds.
    pub delay_ms: f32,
    /// `animation-timing-function`, applied between keyframes.
    pub easing: Easing,
    /// `animation-iteration-count`.
    pub iterations: Iterations,
    /// `animation-direction`.
    pub direction: AnimationDirection,
    /// `animation-fill-mode`.
    pub fill_mode: FillMode,
}

/// Where an animation is `elapsed` milliseconds after it started.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationPhase {
    /// Keyframe progress in `0..=1` after applying direction, before easing.
    /// Easing applies between each pair of keyframes.
    pub progress: f32,
    /// Whether the animation is past its end (and applies only through fill).
    pub finished: bool,
}

impl Animation {
    /// The end of the active interval, or `None` for infinite animations.
    #[must_use]
    pub fn end_ms(&self) -> Option<f32> {
        match self.iterations {
            Iterations::Count(count) => Some(self.delay_ms + self.duration_ms * count.max(0.0)),
            Iterations::Infinite => None,
        }
    }

    /// The animation's phase, or `None` when it has no effect at that time
    /// (before its delay without backwards fill, or after its end without
    /// forwards fill).
    #[must_use]
    pub fn phase(&self, elapsed_ms: f32) -> Option<AnimationPhase> {
        let local = elapsed_ms - self.delay_ms;
        let fills_backwards = matches!(self.fill_mode, FillMode::Backwards | FillMode::Both);
        let fills_forwards = matches!(self.fill_mode, FillMode::Forwards | FillMode::Both);
        if local < 0.0 {
            return fills_backwards.then(|| AnimationPhase {
                progress: self.directed(0, 0.0),
                finished: false,
            });
        }
        let count = match self.iterations {
            Iterations::Count(count) => count.max(0.0),
            Iterations::Infinite => f32::INFINITY,
        };
        let duration = self.duration_ms.max(0.0);
        let active = duration * count;
        if local >= active {
            if !fills_forwards {
                return None;
            }
            // The final iteration ends wherever its fraction ends.
            let ended = if count == 0.0 { 0.0 } else { count };
            let iteration = (ended.ceil() as u32).saturating_sub(1);
            let fraction = if ended.fract() == 0.0 {
                1.0
            } else {
                ended.fract()
            };
            return Some(AnimationPhase {
                progress: self.directed(iteration, fraction),
                finished: true,
            });
        }
        if duration == 0.0 {
            return Some(AnimationPhase {
                progress: self.directed(0, 1.0),
                finished: false,
            });
        }
        let position = local / duration;
        let iteration = position.floor();
        Some(AnimationPhase {
            progress: self.directed(iteration as u32, position - iteration),
            finished: false,
        })
    }

    fn directed(&self, iteration: u32, fraction: f32) -> f32 {
        let reversed = match self.direction {
            AnimationDirection::Normal => false,
            AnimationDirection::Reverse => true,
            AnimationDirection::Alternate => iteration % 2 == 1,
            AnimationDirection::AlternateReverse => iteration % 2 == 0,
        };
        if reversed { 1.0 - fraction } else { fraction }
    }
}

/// An element's animations from its cascaded declarations, longhand lists
/// repeated to the length of `animation-name`.
#[must_use]
pub fn animations<'a>(
    declarations: impl IntoIterator<Item = &'a StyleDeclaration>,
) -> Vec<Animation> {
    let mut names: Vec<Option<CompactString>> = Vec::new();
    let mut durations = vec![0.0_f32];
    let mut delays = vec![0.0_f32];
    let mut easings = vec![Easing::EASE];
    let mut iterations = vec![Iterations::Count(1.0)];
    let mut directions = vec![AnimationDirection::Normal];
    let mut fills = vec![FillMode::None];
    for declaration in declarations {
        let name = declaration.property.as_str();
        if !name.starts_with("animation") {
            continue;
        }
        let Ok(property) = Property::parse_string(
            PropertyId::from(name),
            declaration.value.as_str(),
            ParserOptions::default(),
        ) else {
            continue;
        };
        match property {
            Property::Animation(list, _) => {
                names = list.iter().map(|a| animation_name(&a.name)).collect();
                durations = list.iter().map(|a| a.duration.to_ms()).collect();
                delays = list.iter().map(|a| a.delay.to_ms()).collect();
                easings = list
                    .iter()
                    .map(|a| Easing::from_css(&a.timing_function))
                    .collect();
                iterations = list.iter().map(|a| iteration(&a.iteration_count)).collect();
                directions = list.iter().map(|a| direction(&a.direction)).collect();
                fills = list.iter().map(|a| fill(&a.fill_mode)).collect();
            }
            Property::AnimationName(list, _) => {
                names = list.iter().map(animation_name).collect();
            }
            Property::AnimationDuration(list, _) => durations = times(&list),
            Property::AnimationDelay(list, _) => delays = times(&list),
            Property::AnimationTimingFunction(list, _) => {
                easings = list.iter().map(Easing::from_css).collect();
            }
            Property::AnimationIterationCount(list, _) => {
                iterations = list.iter().map(iteration).collect();
            }
            Property::AnimationDirection(list, _) => {
                directions = list.iter().map(direction).collect();
            }
            Property::AnimationFillMode(list, _) => fills = list.iter().map(fill).collect(),
            _ => {}
        }
    }
    names
        .into_iter()
        .enumerate()
        .map(|(index, name)| Animation {
            name,
            duration_ms: repeat(&durations, index, 0.0),
            delay_ms: repeat(&delays, index, 0.0),
            easing: repeat(&easings, index, Easing::EASE),
            iterations: repeat(&iterations, index, Iterations::Count(1.0)),
            direction: repeat(&directions, index, AnimationDirection::Normal),
            fill_mode: repeat(&fills, index, FillMode::None),
        })
        .collect()
}

fn animation_name(name: &AnimationName<'_>) -> Option<CompactString> {
    match name {
        AnimationName::None => None,
        AnimationName::Ident(ident) => Some(ident.0.as_ref().into()),
        AnimationName::String(string) => Some(string.as_ref().into()),
    }
}

fn iteration(count: &AnimationIterationCount) -> Iterations {
    match count {
        AnimationIterationCount::Number(count) => Iterations::Count(*count),
        AnimationIterationCount::Infinite => Iterations::Infinite,
    }
}

fn direction(direction: &CssAnimationDirection) -> AnimationDirection {
    match direction {
        CssAnimationDirection::Normal => AnimationDirection::Normal,
        CssAnimationDirection::Reverse => AnimationDirection::Reverse,
        CssAnimationDirection::Alternate => AnimationDirection::Alternate,
        CssAnimationDirection::AlternateReverse => AnimationDirection::AlternateReverse,
    }
}

fn fill(fill: &CssFillMode) -> FillMode {
    match fill {
        CssFillMode::None => FillMode::None,
        CssFillMode::Forwards => FillMode::Forwards,
        CssFillMode::Backwards => FillMode::Backwards,
        CssFillMode::Both => FillMode::Both,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AnimationDirection, Easing, FillMode, Iterations, StepPosition, Transition,
        TransitionProperty, animations, transitions,
    };
    use crate::style::StyleDeclaration;

    fn declarations(pairs: &[(&str, &str)]) -> Vec<StyleDeclaration> {
        pairs
            .iter()
            .map(|(property, value)| StyleDeclaration::new(*property, *value, false, None))
            .collect()
    }

    fn close(left: f32, right: f32) -> bool {
        (left - right).abs() < 1e-3
    }

    #[test]
    fn easing_curves_match_css() {
        assert!(close(Easing::Linear.sample(0.3), 0.3));
        assert!(close(Easing::EASE.sample(0.5), 0.802_4));
        assert!(close(Easing::EASE_IN_OUT.sample(0.5), 0.5));
        assert!(close(Easing::EASE_IN.sample(1.0), 1.0));
        assert_eq!(
            Easing::parse("steps(4, jump-end)"),
            Some(Easing::Steps(4, StepPosition::JumpEnd))
        );
        assert!(close(
            Easing::Steps(4, StepPosition::JumpEnd).sample(0.3),
            0.25
        ));
        assert!(close(
            Easing::Steps(4, StepPosition::JumpStart).sample(0.3),
            0.5
        ));
        assert_eq!(Easing::parse("ease-out"), Some(Easing::EASE_OUT));
        assert_eq!(Easing::parse("bogus"), None);
    }

    #[test]
    fn transitions_follow_shorthand_and_longhand_cascade() {
        let list = transitions(&declarations(&[
            (
                "transition",
                "background-color 200ms ease-in, opacity 1s linear 100ms",
            ),
            ("transition-duration", "300ms"),
        ]));
        assert_eq!(list.len(), 2);
        assert_eq!(
            list[0].property,
            TransitionProperty::Property("background-color".into())
        );
        assert!(close(list[0].duration_ms, 300.0));
        assert!(close(list[1].duration_ms, 300.0), "a one-item list repeats");
        assert!(close(list[1].delay_ms, 100.0));
        assert_eq!(list[1].easing, Easing::Linear);

        let all = transitions(&declarations(&[("transition-duration", "150ms")]));
        assert_eq!(all[0].property, TransitionProperty::All);
        assert!(Transition::for_property(&all, "width").is_some());

        assert!(transitions(&declarations(&[("transition", "none")])).is_empty());
        assert!(
            transitions(&declarations(&[("transition-property", "opacity")])).is_empty(),
            "a zero duration transition does not run"
        );
        let shorthand = transitions(&declarations(&[("transition", "background 1s")]));
        assert!(Transition::for_property(&shorthand, "background-color").is_some());
    }

    #[test]
    fn transition_progress_honors_delay_and_easing() {
        let transition = Transition {
            property: TransitionProperty::All,
            duration_ms: 100.0,
            delay_ms: 50.0,
            easing: Easing::Linear,
        };
        assert_eq!(transition.progress(20.0), Some(0.0));
        assert!(transition.progress(100.0).is_some_and(|p| close(p, 0.5)));
        assert_eq!(transition.progress(150.0), None);
    }

    #[test]
    fn animations_parse_and_follow_the_timing_model() {
        let list = animations(&declarations(&[(
            "animation",
            "400ms ease-out 100ms 2 alternate both slide-in",
        )]));
        assert_eq!(list.len(), 1);
        let animation = &list[0];
        assert_eq!(animation.name.as_deref(), Some("slide-in"));
        assert_eq!(animation.iterations, Iterations::Count(2.0));
        assert_eq!(animation.direction, AnimationDirection::Alternate);
        assert_eq!(animation.fill_mode, FillMode::Both);
        assert!(close(animation.end_ms().unwrap_or_default(), 900.0));

        let before = animation.phase(0.0).map(|phase| phase.progress);
        assert_eq!(before, Some(0.0), "backwards fill holds the first keyframe");
        let middle = animation.phase(300.0).map(|phase| phase.progress);
        assert!(middle.is_some_and(|p| close(p, 0.5)));
        let second = animation.phase(600.0).map(|phase| phase.progress);
        assert!(
            second.is_some_and(|p| close(p, 0.75)),
            "alternate runs backwards"
        );
        let after = animation.phase(1000.0);
        assert!(after.is_some_and(|phase| phase.finished && close(phase.progress, 0.0)));

        let none = animations(&declarations(&[("animation-name", "none")]));
        assert_eq!(none[0].name, None);
        assert!(none[0].phase(10.0).is_none());
    }
}

#[cfg(test)]
mod shorthand_tests {
    use super::TransitionProperty;

    #[test]
    fn shorthands_cover_their_longhands() {
        let property = |name: &str| TransitionProperty::Property(name.into());
        assert!(property("border-color").covers("border-left-color"));
        assert!(property("border").covers("border-top-width"));
        assert!(property("border-top").covers("border-top-color"));
        assert!(!property("border-top").covers("border-left-color"));
        assert!(property("border-radius").covers("border-bottom-right-radius"));
        assert!(property("margin").covers("margin-left"));
        assert!(!property("margin").covers("margin-inline"));
        assert!(property("gap").covers("column-gap"));
        assert!(property("inset").covers("left"));
        assert!(!property("background").covers("color"));
    }
}
