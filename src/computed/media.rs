//! Media queries, evaluated against a renderer's environment.

use lightningcss::media_query::{
    MediaCondition, MediaFeatureComparison, MediaFeatureId, MediaFeatureName, MediaFeatureValue,
    MediaList, MediaQuery, MediaType, Operator, Qualifier, QueryFeature,
};
use lightningcss::stylesheet::ParserOptions;

use super::ColorScheme;
use super::values::{ValueContext, length};

/// What media queries are evaluated against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MediaEnvironment {
    pub width: f32,
    pub height: f32,
    /// The root element's font size, which `em` in media queries uses.
    pub font_size: f32,
    pub color_scheme: ColorScheme,
    pub reduced_motion: bool,
    /// Whether the primary pointer can hover.
    pub hover: bool,
    /// Whether the primary pointer is fine (a mouse rather than a finger).
    pub fine_pointer: bool,
}

impl Default for MediaEnvironment {
    fn default() -> Self {
        Self {
            width: 1024.0,
            height: 768.0,
            font_size: 16.0,
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            hover: true,
            fine_pointer: true,
        }
    }
}

impl MediaEnvironment {
    /// Whether a media query list matches. `None` when it uses a feature this
    /// environment cannot answer, or does not parse.
    #[must_use]
    pub fn matches(&self, query: &str) -> Option<bool> {
        let mut input = cssparser::ParserInput::new(query);
        let mut parser = cssparser::Parser::new(&mut input);
        let list = MediaList::parse(&mut parser, &ParserOptions::default()).ok()?;
        parser.expect_exhausted().ok()?;
        if list.media_queries.is_empty() {
            return Some(true);
        }
        let mut any = false;
        for query in &list.media_queries {
            any |= self.query(query)?;
        }
        Some(any)
    }

    fn query(&self, query: &MediaQuery<'_>) -> Option<bool> {
        let type_matches = match &query.media_type {
            MediaType::All | MediaType::Screen => true,
            MediaType::Print | MediaType::Custom(_) => false,
        };
        let condition = match &query.condition {
            Some(condition) => self.condition(condition)?,
            None => true,
        };
        let matches = type_matches && condition;
        Some(if query.qualifier == Some(Qualifier::Not) {
            !matches
        } else {
            matches
        })
    }

    fn condition(&self, condition: &MediaCondition<'_>) -> Option<bool> {
        match condition {
            MediaCondition::Feature(feature) => self.feature(feature),
            MediaCondition::Not(condition) => self.condition(condition).map(|matches| !matches),
            MediaCondition::Operation {
                operator,
                conditions,
            } => {
                let mut results = conditions.iter().map(|condition| self.condition(condition));
                match operator {
                    Operator::And => results.try_fold(true, |all, matches| Some(all && matches?)),
                    Operator::Or => results.try_fold(false, |any, matches| Some(any || matches?)),
                }
            }
            MediaCondition::Unknown(_) => None,
        }
    }

    fn feature(&self, feature: &QueryFeature<'_, MediaFeatureId>) -> Option<bool> {
        match feature {
            QueryFeature::Plain { name, value } => {
                let id = standard(name)?;
                match id {
                    MediaFeatureId::Width
                    | MediaFeatureId::Height
                    | MediaFeatureId::AspectRatio => Some(compare(
                        self.number(id)?,
                        self.value(id, value)?,
                        MediaFeatureComparison::Equal,
                    )),
                    _ => Some(self.keyword(id)? == ident(value)?),
                }
            }
            QueryFeature::Boolean { name } => {
                let id = standard(name)?;
                Some(match id {
                    MediaFeatureId::Width | MediaFeatureId::Height => self.number(id)? != 0.0,
                    MediaFeatureId::Hover | MediaFeatureId::AnyHover => self.hover,
                    MediaFeatureId::Pointer | MediaFeatureId::AnyPointer => true,
                    MediaFeatureId::PrefersReducedMotion => self.reduced_motion,
                    MediaFeatureId::Color => true,
                    MediaFeatureId::PrefersColorScheme | MediaFeatureId::Orientation => true,
                    _ => return None,
                })
            }
            QueryFeature::Range {
                name,
                operator,
                value,
            } => {
                let id = standard(name)?;
                Some(compare(self.number(id)?, self.value(id, value)?, *operator))
            }
            QueryFeature::Interval {
                name,
                start,
                start_operator,
                end,
                end_operator,
            } => {
                let id = standard(name)?;
                let actual = self.number(id)?;
                // `start OP name OP end`: the start comparison is written
                // from the start's side.
                Some(
                    compare(self.value(id, start)?, actual, *start_operator)
                        && compare(actual, self.value(id, end)?, *end_operator),
                )
            }
        }
    }

    /// The environment's value of a numeric feature.
    fn number(&self, id: MediaFeatureId) -> Option<f32> {
        match id {
            MediaFeatureId::Width | MediaFeatureId::DeviceWidth => Some(self.width),
            MediaFeatureId::Height | MediaFeatureId::DeviceHeight => Some(self.height),
            MediaFeatureId::AspectRatio | MediaFeatureId::DeviceAspectRatio => {
                (self.height > 0.0).then(|| self.width / self.height)
            }
            MediaFeatureId::Color => Some(8.0),
            MediaFeatureId::Monochrome => Some(0.0),
            _ => None,
        }
    }

