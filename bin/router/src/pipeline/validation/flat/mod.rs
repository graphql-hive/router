//! Validation on the flat document. The rules are the `graphql_tools` ones, rewritten for the
//! flat AST; they report the same errors, in the same order, at the same places.
//!
//! The plan the router (and plugins) work with is still `graphql_tools`' `ValidationPlan`. Each
//! of its rules runs here when a flat rule with the same error code exists. Any other one (a
//! plugin's own) runs on the tree, built only then, and its errors come after the flat ones.

pub mod rules;
#[cfg(test)]
mod tests;
pub mod visit;

use std::sync::Arc;

use graphql_tools::flat::Document;
use graphql_tools::static_graphql::schema;
use graphql_tools::validation::rules::ValidationOptions;
use graphql_tools::validation::utils::ValidationError;
use graphql_tools::validation::validate::{validate as validate_tree, ValidationPlan};

use crate::config::limits::LimitsConfig;
use crate::pipeline::parser::ParsedOperation;
use visit::{schema_types, Ctx, FlatRule, Walk};

/// The flat rules the router knows, looked up by error code.
pub struct FlatRules {
    rules: Vec<Box<dyn FlatRule>>,
}

impl FlatRules {
    /// The default rules and the router's limits, configured like the tree ones in `lib.rs`.
    pub fn new(options: ValidationOptions, limits: &LimitsConfig) -> Self {
        let mut rules = rules::defaults(options);
        if let Some(config) = &limits.max_depth {
            rules.push(Box::new(rules::max_depth::MaxDepth {
                config: config.clone(),
            }));
        }
        if let Some(config) = &limits.max_directives {
            rules.push(Box::new(rules::max_directives::MaxDirectives {
                config: config.clone(),
            }));
        }
        if let Some(config) = &limits.max_aliases {
            rules.push(Box::new(rules::max_aliases::MaxAliases {
                config: config.clone(),
            }));
        }
        Self { rules }
    }

    pub fn from_rules(rules: Vec<Box<dyn FlatRule>>) -> Self {
        Self { rules }
    }

    fn get(&self, error_code: &str) -> Option<&dyn FlatRule> {
        self.rules
            .iter()
            .find(|r| r.error_code() == error_code)
            .map(|r| r.as_ref())
    }

    /// Runs `plan` on the operation: its rules with a flat twin on the flat document, the rest
    /// on the tree.
    pub fn validate(
        &self,
        schema: &schema::Document,
        operation: &ParsedOperation,
        plan: &ValidationPlan,
    ) -> Vec<ValidationError> {
        let mut flat = Vec::with_capacity(plan.rules.len());
        let mut on_tree = Vec::new();
        for rule in &plan.rules {
            match self.get(rule.error_code()) {
                Some(flat_rule) => flat.push(flat_rule),
                None => on_tree.push(Arc::clone(rule)),
            }
        }

        let mut errors = validate_flat(schema, &operation.flat, &flat);
        if !on_tree.is_empty() {
            let plan = ValidationPlan {
                rules: on_tree,
                hash: 0,
            };
            errors.extend(validate_tree(schema, &operation.tree(), &plan));
        }
        errors
    }
}

/// Runs `rules` on `doc`.
pub fn validate_flat(
    schema: &schema::Document,
    doc: &Document,
    rules: &[&dyn FlatRule],
) -> Vec<ValidationError> {
    if rules.is_empty() {
        return vec![];
    }
    let types = schema_types(doc);
    let mut walk = Walk {
        visitors: rules.iter().map(|r| r.visitor()).collect(),
        ctx: Ctx::new(doc, &types, schema),
        errors: Default::default(),
    };
    walk.document();
    walk.errors.errors
}
