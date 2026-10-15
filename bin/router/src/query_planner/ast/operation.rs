use std::{fmt::Display, sync::OnceLock};

use crate::query_planner::{
    ast::{document::Document, hash::ast_hash},
    planner::plan_nodes::{hash_minified_query, CustomScalarPaths},
    state::supergraph_state::TypeNode,
};
use graphql_tools::parser::query as parser;
use serde::{Deserialize, Serialize};

use crate::query_planner::{
    state::supergraph_state::OperationKind,
    utils::pretty_display::{get_indent, PrettyDisplay},
};

use super::{selection_item::SelectionItem, selection_set::SelectionSet, shrink::ShrinkMemory};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationDefinition {
    pub name: Option<String>,
    // TODO: Should operation_kind be OperationKind or Option<OperationKind>?
    // I don't see a scenario where it should be set to None?
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_kind: Option<OperationKind>,
    pub selection_set: SelectionSet,
    pub variable_definitions: Option<Vec<VariableDefinition>>,
}

impl ShrinkMemory for OperationDefinition {
    fn shrink_memory(&mut self) {
        self.variable_definitions.shrink_memory();
        self.selection_set.shrink_memory();
    }
}

impl OperationDefinition {
    pub fn parts(&self) -> (&OperationKind, &SelectionSet) {
        (
            self.operation_kind
                .as_ref()
                .unwrap_or(&OperationKind::Query),
            &self.selection_set,
        )
    }
    pub fn hash(&self) -> u128 {
        ast_hash(self)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubgraphFetchOperation {
    pub document_str: Box<str>,
    pub hash: u64,
    /// All operations produced by the query planner are anonymous.
    /// The input query may contain name operation, but it's not used.
    /// It's by design, to avoid a situation where changing the operation name,
    /// requires to recompute the plan.
    ///
    /// The value is the position in the document where the operation name should be written.
    pub name_write_position: usize,
    /// Copies of this operation built by `context_aliased`, indexed by the number of groups.
    /// Set by the planner on fetches with contextual arguments, sized by the alias limit.
    #[serde(skip)]
    context_aliased: Option<Box<ContextAliasedCache>>,
}

// boxed twice so the field stays one pointer wide, plans hold one of these per fetch
#[derive(Debug, Clone)]
struct ContextAliasedCache(Box<[OnceLock<Option<ContextAliasedOperation>>]>);

/// An entity fetch operation with its top-level fields and per-group variables repeated once per
/// group of contextual argument values, so that all groups are fetched in a single request.
///
/// For two groups, `query($representations:[_Any!]!,$c:ID){_entities(representations:$representations){...on T{f(a:$c)}}}`
/// becomes `query($representations_0:[_Any!]!,$representations_1:[_Any!]!,$c_0:ID,$c_1:ID){_entities_0:_entities(representations:$representations_0){...on T{f(a:$c_0)}} _entities_1:_entities(representations:$representations_1){...on T{f(a:$c_1)}}}`.
#[derive(Debug, Clone)]
pub struct ContextAliasedOperation {
    pub operation: SubgraphFetchOperation,
    /// The original name of every per-group variable, and its name in each group.
    pub variables: Vec<(String, Vec<String>)>,
    /// The original response key of every top-level field, and its alias in each group.
    pub response_keys: Vec<(String, Vec<String>)>,
    custom_scalar_paths: OnceLock<CustomScalarPaths>,
}

impl ContextAliasedOperation {
    /// Returns custom-scalar paths rooted at the generated response aliases.
    pub fn custom_scalar_paths(&self, original: &CustomScalarPaths) -> &CustomScalarPaths {
        self.custom_scalar_paths.get_or_init(|| CustomScalarPaths {
            children: self
                .response_keys
                .iter()
                .filter_map(|(key, aliases)| {
                    original.children.get(key).map(|paths| (aliases, paths))
                })
                .flat_map(|(aliases, paths)| {
                    aliases
                        .iter()
                        .map(move |alias| (alias.clone(), paths.clone()))
                })
                .collect(),
            terminal: original.terminal,
        })
    }

    /// The name of the variable `name` in the group `group`, if it is a per-group variable.
    pub fn variable(&self, name: &str, group: usize) -> Option<&str> {
        self.variables
            .iter()
            .find(|(original, _)| original == name)
            .map(|(_, names)| names[group].as_str())
    }
}

impl SubgraphFetchOperation {
    /// Allows [`Self::context_aliased`] to put up to `max_groups` groups of contextual argument
    /// values into one request. Called by the planner, a `max_groups` below 2 disables aliasing.
    pub fn set_max_context_groups(&mut self, max_groups: usize) {
        self.context_aliased = (max_groups >= 2).then(|| {
            Box::new(ContextAliasedCache(
                (0..=max_groups).map(|_| OnceLock::new()).collect(),
            ))
        });
    }

    /// The most groups of contextual argument values one request may carry, at least 1.
    pub fn max_context_groups(&self) -> usize {
        self.context_aliased
            .as_ref()
            .map_or(1, |cache| cache.0.len() - 1)
    }

    /// Returns this operation with every top-level field repeated once per group, see
    /// [`ContextAliasedOperation`]. The `variables` get a separate copy per group, all other
    /// variables are shared. The result is built once per group count and cached.
    ///
    /// Returns `None` when `groups` is not between 2 and [`Self::max_context_groups`], or the
    /// operation can not be rewritten.
    pub fn context_aliased(
        &self,
        groups: usize,
        variables: &[&str],
    ) -> Option<&ContextAliasedOperation> {
        if groups < 2 {
            return None;
        }
        self.context_aliased
            .as_ref()?
            .0
            .get(groups)?
            .get_or_init(|| build_context_aliased(&self.document_str, groups, variables))
            .as_ref()
    }
}

fn build_context_aliased(
    document_str: &str,
    groups: usize,
    variables: &[&str],
) -> Option<ContextAliasedOperation> {
    let document = parser::parse_query::<String>(document_str).ok()?;
    let mut query = None;
    let mut fragments = Vec::new();
    for definition in document.definitions {
        match definition {
            parser::Definition::Operation(parser::OperationDefinition::Query(operation))
                if query.is_none() =>
            {
                query = Some(operation)
            }
            parser::Definition::Fragment(fragment) => fragments.push(fragment),
            _ => return None,
        }
    }
    let mut query = query?;
    let fragment_names: Vec<String> = fragments.iter().map(|f| f.name.clone()).collect();
    let suffixed = |name: &str| {
        (0..groups)
            .map(|g| format!("{name}_{g}"))
            .collect::<Vec<_>>()
    };

    let mut renamed = Vec::new();
    for definition in std::mem::take(&mut query.variable_definitions) {
        if !variables.contains(&definition.name.as_str()) {
            query.variable_definitions.push(definition);
            continue;
        }
        let names = suffixed(&definition.name);
        for name in &names {
            query.variable_definitions.push(parser::VariableDefinition {
                name: name.clone(),
                ..definition.clone()
            });
        }
        renamed.push((definition.name, names));
    }

    let mut names = std::collections::HashSet::new();
    if query
        .variable_definitions
        .iter()
        .any(|definition| !names.insert(definition.name.as_str()))
    {
        return None;
    }

    let fields = std::mem::take(&mut query.selection_set.items);
    let mut response_keys = Vec::with_capacity(fields.len());
    for selection in &fields {
        let parser::Selection::Field(field) = selection else {
            return None;
        };
        let key = field.alias.as_ref().unwrap_or(&field.name);
        response_keys.push((key.clone(), suffixed(key)));
    }
    for group in 0..groups {
        for (selection, (_, aliases)) in fields.iter().zip(&response_keys) {
            let mut selection = selection.clone();
            if let parser::Selection::Field(field) = &mut selection {
                field.alias = Some(aliases[group].clone());
            }
            suffix_selection(&mut selection, variables, &fragment_names, group);
            query.selection_set.items.push(selection);
        }
    }

    let mut definitions = vec![parser::Definition::Operation(
        parser::OperationDefinition::Query(query),
    )];
    for group in 0..groups {
        for fragment in &fragments {
            let mut fragment = fragment.clone();
            fragment.name = format!("{}_{group}", fragment.name);
            suffix_directives(&mut fragment.directives, variables, group);
            for selection in &mut fragment.selection_set.items {
                suffix_selection(selection, variables, &fragment_names, group);
            }
            definitions.push(parser::Definition::Fragment(fragment));
        }
    }

    let document_str = parser::minify_query_document(&parser::Document { definitions });
    Some(ContextAliasedOperation {
        operation: SubgraphFetchOperation {
            hash: hash_minified_query(&document_str),
            name_write_position: name_write_position(&document_str),
            document_str: document_str.into_boxed_str(),
            context_aliased: None,
        },
        variables: renamed,
        response_keys,
        custom_scalar_paths: OnceLock::new(),
    })
}

/// Points the variables and fragment spreads in `selection` to their copies for `group`.
fn suffix_selection(
    selection: &mut parser::Selection<'_, String>,
    variables: &[&str],
    fragments: &[String],
    group: usize,
) {
    let items = match selection {
        parser::Selection::Field(field) => {
            for (_, value) in &mut field.arguments {
                suffix_value(value, variables, group);
            }
            suffix_directives(&mut field.directives, variables, group);
            &mut field.selection_set.items
        }
        parser::Selection::FragmentSpread(spread) => {
            if fragments.contains(&spread.fragment_name) {
                spread.fragment_name = format!("{}_{group}", spread.fragment_name);
            }
            suffix_directives(&mut spread.directives, variables, group);
            return;
        }
        parser::Selection::InlineFragment(fragment) => {
            suffix_directives(&mut fragment.directives, variables, group);
            &mut fragment.selection_set.items
        }
    };
    for item in items {
        suffix_selection(item, variables, fragments, group);
    }
}

fn suffix_directives(
    directives: &mut [parser::Directive<'_, String>],
    variables: &[&str],
    group: usize,
) {
    for directive in directives {
        for (_, value) in &mut directive.arguments {
            suffix_value(value, variables, group);
        }
    }
}

fn suffix_value(value: &mut parser::Value<'_, String>, variables: &[&str], group: usize) {
    match value {
        parser::Value::Variable(name) if variables.contains(&name.as_str()) => {
            *name = format!("{name}_{group}");
        }
        parser::Value::List(items) => {
            for item in items {
                suffix_value(item, variables, group);
            }
        }
        parser::Value::Object(fields) => {
            for (_, item) in fields {
                suffix_value(item, variables, group);
            }
        }
        _ => {}
    }
}

/// Finds the operation name write position by looking for the operation kind prefix in the
/// document. If no prefix is found, falls back to the start of the document.
///
/// IMPORTANT NOTE:
/// Documents produced by `Display` impl of `OperationDefinition` always have the operation definition printed first, before fragments.
fn name_write_position(document_str: &str) -> usize {
    ["query", "mutation", "subscription"]
        .into_iter()
        .find_map(|operation_kind| {
            document_str
                .strip_prefix(operation_kind)
                .and_then(|suffix| {
                    if suffix.starts_with('(') || suffix.starts_with('{') {
                        Some(operation_kind.len())
                    } else {
                        None
                    }
                })
        })
        .unwrap_or(0)
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlanningFetchOperation {
    pub document: Box<Document>,
    #[serde(flatten)]
    pub operation: SubgraphFetchOperation,
}

impl AsRef<SubgraphFetchOperation> for SubgraphFetchOperation {
    fn as_ref(&self) -> &SubgraphFetchOperation {
        self
    }
}

impl AsRef<SubgraphFetchOperation> for PlanningFetchOperation {
    fn as_ref(&self) -> &SubgraphFetchOperation {
        &self.operation
    }
}

/// Skips the `_entities` wrapper when printing a selection that is sent to a subgraph.
pub(crate) fn inner_selection_set(document: &Document) -> &SelectionSet {
    if document.operation.selection_set.items.len() == 1 {
        if let SelectionItem::Field(field) = &document.operation.selection_set.items[0] {
            if field.name == "_entities" && field.alias.is_none() {
                return &field.selections;
            }
        }
    }

    &document.operation.selection_set
}

impl PlanningFetchOperation {
    pub fn from_anonymous_operation(document: Document) -> Self {
        let document_str = document.to_string();
        let hash = hash_minified_query(&document_str);
        let name_write_position = name_write_position(&document_str);

        Self {
            document: Box::new(document),
            operation: SubgraphFetchOperation {
                document_str: document_str.into_boxed_str(),
                hash,
                name_write_position,
                context_aliased: None,
            },
        }
    }
}

impl Serialize for SubgraphFetchOperation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.document_str)
    }
}

impl Serialize for PlanningFetchOperation {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.operation.serialize(serializer)
    }
}