    /// A query value for a numeric feature.
    fn value(&self, id: MediaFeatureId, value: &MediaFeatureValue<'_>) -> Option<f32> {
        let context = ValueContext {
            font_size: self.font_size,
            root_font_size: self.font_size,
            viewport_width: self.width,
            viewport_height: self.height,
        };
        match (id, value) {
            (_, MediaFeatureValue::Length(value)) => length(value, &context),
            (_, MediaFeatureValue::Ratio(ratio)) => (ratio.1 != 0.0).then(|| ratio.0 / ratio.1),
            (_, MediaFeatureValue::Number(number)) => Some(*number),
            (_, MediaFeatureValue::Integer(integer)) => Some(*integer as f32),
            _ => None,
        }
    }

    /// The environment's keyword for a discrete feature.
    fn keyword(&self, id: MediaFeatureId) -> Option<&'static str> {
        Some(match id {
            MediaFeatureId::Orientation => {
                if self.height > self.width {
                    "portrait"
                } else {
                    "landscape"
                }
            }
            MediaFeatureId::Hover | MediaFeatureId::AnyHover => {
                if self.hover {
                    "hover"
                } else {
                    "none"
                }
            }
            MediaFeatureId::Pointer | MediaFeatureId::AnyPointer => {
                if self.fine_pointer {
                    "fine"
                } else {
                    "coarse"
                }
            }
            MediaFeatureId::PrefersReducedMotion => {
                if self.reduced_motion {
                    "reduce"
                } else {
                    "no-preference"
                }
            }
            MediaFeatureId::PrefersColorScheme => match self.color_scheme {
                ColorScheme::Light => "light",
                ColorScheme::Dark => "dark",
            },
            MediaFeatureId::PrefersContrast => "no-preference",
            MediaFeatureId::PrefersReducedTransparency | MediaFeatureId::PrefersReducedData => {
                "no-preference"
            }
            MediaFeatureId::ForcedColors | MediaFeatureId::InvertedColors => "none",
            MediaFeatureId::Scripting => "enabled",
            MediaFeatureId::Update => "fast",
            MediaFeatureId::DisplayMode => "standalone",
            MediaFeatureId::ColorGamut | MediaFeatureId::VideoColorGamut => "srgb",
            MediaFeatureId::DynamicRange | MediaFeatureId::VideoDynamicRange => "standard",
            MediaFeatureId::OverflowBlock => "scroll",
            MediaFeatureId::OverflowInline => "scroll",
            _ => return None,
        })
    }
}

fn standard(name: &MediaFeatureName<'_, MediaFeatureId>) -> Option<MediaFeatureId> {
    match name {
        MediaFeatureName::Standard(id) => Some(*id),
        MediaFeatureName::Custom(_) | MediaFeatureName::Unknown(_) => None,
    }
}

fn ident<'a>(value: &'a MediaFeatureValue<'_>) -> Option<&'a str> {
    match value {
        MediaFeatureValue::Ident(ident) => Some(ident.0.as_ref()),
        _ => None,
    }
}

fn compare(left: f32, right: f32, operator: MediaFeatureComparison) -> bool {
    match operator {
        MediaFeatureComparison::Equal => (left - right).abs() < 1e-3,
        MediaFeatureComparison::GreaterThan => left > right,
        MediaFeatureComparison::GreaterThanEqual => left >= right - 1e-3,
        MediaFeatureComparison::LessThan => left < right,
        MediaFeatureComparison::LessThanEqual => left <= right + 1e-3,
    }
}

#[cfg(test)]
mod tests {
    use super::{ColorScheme, MediaEnvironment};

    #[test]
    fn queries_cover_ranges_logic_and_preferences() {
        let environment = MediaEnvironment {
            width: 800.0,
            height: 600.0,
            color_scheme: ColorScheme::Dark,
            ..MediaEnvironment::default()
        };
        let cases = [
            ("(min-width: 600px)", Some(true)),
            ("(max-width: 40em)", Some(false)),
            ("(width >= 50em)", Some(true)),
            ("(400px < width <= 800px)", Some(true)),
            ("screen and (orientation: landscape)", Some(true)),
            ("print", Some(false)),
            ("not print", Some(true)),
            ("(prefers-color-scheme: dark)", Some(true)),
            (
                "(prefers-color-scheme: light) or (min-width: 900px)",
                Some(false),
            ),
            ("not ((hover: none) or (pointer: coarse))", Some(true)),
            ("(prefers-reduced-motion: reduce)", Some(false)),
            ("(min-aspect-ratio: 4/3)", Some(true)),
            ("(min-width: calc(500px + 2em))", Some(true)),
            ("(min-width: 100px), (unknown-feature: x)", None),
        ];
        for (query, expected) in cases {
            assert_eq!(environment.matches(query), expected, "{query}");
        }
    }
}
