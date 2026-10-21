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
    /// How this operation is written out for several groups of contextual argument values.
    /// Set by the planner on fetches with contextual arguments.
    #[serde(skip)]
    context_aliased: Option<Box<ContextAliasing>>,
}

// boxed so the field stays one pointer wide, plans hold one of these per fetch
#[derive(Debug, Clone)]
struct ContextAliasing {
    max_groups: usize,
    template: OnceLock<Option<ContextAliasTemplate>>,
}

/// Stands in for the group number in a [`ContextAliasTemplate`]. A GraphQL document can't
/// contain it, so it can't be mistaken for anything.
const GROUP: char = '\0';
// mark where the per-group variables and the fields end while the template gets minified,
// same deal, no document can contain these
const VARIABLES_START: &str = "\u{1}";
const VARIABLES_END: &str = "\u{2}";
const FIELDS_END: &str = "\u{3}";

/// An entity fetch operation cut into the parts every group of contextual argument values
/// shares and the parts each group gets its own copy of (top-level fields, fragments and the
/// per-group variables), so it can be written out for any number of groups. A group's copies get
/// `_{group}` added to their names.
///
/// For two groups, `query($representations:[_Any!]!$c:ID){_entities(representations:$representations){...on T{f(a:$c)}}}`
/// becomes `query($representations_0:[_Any!]!$c_0:ID$representations_1:[_Any!]!$c_1:ID){_entities_0:_entities(representations:$representations_0){...on T{f(a:$c_0)}}_entities_1:_entities(representations:$representations_1){...on T{f(a:$c_1)}}}`.
///
/// It's built once per fetch and is about as big as the operation. The copies are written per
/// request and dropped with it, so they don't pile up in the plan cache.
#[derive(Debug, Clone)]
pub struct ContextAliasTemplate {
    /// `query(` and the variables all groups share
    head: Box<str>,
    /// The per-group parts, with [`GROUP`] where the group number goes
    variables: Box<str>,
    fields: Box<str>,
    fragments: Box<str>,
    /// Two copies of the fields need a space between them, like `a:b c:d`
    space_between_fields: bool,
    /// The original names of the per-group variables
    group_variables: Box<[String]>,
    /// The original response key of every top-level field
    pub response_keys: Box<[String]>,
}

impl ContextAliasTemplate {
    /// Writes the operation out for `groups` groups.
    pub fn operation(&self, groups: usize) -> SubgraphFetchOperation {
        let mut document = String::with_capacity(
            self.head.len()
                + 3
                + groups * (self.variables.len() + self.fields.len() + self.fragments.len() + 8),
        );
        document.push_str(&self.head);
        for group in 0..groups {
            push_group(&mut document, &self.variables, group);
        }
        document.push_str("){");
        for group in 0..groups {
            if group > 0 && self.space_between_fields {
                document.push(' ');
            }
            push_group(&mut document, &self.fields, group);
        }
        document.push('}');
        for group in 0..groups {
            push_group(&mut document, &self.fragments, group);
        }
        SubgraphFetchOperation {
            hash: hash_minified_query(&document),
            name_write_position: name_write_position(&document),
            document_str: document.into_boxed_str(),
            context_aliased: None,
        }
    }

    /// The name of the variable `name` in the group `group`, if it is a per-group variable.
    pub fn variable(&self, name: &str, group: usize) -> Option<String> {
        self.group_variables
            .iter()
            .any(|variable| variable == name)
            .then(|| format!("{name}_{group}"))
    }

    /// Turns a response key like `_entities_3` into the index of its original key and its group.
    pub fn response_key(&self, alias: &str) -> Option<(usize, usize)> {
        let (key, group) = split_group(alias)?;
        let key = self
            .response_keys
            .iter()
            .position(|original| original == key)?;
        Some((key, group))
    }

    /// Custom-scalar paths rooted at the response keys of `groups` groups.
    pub fn custom_scalar_paths(
        &self,
        original: &CustomScalarPaths,
        groups: usize,
    ) -> CustomScalarPaths {
        CustomScalarPaths {
            children: self
                .response_keys
                .iter()
                .filter_map(|key| Some((key, original.children.get(key)?)))
                .flat_map(|(key, paths)| {
                    (0..groups).map(move |group| (format!("{key}_{group}"), paths.clone()))
                })
                .collect(),
            terminal: original.terminal,
        }
    }
}

fn push_group(document: &mut String, text: &str, group: usize) {
    use std::fmt::Write;
    for (index, piece) in text.split(GROUP).enumerate() {
        if index > 0 {
            let _ = write!(document, "{group}");
        }
        document.push_str(piece);
    }
}

/// Splits a name like `c_12` into `c` and `12`.
fn split_group(name: &str) -> Option<(&str, usize)> {
    let (name, group) = name.rsplit_once('_')?;
    if group.is_empty() || !group.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some((name, group.parse().ok()?))
}

