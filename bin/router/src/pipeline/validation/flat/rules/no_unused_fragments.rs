//! `graphql_tools::validation::rules::NoUnusedFragments`, flat.

use std::collections::{HashMap, HashSet};

use graphql_tools::flat::{Fragment, Selection, Sym};

use crate::pipeline::validation::flat::visit::{
    error, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "NoUnusedFragments";

pub struct NoUnusedFragments;

impl FlatRule for NoUnusedFragments {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(Rule::default())
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

#[derive(Default)]
struct Rule {
    /// Spread from operations.
    in_use: Vec<Sym>,
    current: Option<(Sym, Vec<Sym>)>,
    /// What each fragment spreads; a later fragment with the same name wins, like the tree's.
    spreads: HashMap<Sym, Vec<Sym>>,
}

impl<'a> Visitor<'a> for Rule {
    fn enter_fragment_definition(&mut self, _: &mut Ctx<'a>, _: &mut Errors, f: &'a Fragment) {
        self.current = Some((f.name, vec![]));
    }

    fn leave_fragment_definition(&mut self, _: &mut Ctx<'a>, _: &mut Errors, _: &'a Fragment) {
        if let Some((name, spreads)) = self.current.take() {
            self.spreads.insert(name, spreads);
        }
    }

    fn enter_fragment_spread(&mut self, _: &mut Ctx<'a>, _: &mut Errors, spread: &'a Selection) {
        match &mut self.current {
            Some((_, spreads)) => spreads.push(spread.name),
            None => self.in_use.push(spread.name),
        }
    }

    fn leave_document(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors) {
        let mut reachable = HashSet::new();
        let mut queue = std::mem::take(&mut self.in_use);
        while let Some(name) = queue.pop() {
            if reachable.insert(name) {
                if let Some(spreads) = self.spreads.get(&name) {
                    queue.extend(spreads.iter().filter(|s| !reachable.contains(*s)));
                }
            }
        }
        // the tree goes over a HashMap, so its order is random; ours is the document's,
        // each name once
        let mut reported = HashSet::new();
        for f in ctx.doc.fragments() {
            if !reachable.contains(&f.name) && reported.insert(f.name) {
                errors.report_error(error(
                    CODE,
                    format!("Fragment \"{}\" is never used.", ctx.str(f.name)),
                    vec![],
                ));
            }
        }
    }
}
