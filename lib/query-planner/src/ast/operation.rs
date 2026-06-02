use std::fmt::Display;

use crate::{
    ast::{document::Document, hash::ast_hash},
    planner::plan_nodes::hash_minified_query,
    state::supergraph_state::TypeNode,
};
use graphql_tools::parser::query as parser;
use serde::{Deserialize, Serialize};

use crate::{
    state::supergraph_state::OperationKind,
    utils::pretty_display::{get_indent, PrettyDisplay},
};

use super::{selection_item::SelectionItem, selection_set::SelectionSet};

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

impl OperationDefinition {
    pub fn parts(&self) -> (&OperationKind, &SelectionSet) {
        (
            self.operation_kind
                .as_ref()
                .unwrap_or(&OperationKind::Query),
            &self.selection_set,
        )
    }
    pub fn hash(&self) -> u64 {
        ast_hash(self)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubgraphFetchOperation {
    pub document: Document,
    pub document_str: String,
    pub hash: u64,
    /// All operations produced by the query planner are anonymous.
    /// The input query may contain name operation, but it's not used.
    /// It's by design, to avoid a situation where changing the operation name,
    /// requires to recompute the plan.
    ///
    /// The value is the position in the document where the operation name should be written.
    pub name_write_position: usize,
}

impl SubgraphFetchOperation {
    pub(crate) fn get_inner_selection_set(&self) -> &SelectionSet {
        if self.document.operation.selection_set.items.len() == 1 {
            if let SelectionItem::Field(field) = &self.document.operation.selection_set.items[0] {
                if field.name == "_entities" && field.alias.is_none() {
                    return &field.selections;
                }
            }
        }

        &self.document.operation.selection_set
    }

    pub fn from_anonymous_operation(document: Document) -> Self {
        let document_str = document.to_string();
        let hash = hash_minified_query(&document_str);

        // Find the operation name write position
        // by looking for the operation kind prefix in the document.
        // If no prefix is found, fall back to the start of the document.
        //
        // IMPORTANT NOTE:
        // Documents produced by `Display` impl of `OperationDefinition` always have the operation definition printed first, before fragments.
        //
        let name_write_position = ["query", "mutation", "subscription"]
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
            .unwrap_or(0);

        Self {
            document,
            document_str,
            hash,
            name_write_position,
        }
    }
}

impl Serialize for SubgraphFetchOperation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.document.to_string())
    }
}

impl PrettyDisplay for SubgraphFetchOperation {
    fn pretty_fmt(&self, f: &mut std::fmt::Formatter<'_>, depth: usize) -> std::fmt::Result {
        let indent = get_indent(depth);
        // TODO: improve
        let has_variables = self
            .document
            .operation
            .variable_definitions
            .as_ref()
            .is_some_and(|defs| {
                !defs.is_empty() && defs.iter().all(|v| v.variable_type.inner_type() != "_Any")
            });
        let kind: &str = match &self.document.operation.operation_kind {
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
        let variables =
            if let Some(variables) = self.document.operation.variable_definitions.as_ref() {
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
        self.get_inner_selection_set().pretty_fmt(f, depth + 2)?;
        writeln!(f, "{indent}  }}")?;

        if !self.document.fragments.is_empty() {
            for fragment in &self.document.fragments {
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
    pub default_value: Option<crate::ast::value::Value>,
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
    use super::SubgraphFetchOperation;
    use crate::{ast::document::Document, utils::parsing::parse_operation};
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

    fn parse_subgraph_fetch_operation(query: &str) -> SubgraphFetchOperation {
        SubgraphFetchOperation::from_anonymous_operation(parse_document(query))
    }

    struct InsertPosition<'a> {
        document: &'a str,
        start: usize,
    }

    impl<'a> InsertPosition<'a> {
        fn new(operation: &'a SubgraphFetchOperation) -> Self {
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
}
