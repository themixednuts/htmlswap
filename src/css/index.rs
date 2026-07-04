use compact_str::CompactString;

use crate::plan::{
    RenderStyleCondition, RenderStyleVariant, RenderStylesheetRule, RenderThemePlan,
    RenderThemeScope,
};
use crate::style::{StyleDeclaration, StyleProperty, StyleValue};

use super::{CssRule, StyleElement, Stylesheet};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct StyleIndex {
    rules: Vec<CssRule>,
    theme: RenderThemePlan,
}

impl StyleIndex {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn extend_stylesheet(&mut self, stylesheet: Stylesheet) {
        for rule in &stylesheet.rules {
            self.collect_theme_rule(rule);
        }
        self.rules.extend(stylesheet.rules);
    }

    #[must_use]
    pub(crate) fn styles_for_element(&self, element: &StyleElement<'_, '_>) -> ElementStyles {
        let mut applied = Vec::<AppliedDeclaration>::new();
        let mut declaration_order = 0usize;
        let mut matched_rules = Vec::new();
        let mut variants = Vec::new();

        for (rule_source_order, rule) in self.rules.iter().enumerate() {
            if let Some(variant) = variant_for_rule(rule, element) {
                variants.push(variant);
            }

            if !rule.conditions.is_empty() {
                continue;
            }

            let Some(specificity) = rule.selector.matching_specificity(element) else {
                declaration_order += rule.declarations.len();
                continue;
            };

            matched_rules.push(RenderStylesheetRule {
                source_order: rule_source_order,
                selector: CompactString::from(rule.selector.raw.as_str()),
                conditions: Vec::new(),
                declarations: rule.declarations.clone(),
                span: rule.span,
            });

            for declaration in &rule.declarations {
                let candidate = AppliedDeclaration {
                    declaration: declaration.clone(),
                    key: CascadeKey {
                        important: declaration.important,
                        specificity,
                        source_order: declaration_order,
                    },
                };

                if let Some(existing) = applied
                    .iter_mut()
                    .find(|applied| applied.declaration.property == candidate.declaration.property)
                {
                    if candidate.key.wins_over(existing.key) {
                        *existing = candidate;
                    }
                } else {
                    applied.push(candidate);
                }

                declaration_order += 1;
            }
        }

        ElementStyles {
            declarations: applied
                .into_iter()
                .map(|applied| applied.declaration)
                .collect(),
            matched_rules,
            variants,
        }
    }

    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    #[must_use]
    pub(crate) fn theme_plan(&self) -> RenderThemePlan {
        self.theme.clone()
    }

    fn collect_theme_rule(&mut self, rule: &CssRule) {
        for declaration in &rule.declarations {
            let StyleProperty::Custom(name) = &declaration.property else {
                continue;
            };

            self.theme.push_token_value(
                name.clone(),
                theme_scope_for_rule(rule),
                StyleValue::from_theme_raw(declaration.value.as_str(), declaration.span),
                declaration.span,
            );
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ElementStyles {
    pub(crate) declarations: Vec<StyleDeclaration>,
    pub(crate) matched_rules: Vec<RenderStylesheetRule>,
    pub(crate) variants: Vec<RenderStyleVariant>,
}

fn variant_for_rule(rule: &CssRule, element: &StyleElement<'_, '_>) -> Option<RenderStyleVariant> {
    if !rule.selector.has_static_anchor() {
        return None;
    }

    let mut conditions = rule.conditions.clone();

    if let Some(dynamic_conditions) = rule.selector.matching_dynamic_conditions(element) {
        extend_unique_conditions(&mut conditions, dynamic_conditions);
    } else if rule.conditions.is_empty() || rule.selector.matching_specificity(element).is_none() {
        return None;
    }

    (!conditions.is_empty()).then(|| RenderStyleVariant {
        conditions,
        selector: CompactString::from(rule.selector.raw.as_str()),
        declarations: rule.declarations.clone(),
        span: rule.span,
    })
}

fn extend_unique_conditions(
    conditions: &mut Vec<RenderStyleCondition>,
    extra: Vec<RenderStyleCondition>,
) {
    for condition in extra {
        if !conditions.contains(&condition) {
            conditions.push(condition);
        }
    }
}

fn theme_scope_for_rule(rule: &CssRule) -> RenderThemeScope {
    let selector = CompactString::from(rule.selector.raw.as_str());
    if rule.conditions.is_empty() {
        if is_root_theme_selector(&rule.selector.raw) {
            return RenderThemeScope::Root;
        }

        return RenderThemeScope::Selector(selector);
    }

    RenderThemeScope::Conditional {
        selector: (!is_root_theme_selector(&rule.selector.raw)).then_some(selector),
        conditions: rule.conditions.clone(),
    }
}

fn is_root_theme_selector(selector: &str) -> bool {
    selector.split(',').all(|part| {
        matches!(
            part.trim().to_ascii_lowercase().as_str(),
            ":root" | "html" | ":host"
        )
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AppliedDeclaration {
    declaration: StyleDeclaration,
    key: CascadeKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CascadeKey {
    important: bool,
    specificity: u32,
    source_order: usize,
}

impl CascadeKey {
    fn wins_over(self, other: Self) -> bool {
        (self.important, self.specificity, self.source_order)
            > (other.important, other.specificity, other.source_order)
    }
}
