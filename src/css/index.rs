use std::collections::HashMap;

use compact_str::CompactString;

use crate::plan::{
    RenderMotionPlan, RenderStyleCondition, RenderStyleVariant, RenderStylesheetRule,
    RenderThemePlan, RenderThemeScope,
};
use crate::style::{StyleDeclaration, StyleProperty, StyleValue};

use super::{CssRule, StyleElement, Stylesheet};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct StyleIndex {
    rules: Vec<CssRule>,
    theme: RenderThemePlan,
    motion: RenderMotionPlan,
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
        self.motion.extend(stylesheet.motion);
    }

    #[must_use]
    pub(crate) fn styles_for_element(&self, element: &StyleElement<'_, '_>) -> ElementStyles {
        let mut applied = Vec::<AppliedDeclaration>::new();
        let mut applied_variants = Vec::<AppliedVariant>::new();
        let mut conditional_winners = HashMap::<ConditionalProperty, ConditionalWinner>::new();
        let mut declaration_order = 0usize;
        let mut matched_rules = Vec::new();

        for (rule_source_order, rule) in self.rules.iter().enumerate() {
            let rule_declaration_order = declaration_order;
            declaration_order += rule.declarations.len();
            for matched in conditional_matches_for_rule(rule, element) {
                apply_conditional_match(
                    matched,
                    rule_declaration_order,
                    &mut applied_variants,
                    &mut conditional_winners,
                );
            }

            if !rule.conditions.is_empty() {
                continue;
            }

            let Some(specificity) = rule.selector.matching_specificity(element) else {
                continue;
            };

            matched_rules.push(RenderStylesheetRule {
                source_order: rule_source_order,
                selector: CompactString::from(rule.selector.raw.as_str()),
                conditions: Vec::new(),
                declarations: rule.declarations.clone(),
                span: rule.span,
            });

            for (declaration_index, declaration) in rule.declarations.iter().enumerate() {
                let candidate = AppliedDeclaration {
                    declaration: declaration.clone(),
                    key: CascadeKey {
                        important: declaration.important,
                        specificity,
                        source_order: rule_declaration_order + declaration_index,
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
            }
        }

        let variants = applied_variants
            .into_iter()
            .filter_map(AppliedVariant::finish)
            .collect();

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
        self.rules.is_empty() && self.motion.is_empty()
    }

    #[must_use]
    pub(crate) fn motion_plan(&self) -> RenderMotionPlan {
        self.motion.clone()
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

fn conditional_matches_for_rule(
    rule: &CssRule,
    element: &StyleElement<'_, '_>,
) -> Vec<ConditionalMatch> {
    if !rule.selector.has_static_anchor() {
        return Vec::new();
    }

    let mut matches = Vec::new();
    for (specificity, dynamic_conditions) in rule.selector.matching_dynamic_branches(element) {
        let mut conditions = rule.conditions.clone();
        extend_unique_conditions(&mut conditions, dynamic_conditions);
        push_conditional_match(
            &mut matches,
            conditional_match(rule, conditions, specificity),
        );
    }
    if !rule.conditions.is_empty()
        && let Some(specificity) = rule.selector.matching_specificity(element)
    {
        push_conditional_match(
            &mut matches,
            conditional_match(rule, rule.conditions.clone(), specificity),
        );
    }
    matches
}

fn conditional_match(
    rule: &CssRule,
    conditions: Vec<RenderStyleCondition>,
    specificity: u32,
) -> ConditionalMatch {
    ConditionalMatch {
        specificity,
        variant: RenderStyleVariant {
            conditions,
            selector: CompactString::from(rule.selector.raw.as_str()),
            declarations: rule.declarations.clone(),
            span: rule.span,
        },
    }
}

fn push_conditional_match(matches: &mut Vec<ConditionalMatch>, candidate: ConditionalMatch) {
    if let Some(existing) = matches.iter_mut().find(|existing| {
        condition_key(&existing.variant.conditions) == condition_key(&candidate.variant.conditions)
    }) {
        if candidate.specificity > existing.specificity {
            *existing = candidate;
        }
    } else {
        matches.push(candidate);
    }
}

fn apply_conditional_match(
    matched: ConditionalMatch,
    source_order: usize,
    variants: &mut Vec<AppliedVariant>,
    winners: &mut HashMap<ConditionalProperty, ConditionalWinner>,
) {
    let variant_index = variants.len();
    let condition = condition_key(&matched.variant.conditions);
    let mut declarations = Vec::with_capacity(matched.variant.declarations.len());
    for (declaration_index, declaration) in matched.variant.declarations.iter().enumerate() {
        let key = CascadeKey {
            important: declaration.important,
            specificity: matched.specificity,
            source_order: source_order + declaration_index,
        };
        let property = ConditionalProperty {
            condition: condition.clone(),
            property: declaration.property.clone(),
        };
        let winner = ConditionalWinner {
            variant_index,
            declaration_index: declarations.len(),
            key,
        };
        let should_apply = winners
            .get(&property)
            .is_none_or(|existing| key.wins_over(existing.key));
        if should_apply {
            if let Some(existing) = winners.insert(property, winner) {
                variants[existing.variant_index].declarations[existing.declaration_index] = None;
            }
            declarations.push(Some(declaration.clone()));
        } else {
            declarations.push(None);
        }
    }
    variants.push(AppliedVariant {
        variant: matched.variant,
        declarations,
    });
}

fn condition_key(conditions: &[RenderStyleCondition]) -> Vec<ConditionPart> {
    let mut key = conditions
        .iter()
        .map(ConditionPart::from)
        .collect::<Vec<_>>();
    key.sort_unstable();
    key.dedup();
    key
}

#[derive(Clone, Debug)]
struct ConditionalMatch {
    variant: RenderStyleVariant,
    specificity: u32,
}

#[derive(Clone, Debug)]
struct AppliedVariant {
    variant: RenderStyleVariant,
    declarations: Vec<Option<StyleDeclaration>>,
}

impl AppliedVariant {
    fn finish(mut self) -> Option<RenderStyleVariant> {
        self.variant.declarations = self.declarations.into_iter().flatten().collect();
        (!self.variant.declarations.is_empty()).then_some(self.variant)
    }
}

#[derive(Clone, Copy, Debug)]
struct ConditionalWinner {
    variant_index: usize,
    declaration_index: usize,
    key: CascadeKey,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ConditionalProperty {
    condition: Vec<ConditionPart>,
    property: StyleProperty,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum ConditionPart {
    PseudoClass(CompactString),
    PseudoElement(CompactString),
    Media(CompactString),
    Supports(CompactString),
    Container(CompactString),
    StartingStyle,
    ActiveViewTransitionType(Vec<CompactString>),
}

impl From<&RenderStyleCondition> for ConditionPart {
    fn from(condition: &RenderStyleCondition) -> Self {
        match condition {
            RenderStyleCondition::PseudoClass(value) => Self::PseudoClass(value.clone()),
            RenderStyleCondition::PseudoElement(value) => Self::PseudoElement(value.clone()),
            RenderStyleCondition::Media(value) => Self::Media(value.clone()),
            RenderStyleCondition::Supports(value) => Self::Supports(value.clone()),
            RenderStyleCondition::Container(value) => Self::Container(value.clone()),
            RenderStyleCondition::StartingStyle => Self::StartingStyle,
            RenderStyleCondition::ActiveViewTransitionType(types) => {
                Self::ActiveViewTransitionType(types.clone())
            }
        }
    }
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