impl PrettyDisplay for SubgraphFetchOperation {
    fn pretty_fmt(&self, f: &mut std::fmt::Formatter<'_>, depth: usize) -> std::fmt::Result {
        writeln!(f, "{}  {}", get_indent(depth), self.document_str)
    }
}

impl PrettyDisplay for PlanningFetchOperation {
    fn pretty_fmt(&self, f: &mut std::fmt::Formatter<'_>, depth: usize) -> std::fmt::Result {
        let indent = get_indent(depth);
        let document = &self.document;
        // TODO: improve
        let has_variables = document
            .operation
            .variable_definitions
            .as_ref()
            .is_some_and(|defs| {
                !defs.is_empty() && defs.iter().all(|v| v.variable_type.inner_type() != "_Any")
            });
        let kind: &str = match &document.operation.operation_kind {
            Some(kind) => match kind {
                OperationKind::Query => match has_variables {
                    true => "query ",
                    false => "",
                },
                OperationKind::Mutation => "mutation ",
                OperationKind::Subscription => "subscription ",
            },
            None => "",
        };
        let variables = if let Some(variables) = document.operation.variable_definitions.as_ref() {
            let representationless = variables
                .iter()
                .filter(|v| v.variable_type.inner_type() != "_Any")
                .collect::<Vec<_>>();

            if representationless.is_empty() {
                "".to_string()
            } else {
                format!(
                    "({}) ",
                    representationless
                        .iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<String>>()
                        .join(",")
                )
            }
        } else {
            "".to_string()
        };
        writeln!(f, "{indent}  {kind}{variables}{{")?;
        inner_selection_set(document).pretty_fmt(f, depth + 2)?;
        writeln!(f, "{indent}  }}")?;

        if !document.fragments.is_empty() {
            for fragment in &document.fragments {
                fragment.pretty_fmt(f, depth)?;
            }
        }

        Ok(())
    }
}