impl SubgraphFetchOperation {
    /// Allows [`Self::context_alias_template`] to put up to `max_groups` groups of contextual
    /// argument values into one request. Called by the planner, a `max_groups` below 2 disables
    /// aliasing, `usize::MAX` means no limit.
    pub fn set_max_context_groups(&mut self, max_groups: usize) {
        self.context_aliased = (max_groups >= 2).then(|| {
            Box::new(ContextAliasing {
                max_groups,
                template: OnceLock::new(),
            })
        });
    }

    /// The most groups of contextual argument values one request may carry, at least 1.
    pub fn max_context_groups(&self) -> usize {
        self.context_aliased
            .as_ref()
            .map_or(1, |aliasing| aliasing.max_groups)
    }

    /// Returns the template to write this operation out for several groups, see
    /// [`ContextAliasTemplate`]. The `variables` get a separate copy per group, all other
    /// variables are shared. Built on first use and kept with the operation.
    ///
    /// Returns `None` when aliasing is off, or the operation can not be rewritten.
    pub fn context_alias_template(&self, variables: &[&str]) -> Option<&ContextAliasTemplate> {
        self.context_aliased
            .as_ref()?
            .template
            .get_or_init(|| build_context_alias_template(&self.document_str, variables))
            .as_ref()
    }
}

fn build_context_alias_template(
    document_str: &str,
    variables: &[&str],
) -> Option<ContextAliasTemplate> {
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
    // they'd be written between the variables and the fields, the planner never adds any
    if !query.directives.is_empty() {
        return None;
    }
    let fragment_names: Vec<String> = fragments.iter().map(|f| f.name.clone()).collect();

    let mut shared = Vec::new();
    let mut grouped = Vec::new();
    let mut group_variables = Vec::new();
    for definition in std::mem::take(&mut query.variable_definitions) {
        if variables.contains(&definition.name.as_str()) {
            group_variables.push(definition.name.clone());
            grouped.push(parser::VariableDefinition {
                name: format!("{}_{GROUP}", definition.name),
                ..definition
            });
        } else {
            shared.push(definition);
        }
    }
    if grouped.is_empty() {
        return None;
    }
    // a shared `$c_1` next to a per-group `$c` would clash with the copy for group 1
    if shared.iter().any(|definition| {
        split_group(&definition.name)
            .is_some_and(|(name, _)| group_variables.iter().any(|v| v == name))
    }) {
        return None;
    }

    let mut response_keys = Vec::with_capacity(query.selection_set.items.len());
    for selection in &mut query.selection_set.items {
        let parser::Selection::Field(field) = selection else {
            return None;
        };
        let key = field.alias.take().unwrap_or_else(|| field.name.clone());
        field.alias = Some(format!("{key}_{GROUP}"));
        response_keys.push(key);
        suffix_selection(selection, variables, &fragment_names, GROUP);
    }
    for fragment in &mut fragments {
        fragment.name = format!("{}_{GROUP}", fragment.name);
        suffix_directives(&mut fragment.directives, variables, GROUP);
        for selection in &mut fragment.selection_set.items {
            suffix_selection(selection, variables, &fragment_names, GROUP);
        }
    }

    // minify it all in one go, with markers where the shared and per-group parts meet
    let marker = |name: &str| parser::VariableDefinition {
        position: query.position,
        name: name.to_string(),
        var_type: parser::Type::NamedType(name.to_string()),
        default_value: None,
    };
    let (start, end) = (marker(VARIABLES_START), marker(VARIABLES_END));
    query.variable_definitions = shared;
    query.variable_definitions.push(start);
    query.variable_definitions.extend(grouped);
    query.variable_definitions.push(end);
    query
        .selection_set
        .items
        .push(parser::Selection::Field(parser::Field {
            position: query.position,
            alias: None,
            name: FIELDS_END.to_string(),
            arguments: Vec::new(),
            directives: Vec::new(),
            selection_set: parser::SelectionSet {
                span: query.selection_set.span,
                items: Vec::new(),
            },
        }));
    let mut definitions = vec![parser::Definition::Operation(
        parser::OperationDefinition::Query(query),
    )];
    definitions.extend(fragments.into_iter().map(parser::Definition::Fragment));
    let minified = parser::minify_query_document(&parser::Document { definitions });

    // query(<shared>$\1:\1<per-group>$\2:\2){<fields> \3}<fragments>
    let (head, rest) = minified.split_once(&format!("${VARIABLES_START}:{VARIABLES_START}"))?;
    let (variables, rest) = rest.split_once(&format!("${VARIABLES_END}:{VARIABLES_END}){{"))?;
    let (fields, fragments) = rest.split_once(FIELDS_END)?;
    let fields = fields.strip_suffix(' ').unwrap_or(fields);
    let fragments = fragments.strip_prefix('}')?;
    let is_word =
        |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == GROUP);

    Some(ContextAliasTemplate {
        head: head.into(),
        variables: variables.into(),
        space_between_fields: is_word(fields.chars().last()) && is_word(fields.chars().next()),
        fields: fields.into(),
        fragments: fragments.into(),
        group_variables: group_variables.into(),
        response_keys: response_keys.into(),
    })
}

