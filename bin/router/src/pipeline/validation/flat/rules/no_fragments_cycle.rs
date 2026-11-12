//! `graphql_tools::validation::rules::NoFragmentsCycle`, flat.

use std::collections::{HashMap, HashSet};

use graphql_tools::flat::{Fragment, Kind, Selection, Sym};

use crate::pipeline::validation::flat::visit::{
    error, pos, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "NoFragmentsCycle";

pub struct NoFragmentsCycle;

impl FlatRule for NoFragmentsCycle {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(Rule::default())
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

/// Lives for the whole document, like the tree's: a fragment is explored once.
#[derive(Default)]
struct Rule {
    visited: HashSet<Sym>,
}

/// The tree's `get_recursive_fragment_spreads`, which isn't recursive: the spreads in the set,
/// and the ones right inside its fields and inline fragments. Nothing deeper.
fn spreads<'a>(ctx: &Ctx<'a>, f: &'a Fragment) -> Vec<&'a Selection> {
    let doc = ctx.doc;
    let mut out = vec![];
    for &id in doc.items(f.selection_set) {
        let s = doc.selection(id);
        if s.kind == Kind::FragmentSpread {
            out.push(s);
        } else {
            out.extend(
                doc.items(s.children)
                    .iter()
                    .map(|&id| doc.selection(id))
                    .filter(|c| c.kind == Kind::FragmentSpread),
            );
        }
    }
    out
}

enum Frame<'a> {
    EnterFragment(&'a Fragment),
    ExitFragment(Sym),
    EnterSpread(&'a Selection),
    PopSpread,
}

impl<'a> Visitor<'a> for Rule {
    fn enter_fragment_definition(
        &mut self,
        ctx: &mut Ctx<'a>,
        errors: &mut Errors,
        root: &'a Fragment,
    ) {
        let mut path: Vec<&Selection> = vec![];
        let mut index_by_name: HashMap<Sym, usize> = HashMap::new();
        let mut stack = vec![Frame::EnterFragment(root)];

        while let Some(frame) = stack.pop() {
            match frame {
                Frame::ExitFragment(name) => {
                    index_by_name.remove(&name);
                }
                Frame::PopSpread => {
                    path.pop();
                }
                Frame::EnterFragment(f) => {
                    if !self.visited.insert(f.name) {
                        continue;
                    }
                    let spread_nodes = spreads(ctx, f);
                    if spread_nodes.is_empty() {
                        continue;
                    }
                    index_by_name.insert(f.name, path.len());
                    stack.push(Frame::ExitFragment(f.name));
                    stack.extend(spread_nodes.into_iter().rev().map(Frame::EnterSpread));
                }
                Frame::EnterSpread(spread) => match index_by_name.get(&spread.name) {
                    None => {
                        if let Some(def) = ctx.known_fragments.get(ctx.str(spread.name)) {
                            path.push(spread);
                            stack.push(Frame::PopSpread);
                            stack.push(Frame::EnterFragment(def));
                        }
                    }
                    Some(&at) => {
                        path.push(spread);
                        let cycle = &path[at..];
                        let via: Vec<String> = cycle[..cycle.len() - 1]
                            .iter()
                            .map(|s| format!("\"{}\"", ctx.str(s.name)))
                            .collect();
                        let name = ctx.str(spread.name);
                        let message = if via.is_empty() {
                            format!("Cannot spread fragment \"{}\" within itself.", name)
                        } else {
                            format!(
                                "Cannot spread fragment \"{}\" within itself via {}.",
                                name,
                                via.join(", ")
                            )
                        };
                        errors.report_error(error(
                            CODE,
                            message,
                            cycle.iter().map(|s| pos(s.pos)).collect(),
                        ));
                        path.pop();
                    }
                },
            }
        }
    }
}
