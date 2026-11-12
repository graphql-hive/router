//! The router's `MaxDepthRule`, flat.

use std::collections::HashMap;

use graphql_tools::flat::{Definition, Document, Fragment, Kind, ListId, Selection, Span, Sym};
use graphql_tools::validation::utils::ValidationError;

use crate::config::limits::MaxDepthRuleConfig;
use crate::pipeline::validation::flat::visit::{
    error, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "MAX_DEPTH_EXCEEDED";

pub struct MaxDepth {
    pub config: MaxDepthRuleConfig,
}

impl FlatRule for MaxDepth {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(Rule {
            config: self.config.clone(),
            visited: HashMap::new(),
        })
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

/// `CountableNode` of the tree, shared by the three limit rules.
#[derive(Clone, Copy)]
pub(super) enum Node<'a> {
    Operation {
        selection_set: ListId,
        directives: Span,
    },
    Fragment(&'a Fragment),
    Selection(&'a Selection),
}

impl<'a> Node<'a> {
    /// `None` for a spread only.
    pub(super) fn selection_set(self) -> Option<ListId> {
        match self {
            Node::Operation { selection_set, .. } => Some(selection_set),
            Node::Fragment(f) => Some(f.selection_set),
            Node::Selection(s) if s.kind == Kind::FragmentSpread => None,
            Node::Selection(s) => Some(s.children),
        }
    }

    pub(super) fn directives(self) -> Span {
        match self {
            Node::Operation { directives, .. } => directives,
            Node::Fragment(f) => f.directives,
            Node::Selection(s) => s.directives,
        }
    }

    /// Every operation, in document order.
    pub(super) fn operations(doc: &'a Document) -> impl Iterator<Item = Node<'a>> {
        doc.definitions.iter().filter_map(|d| match d {
            Definition::Operation(op) => Some(Node::Operation {
                selection_set: op.selection_set,
                directives: op.directives,
            }),
            Definition::Fragment(_) => None,
        })
    }
}

/// `VisitedFragment` of the tree.
pub(super) enum Visited {
    Counted(usize),
    Visiting,
}

struct Rule {
    config: MaxDepthRuleConfig,
    visited: HashMap<Sym, Visited>,
}

impl Rule {
    fn check_limit(&self, count: usize) -> Result<usize, ValidationError> {
        if count > self.config.n {
            Err(error(
                CODE,
                "Query depth limit exceeded.".to_string(),
                vec![],
            ))
        } else {
            Ok(count)
        }
    }

    fn count_depth<'a>(
        &mut self,
        ctx: &Ctx<'a>,
        node: Node<'a>,
        parent_depth: usize,
    ) -> Result<usize, ValidationError> {
        let doc = ctx.doc;
        if self.config.ignore_introspection {
            if let Node::Selection(s) = node {
                if s.kind == Kind::Field && matches!(doc.str(s.name), "__schema" | "__type") {
                    return Ok(0);
                }
            }
        }

        let mut parent_depth = parent_depth;
        let mut depth = self.check_limit(parent_depth)?;

        if let Some(list) = node.selection_set() {
            for &id in doc.items(list) {
                let child = doc.selection(id);
                let increase_by = if self.config.flatten_fragments && child.kind != Kind::Field {
                    0
                } else {
                    1
                };
                depth = depth.max(self.count_depth(
                    ctx,
                    Node::Selection(child),
                    parent_depth + increase_by,
                )?);
            }
        }

        if let Node::Selection(s) = node {
            if s.kind == Kind::FragmentSpread {
                if !self.config.flatten_fragments {
                    parent_depth += 1;
                }
                match self.visited.get(&s.name) {
                    Some(Visited::Counted(d)) => return self.check_limit(parent_depth + d),
                    Some(Visited::Visiting) => return Ok(depth),
                    None => {}
                }
                self.visited.insert(s.name, Visited::Visiting);
                if let Some(fragment) = ctx.known_fragments.get(doc.str(s.name)) {
                    let fragment_depth = self.count_depth(ctx, Node::Fragment(fragment), 0)?;
                    self.visited
                        .insert(s.name, Visited::Counted(fragment_depth));
                    depth = depth.max(self.check_limit(parent_depth + fragment_depth)?);
                }
            }
        }

        Ok(depth)
    }
}

impl<'a> Visitor<'a> for Rule {
    fn enter_document(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors) {
        self.visited = HashMap::with_capacity(ctx.known_fragments.len());
        for op in Node::operations(ctx.doc) {
            if let Err(err) = self.count_depth(ctx, op, 0) {
                errors.report_error(err);
            }
        }
    }
}
