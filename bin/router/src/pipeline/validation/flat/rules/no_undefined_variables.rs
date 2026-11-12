//! `graphql_tools::validation::rules::NoUndefinedVariables`, flat.

use std::collections::{HashMap, HashSet};

use graphql_tools::flat::{
    Argument, Document, Fragment, Operation, Selection, Sym, Value, ValueId, VariableDefinition,
};

use crate::pipeline::validation::flat::visit::{
    error, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "NoUndefinedVariables";

pub struct NoUndefinedVariables;

impl FlatRule for NoUndefinedVariables {
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

/// The tree keeps all of this in HashMaps/HashSets and walks them to report, so its order is
/// random once there's more than one error. We go in document order instead: operations as
/// they come, variables as the walk from the operation first runs into them.
#[derive(Default)]
struct Rule {
    current: Option<Scope>,
    /// Per operation name, its variables. A second operation with the same name starts over,
    /// like the tree's `insert`.
    defined: Vec<(Sym, Vec<Sym>)>,
    used: HashMap<Scope, Vec<Sym>>,
    spreads: HashMap<Scope, Vec<Sym>>,
}

fn variables_in(doc: &Document, id: ValueId, out: &mut Vec<Sym>) {
    match *doc.value(id) {
        Value::Variable(name) => out.push(name),
        Value::List(items) => {
            for &item in &doc.value_items[items.range()] {
                variables_in(doc, item, out);
            }
        }
        Value::Object(fields) => {
            for &(_, value) in &doc.object_fields[fields.range()] {
                variables_in(doc, value, out);
            }
        }
        _ => {}
    }
}

impl Rule {
    fn find_undefined(
        &self,
        from: Scope,
        defined: &[Sym],
        undefined: &mut Vec<Sym>,
        visited: &mut HashSet<Scope>,
    ) {
        if !visited.insert(from) {
            return;
        }
        if let Some(vars) = self.used.get(&from) {
            for var in vars {
                if !defined.contains(var) && !undefined.contains(var) {
                    undefined.push(*var);
                }
            }
        }
        if let Some(spreads) = self.spreads.get(&from) {
            for &spread in spreads {
                self.find_undefined(Scope::Fragment(spread), defined, undefined, visited);
            }
        }
    }
}

impl<'a> Visitor<'a> for Rule {
    fn enter_operation(&mut self, _: &mut Ctx<'a>, _: &mut Errors, op: &'a Operation) {
        self.current = Some(Scope::Operation(op.name));
        match self.defined.iter_mut().find(|(name, _)| *name == op.name) {
            Some((_, vars)) => vars.clear(),
            None => self.defined.push((op.name, vec![])),
        }
    }

    fn enter_fragment_definition(&mut self, _: &mut Ctx<'a>, _: &mut Errors, f: &'a Fragment) {
        self.current = Some(Scope::Fragment(f.name));
    }

    fn enter_fragment_spread(&mut self, _: &mut Ctx<'a>, _: &mut Errors, s: &'a Selection) {
        if let Some(scope) = self.current {
            self.spreads.entry(scope).or_default().push(s.name);
        }
    }

    fn enter_variable_definition(
        &mut self,
        _: &mut Ctx<'a>,
        _: &mut Errors,
        variable: &'a VariableDefinition,
    ) {
        if let Some(Scope::Operation(op)) = self.current {
            if let Some((_, vars)) = self.defined.iter_mut().find(|(name, _)| *name == op) {
                if !vars.contains(&variable.name) {
                    vars.push(variable.name);
                }
            }
        }
    }

    fn enter_argument(&mut self, ctx: &mut Ctx<'a>, _: &mut Errors, arg: &'a Argument) {
        if let Some(scope) = self.current {
            variables_in(ctx.doc, arg.value, self.used.entry(scope).or_default());
        }
    }

    fn leave_document(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors) {
        for (op, defined) in &self.defined {
            let mut undefined = Vec::new();
            self.find_undefined(
                Scope::Operation(*op),
                defined,
                &mut undefined,
                &mut HashSet::new(),
            );
            for var in undefined {
                let message = match ctx.doc.opt_str(*op) {
                    Some(op) => format!(
                        "Variable \"${}\" is not defined by operation \"{}\".",
                        ctx.str(var),
                        op
                    ),
                    None => format!("Variable \"${}\" is not defined.", ctx.str(var)),
                };
                errors.report_error(error(CODE, message, vec![]));
            }
        }
    }
}