impl Display for OperationDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self
            .operation_kind
            .as_ref()
            .is_none_or(|k| matches!(k, OperationKind::Query))
            && self.name.as_deref().is_none_or(str::is_empty)
            && self.variable_definitions.as_ref().is_none_or(Vec::is_empty)
        {
            // Short form for anonymous query
            return self.selection_set.fmt(f);
        }
        if let Some(operation_kind) = &self.operation_kind {
            write!(f, "{}", operation_kind)?;
        }

        if let Some(name) = &self.name {
            write!(f, " {} ", name)?;
        }

        if let Some(variable_definitions) = &self.variable_definitions {
            if !variable_definitions.is_empty() {
                f.write_str("(")?;
                let mut iter = variable_definitions.iter().peekable();
                while let Some(variable_definition) = iter.next() {
                    variable_definition.fmt(f)?;
                    if iter.peek().is_some() {
                        f.write_str(", ")?;
                    }
                }
                f.write_str(")")?;
            }
        }

        self.selection_set.fmt(f)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableDefinition {
    pub name: String,
    pub variable_type: TypeNode,
    pub default_value: Option<crate::query_planner::ast::value::Value>,
}

impl ShrinkMemory for VariableDefinition {
    fn shrink_memory(&mut self) {
        self.default_value.shrink_memory();
    }
}