/// Points the variables and fragment spreads in `selection` to their copies for `group`.
fn suffix_selection(
    selection: &mut parser::Selection<'_, String>,
    variables: &[&str],
    fragments: &[String],
    group: char,
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
    group: char,
) {
    for directive in directives {
        for (_, value) in &mut directive.arguments {
            suffix_value(value, variables, group);
        }
    }
}

fn suffix_value(value: &mut parser::Value<'_, String>, variables: &[&str], group: char) {
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
    fn context_alias_template_rejects_generated_variable_collisions() {
        for shared in ["__context_0_0", "representations_0", "representations_17"] {
            let mut operation = parse_subgraph_fetch_operation(&format!(
                "query($representations: [_Any!]!, $__context_0: String, ${shared}: Int) {{ _entities(representations: $representations) {{ ... on T {{ f(a: $__context_0, b: ${shared}) }} }} }}",
            ))
            .operation;
            operation.set_max_context_groups(usize::MAX);
            assert!(operation
                .context_alias_template(&["representations", "__context_0"])
                .is_none());
        }
    }

    #[test]
    fn context_alias_template_preserves_custom_scalars_under_generated_aliases() {
        use crate::executor::response::{subgraph_response::SubgraphResponse, value::Value};

        for key in ["_entities", "_e0"] {
            let mut operation = parse_subgraph_fetch_operation(&format!(
                "query($representations: [_Any!]!, $c: ID) {{ {key}: _entities(representations: $representations) {{ ... on T {{ json(a: $c) }} }} }}",
            ))
            .operation;
            operation.set_max_context_groups(2);
            let template = operation
                .context_alias_template(&["representations", "c"])
                .unwrap();
            let mut original = CustomScalarPaths::default();
            original.insert_path([key, "json"]);
            let paths = &template.custom_scalar_paths(&original, 2);
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
    fn context_alias_template_repeats_fields_variables_and_fragments_per_group() {
        let operation = parse_subgraph_fetch_operation(
            "query($representations: [_Any!]!, $c: ID, $shared: Int) { _entities(representations: $representations) { ...F ... on T @include(if: $c) { g(a: [{x: $c}], b: $shared) } } } fragment F on T { f(a: $c) }",
        );
        let mut operation = operation.operation;
        // not enabled by the planner
        assert!(operation
            .context_alias_template(&["representations", "c"])
            .is_none());
        operation.set_max_context_groups(usize::MAX);
        assert_eq!(operation.max_context_groups(), usize::MAX);

        let template = operation
            .context_alias_template(&["representations", "c"])
            .unwrap();
        let aliased = template.operation(2);
        insta::assert_snapshot!(aliased.document_str, @"query($shared:Int$representations_0:[_Any!]!$c_0:ID$representations_1:[_Any!]!$c_1:ID){_entities_0:_entities(representations:$representations_0){...F_0...on T@include(if:$c_0){g(a:[{x:$c_0}]b:$shared)}}_entities_1:_entities(representations:$representations_1){...F_1...on T@include(if:$c_1){g(a:[{x:$c_1}]b:$shared)}}}fragment F_0 on T{f(a:$c_0)}fragment F_1 on T{f(a:$c_1)}");
        assert_eq!(aliased.name_write_position, 5);
        assert_eq!(
            aliased.hash,
            crate::query_planner::planner::plan_nodes::hash_minified_query(&aliased.document_str)
        );
        assert_eq!(template.variable("c", 1).as_deref(), Some("c_1"));
        assert_eq!(template.variable("shared", 1), None);
        assert_eq!(&*template.response_keys, ["_entities".to_string()]);
        assert_eq!(template.response_key("_entities_12"), Some((0, 12)));
        assert_eq!(template.response_key("_entities_"), None);
        assert_eq!(template.response_key("other_1"), None);
        // one template for every group count
        assert!(std::ptr::eq(
            template,
            operation.context_alias_template(&[]).unwrap()
        ));
        // no limit on the group count
        let aliased = template.operation(40);
        assert!(graphql_tools::parser::parse_query::<String>(&aliased.document_str).is_ok());
        assert!(aliased
            .document_str
            .contains("_entities_39:_entities(representations:$representations_39)"));
        assert!(aliased
            .document_str
            .ends_with("fragment F_39 on T{f(a:$c_39)}"));
    }

    #[test]
    fn context_alias_template_separates_fields_that_end_with_a_name() {
        let mut operation =
            parse_subgraph_fetch_operation("query($r: [_Any!]!) { a(x: $r) b }").operation;
        operation.set_max_context_groups(usize::MAX);
        let template = operation.context_alias_template(&["r"]).unwrap();
        insta::assert_snapshot!(template.operation(2).document_str, @"query($r_0:[_Any!]!$r_1:[_Any!]!){a_0:a(x:$r_0)b_0:b a_1:a(x:$r_1)b_1:b}");
    }
}
