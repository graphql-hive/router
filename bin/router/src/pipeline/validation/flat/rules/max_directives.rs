//! The router's `MaxDirectivesRule`, flat.

use std::collections::HashMap;

use graphql_tools::flat::{Kind, Sym};
use graphql_tools::validation::utils::ValidationError;

use super::max_depth::{Node, Visited};
use crate::config::limits::MaxDirectivesRuleConfig;
use crate::pipeline::validation::flat::visit::{
    error, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "MAX_DIRECTIVES_EXCEEDED";

pub struct MaxDirectives {
    pub config: MaxDirectivesRuleConfig,
}

impl FlatRule for MaxDirectives {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(Rule {
            n: self.config.n,
            visited: HashMap::new(),
        })
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

struct Rule {
    n: usize,
    visited: HashMap<Sym, Visited>,
}

impl Rule {
    fn check_limit(&self, count: usize) -> Result<usize, ValidationError> {
        if count > self.n {
            Err(error(
                CODE,
                "Directives limit exceeded.".to_string(),
                vec![],
            ))
        } else {
            Ok(count)
        }
    }

    fn count_directives<'a>(
        &mut self,
        ctx: &Ctx<'a>,
        node: Node<'a>,
    ) -> Result<usize, ValidationError> {
        let doc = ctx.doc;
        // every node has directives, so the tree's `Some` always holds
        let mut count = self.check_limit(node.directives().len as usize)?;

        if let Some(list) = node.selection_set() {
            for &id in doc.items(list) {
                let child = self.count_directives(ctx, Node::Selection(doc.selection(id)))?;
                count = self.check_limit(count + child)?;
            }
        }

        if let Node::Selection(s) = node {
            if s.kind == Kind::FragmentSpread {
                match self.visited.get(&s.name) {
                    Some(Visited::Counted(num)) => return self.check_limit(count + num),
                    Some(Visited::Visiting) => return Ok(count),
                    None => {}
                }
                self.visited.insert(s.name, Visited::Visiting);
                if let Some(fragment) = ctx.known_fragments.get(doc.str(s.name)) {
                    let fragment_count = self.count_directives(ctx, Node::Fragment(fragment))?;
                    self.visited
                        .insert(s.name, Visited::Counted(fragment_count));
                    count = self.check_limit(count + fragment_count)?;
                }
            }
        }

        Ok(count)
    }
}

impl<'a> Visitor<'a> for Rule {
    fn enter_document(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors) {
        self.visited = HashMap::with_capacity(ctx.known_fragments.len());
        for op in Node::operations(ctx.doc) {
            if let Err(err) = self.count_directives(ctx, op) {
                errors.report_error(err);
            }
        }
    }
}