impl VariableDefinition {
    /// Checks if this variable definition is compatible with another
    pub fn can_merge(&self, other: &Self) -> bool {
        if self.name != other.name {
            return false;
        }

        self.variable_type == other.variable_type && self.default_value == other.default_value
    }
}

impl Display for VariableDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.default_value {
            Some(default_value) => {
                write!(f, "${}:{}={}", self.name, self.variable_type, default_value)
            }
            None => write!(f, "${}:{}", self.name, self.variable_type),
        }
    }
}

impl<'a, T: parser::Text<'a>> From<parser::OperationDefinition<'a, T>> for OperationDefinition {
    fn from(value: parser::OperationDefinition<'a, T>) -> Self {
        match value {
            parser::OperationDefinition::Query(query) => OperationDefinition {
                name: query.name.map(|n| n.as_ref().to_string()),
                operation_kind: Some(OperationKind::Query),
                variable_definitions: match query.variable_definitions.len() {
                    0 => None,
                    _ => Some(
                        query
                            .variable_definitions
                            .into_iter()
                            .map(|v| v.into())
                            .collect(),
                    ),
                },
                selection_set: query.selection_set.into(),
            },
            parser::OperationDefinition::SelectionSet(s) => OperationDefinition {
                name: None,
                operation_kind: Some(OperationKind::Query),
                variable_definitions: None,
                selection_set: s.into(),
            },
            parser::OperationDefinition::Mutation(mutation) => OperationDefinition {
                name: mutation.name.map(|n| n.as_ref().to_string()),
                operation_kind: Some(OperationKind::Mutation),
                variable_definitions: match mutation.variable_definitions.len() {
                    0 => None,
                    _ => Some(
                        mutation
                            .variable_definitions
                            .into_iter()
                            .map(|v| v.into())
                            .collect(),
                    ),
                },
                selection_set: mutation.selection_set.into(),
            },
            parser::OperationDefinition::Subscription(subscription) => OperationDefinition {
                name: subscription.name.map(|n| n.as_ref().to_string()),
                operation_kind: Some(OperationKind::Subscription),
                variable_definitions: match subscription.variable_definitions.len() {
                    0 => None,
                    _ => Some(
                        subscription
                            .variable_definitions
                            .into_iter()
                            .map(|v| v.into())
                            .collect(),
                    ),
                },
                selection_set: subscription.selection_set.into(),
            },
        }
    }
}

