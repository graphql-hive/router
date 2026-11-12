//! `graphql_tools::validation::rules::VariablesInAllowedPosition`, flat.

use std::collections::{HashMap, HashSet};

use graphql_tools::flat::{Fragment, Operation, Selection, Sym, Value, VariableDefinition};
use graphql_tools::static_graphql::query::Type;

use crate::pipeline::validation::flat::visit::{
    error, pos, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "VariablesInAllowedPosition";

pub struct VariablesInAllowedPosition;

impl FlatRule for VariablesInAllowedPosition {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(Rule::default())
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

/// An operation by its name (`NONE` when anonymous), or a fragment.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Scope {
    Operation(Sym),
    Fragment(Sym),
}

/// Name, the type where it's used, whether that spot has a default, the oneOf parent.
type Usage<'a> = (Sym, &'a Type, bool, Option<&'a str>);

/// The tree walks its operations and spreads out of HashMaps/HashSets, so its order is random
/// once there's more than one error. We go in document order instead.
#[derive(Default)]
struct Rule<'a> {
    current: Option<Scope>,
    /// Deduped, like the tree's HashSet.
    spreads: HashMap<Scope, Vec<Sym>>,
    usages: HashMap<Scope, Vec<Usage<'a>>>,
    defs: Vec<(Scope, Vec<&'a VariableDefinition>)>,
}

impl<'a> Rule<'a> {
    fn collect(
        &self,
        ctx: &Ctx<'a>,
        errors: &mut Errors,
        from: Scope,
        defs: &[&'a VariableDefinition],
        visited: &mut HashSet<Scope>,
    ) {
        if !visited.insert(from) {
            return;
        }
        for &(name, location_type, has_default, one_of_parent) in
            self.usages.get(&from).map(Vec::as_slice).unwrap_or(&[])
        {
            let Some(def) = defs.iter().find(|d| d.name == name) else {
                continue;
            };
            let var_type = ctx.ty(def.ty);
            let has_non_null_default = def
                .default_value
                .is_some_and(|v| !matches!(ctx.doc.value(v), Value::Null));
            let variable_type = match var_type {
                Type::NonNullType(_) => var_type.clone(),
                t if has_non_null_default => Type::NonNullType(Box::new(t.clone())),
                t => t.clone(),
            };
            let effective_location_type = match (has_default, location_type) {
                (true, Type::NonNullType(inner)) => inner.as_ref(),
                _ => location_type,
            };
            if !ctx
                .schema
                .is_subtype(&variable_type, effective_location_type)
            {
                errors.report_error(error(
                    CODE,
                    format!(
                        "Variable \"${}\" of type \"{}\" used in position expecting type \"{}\".",
                        ctx.str(name),
                        variable_type,
                        location_type,
                    ),
                    vec![pos(def.pos)],
                ));
            }
            if let Some(one_of_type) = one_of_parent {
                if !matches!(var_type, Type::NonNullType(_)) {
                    errors.report_error(error(
                        CODE,
                        format!(
                            "Variable \"${}\" is of type \"{}\" but must be non-nullable to be used for OneOf Input Object \"{}\".",
                            ctx.str(name),
                            var_type,
                            one_of_type,
                        ),
                        vec![pos(def.pos)],
                    ));
                }
            }
        }
        if let Some(spreads) = self.spreads.get(&from) {
            for &spread in spreads {
                self.collect(ctx, errors, Scope::Fragment(spread), defs, visited);
            }
        }
    }
}

impl<'a> Visitor<'a> for Rule<'a> {
    fn leave_document(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors) {
        for (scope, defs) in &self.defs {
            self.collect(ctx, errors, *scope, defs, &mut HashSet::new());
        }
    }

    fn enter_fragment_definition(&mut self, _: &mut Ctx<'a>, _: &mut Errors, f: &'a Fragment) {
        self.current = Some(Scope::Fragment(f.name));
    }

    fn enter_operation(&mut self, _: &mut Ctx<'a>, _: &mut Errors, op: &'a Operation) {
        self.current = Some(Scope::Operation(op.name));
    }

    fn enter_fragment_spread(&mut self, _: &mut Ctx<'a>, _: &mut Errors, s: &'a Selection) {
        if let Some(scope) = self.current {
            let spreads = self.spreads.entry(scope).or_default();
            if !spreads.contains(&s.name) {
                spreads.push(s.name);
            }
        }
    }

    fn enter_variable_definition(
        &mut self,
        _: &mut Ctx<'a>,
        _: &mut Errors,
        variable: &'a VariableDefinition,
    ) {
        let Some(scope) = self.current else {
            return;
        };
        match self.defs.iter_mut().find(|(s, _)| *s == scope) {
            Some((_, defs)) => defs.push(variable),
            None => self.defs.push((scope, vec![variable])),
        }
    }

    fn enter_variable_value(&mut self, ctx: &mut Ctx<'a>, _: &mut Errors, name: Sym) {
        let (Some(scope), Some(input_type)) = (self.current, ctx.current_input_type_literal())
        else {
            return;
        };
        let has_default = ctx.current_input_type_has_default();
        let one_of_parent = ctx
            .parent_input_type()
            .filter(|parent| parent.is_one_of())
            .map(|parent| parent.name());
        self.usages
            .entry(scope)
            .or_default()
            .push((name, input_type, has_default, one_of_parent));
    }
}
