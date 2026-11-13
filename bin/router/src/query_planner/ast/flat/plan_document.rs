//! What a fetch sends to a subgraph, flat: one operation and its fragments, their selections
//! in an `Arena` of their own. Prints the same text the tree `Document` prints.

use std::fmt::{self, Display, Write};

use super::arena::{Arena, ListId, NodeId};
use super::Kind;
use crate::query_planner::ast::document::Document;
use crate::query_planner::ast::fragment_expansion::FragmentExpansionError;
use crate::query_planner::ast::operation::{OperationDefinition, VariableDefinition};
use crate::query_planner::state::supergraph_state::OperationKind;
use crate::query_planner::utils::pretty_display::{get_indent, PrettyDisplay};

#[derive(Debug, Clone)]
pub struct PlanFragment {
    pub name: String,
    pub type_condition: String,
    pub selections: ListId,
}

#[derive(Debug, Clone)]
pub struct PlanDocument {
    pub arena: Arena,
    /// `None` prints like a query, see `OperationDefinition`.
    pub kind: Option<OperationKind>,
    pub name: Option<String>,
    pub variables: Option<Vec<VariableDefinition>>,
    pub selections: ListId,
    pub fragments: Vec<PlanFragment>,
}

impl Default for PlanDocument {
    fn default() -> Self {
        let mut arena = Arena::new();
        let selections = arena.new_list();
        PlanDocument {
            arena,
            kind: None,
            name: None,
            variables: None,
            selections,
            fragments: vec![],
        }
    }
}

impl PlanDocument {
    /// From the tree.
    // ponytail: a bridge while plan nodes move over, goes with the tree
    pub fn from_tree(document: &Document) -> Self {
        let mut arena = Arena::new();
        let selections = arena.import(&document.operation.selection_set);
        let fragments = document
            .fragments
            .iter()
            .map(|f| PlanFragment {
                name: f.name.clone(),
                type_condition: f.type_condition.clone(),
                selections: arena.import(&f.selection_set),
            })
            .collect();
        PlanDocument {
            arena,
            kind: document.operation.operation_kind.clone(),
            name: document.operation.name.clone(),
            variables: document.operation.variable_definitions.clone(),
            selections,
            fragments,
        }
    }

    /// The operation as a tree. Only plugin hooks (and tools, tests) want one, the request path
    /// reads the flat document. Fragments aren't part of it, a normalized operation has none.
    pub fn operation_tree(&self) -> OperationDefinition {
        OperationDefinition {
            name: self.name.clone(),
            operation_kind: self.kind.clone(),
            variable_definitions: self.variables.clone(),
            selection_set: self.arena.export(self.selections),
        }
    }

    /// Splits off the introspection fields, which the router answers itself, like
    /// the old tree partition did: the rest for the subgraphs, and the introspection part if any.
    /// `None` for the introspection part means there's nothing to split, the document stays as
    /// it is.
    pub fn partition_introspection(&self) -> Option<(PlanDocument, PlanDocument)> {
        // (downstream, introspection) lists
        fn split(arena: &mut Arena, list: ListId) -> (Vec<NodeId>, Vec<NodeId>) {
            let mut down = vec![];
            let mut intro = vec![];
            for index in 0..arena.items(list).len() {
                let id = arena.items(list)[index];
                let node = *arena.node(id);
                match node.kind {
                    Kind::Field => {
                        let name = arena.str(node.name);
                        if name.starts_with("__") && name != "__typename" {
                            intro.push(id);
                            continue;
                        }
                        let is_leaf = arena.is_empty(node.children);
                        let (d, i) = split(arena, node.children);
                        if is_leaf || !d.is_empty() {
                            down.push(shell(arena, id, &d));
                        }
                        if !i.is_empty() {
                            intro.push(shell(arena, id, &i));
                        }
                    }
                    Kind::InlineFragment => {
                        let (d, i) = split(arena, node.children);
                        if !d.is_empty() {
                            down.push(shell(arena, id, &d));
                        }
                        if !i.is_empty() {
                            intro.push(shell(arena, id, &i));
                        }
                    }
                    Kind::FragmentSpread => down.push(id),
                }
            }
            (down, intro)
        }
        // the node with these children
        fn shell(arena: &mut Arena, id: NodeId, children: &[NodeId]) -> NodeId {
            let mut node = *arena.node(id);
            node.children = arena.list_of(children);
            arena.add_node(node)
        }

        let mut arena = self.arena.clone();
        let (down, intro) = split(&mut arena, self.selections);
        if intro.is_empty() {
            return None;
        }
        let down = arena.list_of(&down);
        let intro = arena.list_of(&intro);
        let mut used = std::collections::BTreeSet::new();
        arena.variable_usages(intro, &mut used);
        let intro_variables = self.variables.as_ref().map(|defs| {
            defs.iter()
                .filter(|def| used.contains(&def.name))
                .cloned()
                .collect()
        });
        let downstream = PlanDocument {
            arena: arena.clone(),
            kind: self.kind.clone(),
            name: self.name.clone(),
            variables: self.variables.clone(),
            selections: down,
            fragments: vec![],
        };
        let introspection = PlanDocument {
            arena,
            kind: self.kind.clone(),
            name: self.name.clone(),
            variables: intro_variables,
            selections: intro,
            fragments: vec![],
        };
        Some((downstream, introspection))
    }