impl<'a, T: parser::Text<'a>> From<&parser::VariableDefinition<'a, T>> for VariableDefinition {
    fn from(value: &parser::VariableDefinition<'a, T>) -> Self {
        VariableDefinition {
            name: value.name.as_ref().to_string(),
            variable_type: (&value.var_type).into(),
            default_value: value.default_value.as_ref().map(|v| v.into()),
        }
    }
}

impl<'a, T: parser::Text<'a>> From<parser::VariableDefinition<'a, T>> for VariableDefinition {
    fn from(value: parser::VariableDefinition<'a, T>) -> Self {
        VariableDefinition {
            name: value.name.as_ref().to_string(),
            variable_type: (&value.var_type).into(),
            default_value: value.default_value.as_ref().map(|v| v.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CustomScalarPaths, PlanningFetchOperation};
    use crate::query_planner::{ast::document::Document, utils::parsing::parse_operation};
    use graphql_tools::parser::query::Definition;
    use std::fmt;

    fn parse_document(query: &str) -> Document {
        let document = parse_operation(query);
        let mut operation = None;
        let mut fragments = Vec::new();

        for definition in document.definitions {
            match definition {
                Definition::Operation(current_operation) => {
                    if operation.is_none() {
                        operation = Some(current_operation.into());
                    }
                }
                Definition::Fragment(fragment) => fragments.push(fragment.into()),
            }
        }

        Document {
            operation: operation.expect("operation definition should exist"),
            fragments,
        }
    }

    fn parse_subgraph_fetch_operation(query: &str) -> PlanningFetchOperation {
        PlanningFetchOperation::from_anonymous_operation(parse_document(query))
    }

    struct InsertPosition<'a> {
        document: &'a str,
        start: usize,
    }

    impl<'a> InsertPosition<'a> {
        fn new(operation: &'a PlanningFetchOperation) -> Self {
            let operation = operation.as_ref();
            Self {
                document: &operation.document_str,
                start: operation.name_write_position,
            }
        }
    }

    impl fmt::Display for InsertPosition<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            for _ in 0..self.start {
                write!(f, " ")?;
            }
            writeln!(f, "↓ {}", self.start)?;
            writeln!(f, "{}", self.document)?;
            Ok(())
        }
    }

    #[test]
    fn operation_name_position_tests() {
        let operation = parse_subgraph_fetch_operation("{ field }");
        insta::assert_snapshot!(
            InsertPosition::new(&operation),
            @r"
            ↓ 0
            {field}
            "
        );

        let operation = parse_subgraph_fetch_operation("{ query mutation: subscription }");
        insta::assert_snapshot!(
            InsertPosition::new(&operation),
            @r"
            ↓ 0
            {query mutation: subscription}
            "
        );

        let operation = parse_subgraph_fetch_operation(
            "query($id: ID!) { node(id: $id) { aliasQuery: query } }",
        );
        insta::assert_snapshot!(
            InsertPosition::new(&operation),
            @r"
             ↓ 5
        query($id:ID!){node(id: $id){aliasQuery: query}}
        "
        );

        let operation = parse_subgraph_fetch_operation("mutation { updateUser(id: 1) { name } }");
        insta::assert_snapshot!(
            InsertPosition::new(&operation),
            @r"
                    ↓ 8
            mutation{updateUser(id: 1){name}}
            "
        );

        let operation = parse_subgraph_fetch_operation("subscription { onMessage { text } }");
        insta::assert_snapshot!(
            InsertPosition::new(&operation),
            @r"
                        ↓ 12
            subscription{onMessage{text}}
            "
        );

        let operation =
            parse_subgraph_fetch_operation("mutation($x: Int!) { updateUser(x: $x) { name } }");
        insta::assert_snapshot!(
            InsertPosition::new(&operation),
            @r"
                    ↓ 8
            mutation($x:Int!){updateUser(x: $x){name}}
            "
        );

        let operation = parse_subgraph_fetch_operation(
            "subscription($channel: String!) { onMessage(channel: $channel) { text } }",
        );
        insta::assert_snapshot!(
            InsertPosition::new(&operation),
            @r"
                        ↓ 12
            subscription($channel:String!){onMessage(channel: $channel){text}}
            "
        );

        let operation = parse_subgraph_fetch_operation(
            "mutation { query mutation subscription query: update mutation: updateAlias subscription: updateSubscription }",
        );
        insta::assert_snapshot!(
            InsertPosition::new(&operation),
            @r"
                    ↓ 8
            mutation{query mutation subscription query: update mutation: updateAlias subscription: updateSubscription}
            "
        );

        let operation = parse_subgraph_fetch_operation(
            "subscription { query mutation subscription query: onQuery mutation: onMutation subscription: onSubscription }",
        );
        insta::assert_snapshot!(
            InsertPosition::new(&operation),
            @r"
                        ↓ 12
            subscription{query mutation subscription query: onQuery mutation: onMutation subscription: onSubscription}
            "
        );

        let operation = parse_subgraph_fetch_operation(
            "query($id: ID!) { node(id: $id) { ...QueryFields } } fragment QueryFields on Node { query mutation: subscription }",
        );
        insta::assert_snapshot!(
            InsertPosition::new(&operation),
            @r"
                 ↓ 5
            query($id:ID!){node(id: $id){...QueryFields}}

            fragment QueryFields on Node {query mutation: subscription}
            "
        );

        let operation = parse_subgraph_fetch_operation(
            "fragment QueryFields on Node { query mutation: subscription } query($id: ID!) { node(id: $id) { ...QueryFields } }",
        );
        insta::assert_snapshot!(
            InsertPosition::new(&operation),
            @r"
                 ↓ 5
            query($id:ID!){node(id: $id){...QueryFields}}

            fragment QueryFields on Node {query mutation: subscription}
            "
        );

        let operation = parse_subgraph_fetch_operation("query query { field }");
        insta::assert_snapshot!(
            InsertPosition::new(&operation),
            @r"
            ↓ 0
            query query {field}
            "
        );
    }

    #[test]
    fn context_aliased_rejects_generated_variable_collisions() {
        for shared in ["__context_0_0", "representations_0"] {
            let mut operation = parse_subgraph_fetch_operation(&format!(
                "query($representations: [_Any!]!, $__context_0: String, ${shared}: Int) {{ _entities(representations: $representations) {{ ... on T {{ f(a: $__context_0, b: ${shared}) }} }} }}",
            ))
            .operation;
            operation.set_max_context_groups(2);
            assert!(operation
                .context_aliased(2, &["representations", "__context_0"])
                .is_none());
        }
    }

    #[test]
    fn context_aliased_preserves_custom_scalars_under_generated_aliases() {
        use crate::executor::response::{subgraph_response::SubgraphResponse, value::Value};

        for key in ["_entities", "_e0"] {
            let mut operation = parse_subgraph_fetch_operation(&format!(
                "query($representations: [_Any!]!, $c: ID) {{ {key}: _entities(representations: $representations) {{ ... on T {{ json(a: $c) }} }} }}",
            ))
            .operation;
            operation.set_max_context_groups(2);
            let aliased = operation
                .context_aliased(2, &["representations", "c"])
                .unwrap();
            let mut original = CustomScalarPaths::default();
            original.insert_path([key, "json"]);
            let paths = aliased.custom_scalar_paths(&original);
            assert!(!paths.children.contains_key(key));
            for group in 0..2 {
                let alias = format!("{key}_{group}");
                assert_eq!(paths.children[&alias], original.children[key]);
                let response = SubgraphResponse::deserialize_from_bytes(
                    bytes::Bytes::from(format!(
                        r#"{{"data":{{"{alias}":[{{"json":{{"escaped\tkey":1}}}}]}}}}"#,
                    )),
                    Some(paths),
                )
                .unwrap();
                let Value::Object(fields) = response.data else {
                    panic!("object")
                };
                let Value::Array(entities) = &fields[0].1 else {
                    panic!("array")
                };
                let Value::Object(entity) = &entities[0] else {
                    panic!("entity")
                };
                assert!(matches!(entity[0].1, Value::RawJson(_)));
            }
            assert!(original.children.contains_key(key));
        }
    }

    #[test]
    fn context_aliased_repeats_fields_variables_and_fragments_per_group() {
        let operation = parse_subgraph_fetch_operation(
            "query($representations: [_Any!]!, $c: ID, $shared: Int) { _entities(representations: $representations) { ...F ... on T @include(if: $c) { g(a: [{x: $c}], b: $shared) } } } fragment F on T { f(a: $c) }",
        );
        let mut operation = operation.operation;
        // not enabled by the planner
        assert!(operation
            .context_aliased(2, &["representations", "c"])
            .is_none());
        operation.set_max_context_groups(15);
        assert_eq!(operation.max_context_groups(), 15);
        assert!(operation
            .context_aliased(1, &["representations", "c"])
            .is_none());
        assert!(operation
            .context_aliased(16, &["representations", "c"])
            .is_none());

        let aliased = operation
            .context_aliased(2, &["representations", "c"])
            .unwrap();
        insta::assert_snapshot!(aliased.operation.document_str, @"query($representations_0:[_Any!]!$representations_1:[_Any!]!$c_0:ID$c_1:ID$shared:Int){_entities_0:_entities(representations:$representations_0){...F_0...on T@include(if:$c_0){g(a:[{x:$c_0}]b:$shared)}}_entities_1:_entities(representations:$representations_1){...F_1...on T@include(if:$c_1){g(a:[{x:$c_1}]b:$shared)}}}fragment F_0 on T{f(a:$c_0)}fragment F_1 on T{f(a:$c_1)}");
        assert_eq!(aliased.operation.name_write_position, 5);
        assert_eq!(aliased.variable("c", 1), Some("c_1"));
        assert_eq!(aliased.variable("shared", 1), None);
        assert_eq!(
            aliased.response_keys,
            vec![(
                "_entities".to_string(),
                vec!["_entities_0".to_string(), "_entities_1".to_string()]
            )]
        );
        // cached per group count
        assert!(std::ptr::eq(
            aliased,
            operation.context_aliased(2, &[]).unwrap()
        ));
    }
}