    pub fn fragment(&self, name: &str) -> Option<&PlanFragment> {
        self.fragments.iter().find(|f| f.name == name)
    }

    /// `inline_fragment_spreads`: copies `list` of `source` into `target`, every `...Name` swapped
    /// for a `... on Type` holding that fragment's selections. Fragments come from this document.
    pub fn copy_inlined(
        &self,
        source: &Arena,
        list: ListId,
        target: &mut Arena,
    ) -> Result<ListId, FragmentExpansionError> {
        self.copy_inlined_inner(source, list, target, &mut Vec::new())
    }

    fn copy_inlined_inner(
        &self,
        source: &Arena,
        list: ListId,
        target: &mut Arena,
        visiting: &mut Vec<String>,
    ) -> Result<ListId, FragmentExpansionError> {
        let mut nodes = Vec::with_capacity(source.items(list).len());
        for &id in source.items(list) {
            let node = source.node(id);
            let copy = if node.kind == Kind::FragmentSpread {
                let name = source.str(node.name);
                if visiting.iter().any(|v| v == name) {
                    return Err(FragmentExpansionError::Cycle(name.to_string()));
                }
                let fragment = self
                    .fragment(name)
                    .ok_or_else(|| FragmentExpansionError::MissingFragment(name.to_string()))?;
                visiting.push(name.to_string());
                let children =
                    self.copy_inlined_inner(&self.arena, fragment.selections, target, visiting)?;
                visiting.pop();
                let wrapper = target.new_fragment(&fragment.type_condition, None);
                target.node_mut(wrapper).children = children;
                wrapper
            } else {
                let children = self.copy_inlined_inner(source, node.children, target, visiting)?;
                target.copy_shell_from(source, id, children)
            };
            nodes.push(copy);
        }
        let copy = target.new_list();
        target.set_items(copy, &nodes);
        Ok(copy)
    }

    /// Moves a plain `__typename` to the front of every selection set, like the tree's
    /// `typename_first`.
    pub fn typename_first(&mut self) {
        let mut lists = vec![self.selections];
        lists.extend(self.fragments.iter().map(|f| f.selections));
        while let Some(list) = lists.pop() {
            let arena = &mut self.arena;
            let typename = arena.items(list).iter().position(|&id| {
                let node = arena.node(id);
                node.kind == Kind::Field
                    && arena.str(node.name) == "__typename"
                    && arena.opt_str(node.alias).is_none()
            });
            if let Some(index) = typename {
                arena.items_mut(list)[..=index].rotate_right(1);
            }
            lists.extend(arena.items(list).iter().map(|&id| arena.node(id).children));
        }
    }
}

/// The selection set the way the tree's `SelectionSet` prints it.
pub struct DisplayList<'a>(pub &'a Arena, pub ListId);

impl Display for DisplayList<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_list(self.0, self.1, f)
    }
}

fn write_list(arena: &Arena, list: ListId, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let items = arena.items(list);
    if items.is_empty() {
        return Ok(());
    }
    f.write_char('{')?;
    for (i, &id) in items.iter().enumerate() {
        write_node(arena, id, f)?;
        if i + 1 != items.len() {
            f.write_char(' ')?;
        }
    }
    f.write_char('}')
}

fn write_node(arena: &Arena, id: NodeId, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let node = arena.node(id);
    let conditions = |f: &mut fmt::Formatter<'_>| -> fmt::Result {
        if let Some(skip_if) = arena.opt_str(node.skip_if) {
            write!(f, " @skip(if: ${skip_if})")?;
        }
        if let Some(include_if) = arena.opt_str(node.include_if) {
            write!(f, " @include(if: ${include_if})")?;
        }
        Ok(())
    };
    match node.kind {
        Kind::Field => {
            if let Some(alias) = arena.opt_str(node.alias) {
                write!(f, "{alias}: ")?;
            }
            f.write_str(arena.str(node.name))?;
            if let Some(args) = arena.args(node.args).filter(|a| !a.is_empty()) {
                write!(f, "({args})")?;
            }
            conditions(f)?;
            write_list(arena, node.children, f)
        }
        Kind::InlineFragment => {
            write!(f, "...on {}", arena.str(node.name))?;
            conditions(f)?;
            write_list(arena, node.children, f)
        }
        Kind::FragmentSpread => write!(f, "...{}", arena.str(node.name)),
    }
}

/// The selection set the way the tree's `SelectionSet` pretty-prints it, one item a line.
pub struct PrettyList<'a>(pub &'a Arena, pub ListId);

impl PrettyDisplay for PrettyList<'_> {
    fn pretty_fmt(&self, f: &mut fmt::Formatter<'_>, depth: usize) -> fmt::Result {
        let PrettyList(arena, list) = *self;
        let indent = get_indent(depth);
        for &id in arena.items(list) {
            let node = arena.node(id);
            let children = PrettyList(arena, node.children);
            match node.kind {
                Kind::Field => {
                    f.write_str(&indent)?;
                    if let Some(alias) = arena.opt_str(node.alias) {
                        write!(f, "{alias}: ")?;
                    }
                    f.write_str(arena.str(node.name))?;
                    if let Some(args) = arena.args(node.args).filter(|a| !a.is_empty()) {
                        write!(f, "({args})")?;
                    }
                    if let Some(skip_if) = arena.opt_str(node.skip_if) {
                        write!(f, " @skip(if: ${skip_if})")?;
                    }
                    if let Some(include_if) = arena.opt_str(node.include_if) {
                        write!(f, " @include(if: ${include_if})")?;
                    }
                    if arena.is_empty(node.children) {
                        writeln!(f)?;
                    } else {
                        writeln!(f, " {{")?;
                        children.pretty_fmt(f, depth + 1)?;
                        writeln!(f, "{indent}}}")?;
                    }
                }
                Kind::InlineFragment => {
                    write!(f, "{indent}... on {} ", arena.str(node.name))?;
                    if let Some(skip_if) = arena.opt_str(node.skip_if) {
                        write!(f, "@skip(if: ${skip_if}) ")?;
                    }
                    if let Some(include_if) = arena.opt_str(node.include_if) {
                        write!(f, "@include(if: ${include_if}) ")?;
                    }
                    writeln!(f, "{{")?;
                    children.pretty_fmt(f, depth + 1)?;
                    writeln!(f, "{indent}}}")?;
                }
                Kind::FragmentSpread => writeln!(f, "{indent}...{}", arena.str(node.name))?,
            }
        }
        Ok(())
    }
}

/// The tree's `Document` print: the operation, then each fragment on its own line.
impl Display for PlanDocument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let selections = DisplayList(&self.arena, self.selections);
        let short = self
            .kind
            .as_ref()
            .is_none_or(|k| matches!(k, OperationKind::Query))
            && self.name.as_deref().is_none_or(str::is_empty)
            && self.variables.as_ref().is_none_or(Vec::is_empty);
        if short {
            write!(f, "{selections}")?;
        } else {
            if let Some(kind) = &self.kind {
                write!(f, "{kind}")?;
            }
            if let Some(name) = &self.name {
                write!(f, " {name} ")?;
            }
            if let Some(variables) = self.variables.as_ref().filter(|v| !v.is_empty()) {
                f.write_char('(')?;
                for (i, variable) in variables.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{variable}")?;
                }
                f.write_char(')')?;
            }
            write!(f, "{selections}")?;
        }
        if !self.fragments.is_empty() {
            f.write_str("\n\n")?;
            for fragment in &self.fragments {
                writeln!(
                    f,
                    "fragment {} on {} {}",
                    fragment.name,
                    fragment.type_condition,
                    DisplayList(&self.arena, fragment.selections)
                )?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::query_planner::ast::normalization::normalize_operation;
    use crate::query_planner::state::supergraph_state::SupergraphState;
    use crate::query_planner::utils::parsing::parse_schema;
    use graphql_tools::parser::parse_query;

    fn split(query: &str) -> Option<(String, String)> {
        let schema = parse_schema(
            r#"
            type User { id: ID! name: String }
            type Query { me: User users(first: Int): [User] }
            "#,
        );
        let supergraph = SupergraphState::new(&schema);
        let document = parse_query(query).expect("parses").into_static();
        let normalized = normalize_operation(&supergraph, &document, None).expect("normalizes");
        normalized
            .document
            .partition_introspection()
            .map(|(down, intro)| (down.to_string(), intro.to_string()))
    }

    #[test]
    fn partition_introspection_splits_the_router_part_off() {
        assert_eq!(
            split("{ me { id __typename } users(first: 1) { name } }"),
            None
        );

        let (down, intro) = split(
            r#"query Q($n: Int, $t: String!) {
              __type(name: $t) { name }
              me { id __typename }
              users(first: $n) { name }
              __schema { queryType { name } }
            }"#,
        )
        .expect("has introspection");
        // the subgraph part keeps every variable, the router part only the ones it uses
        assert_eq!(
            down,
            "query Q ($n:Int, $t:String!){me{id __typename} users(first: $n){name}}"
        );
        assert_eq!(
            intro,
            "query Q ($t:String!){__type(name: $t){name} __schema{queryType{name}}}"
        );

        // nothing but introspection: the subgraph part is empty
        let (down, intro) =
            split("{ __schema { queryType { name } } }").expect("has introspection");
        assert_eq!(down, "");
        assert_eq!(intro, "{__schema{queryType{name}}}");
    }
}
