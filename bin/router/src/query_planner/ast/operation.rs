use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    hash::{Hash, Hasher},
};

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
    /// Compiled by the planner on fetches with contextual arguments, boxed so the field stays one
    /// pointer wide.
    #[serde(skip)]
    context_template: Option<Box<ContextAliasTemplate>>,
}

/// An entity fetch operation cut into the parts every group of contextual argument values
/// shares and the parts each group gets its own copy of (top-level fields, fragments and the
/// per-group variables), so it can be written out for any number of groups. A group's copies get
/// `_{group}` added to their names.
///
/// For two groups, `query($representations:[_Any!]!$c:ID){_entities(representations:$representations){...on T{f(a:$c)}}}`
/// becomes `query($representations_0:[_Any!]!$c_0:ID$representations_1:[_Any!]!$c_1:ID){_entities_0:_entities(representations:$representations_0){...on T{f(a:$c_0)}} _entities_1:_entities(representations:$representations_1){...on T{f(a:$c_1)}}}`.
///
/// It's compiled once with the plan and is about as big as the operation. The copies are written
/// per request and dropped with it, so they don't pile up in the plan cache.
#[derive(Debug, Clone)]
pub struct ContextAliasTemplate {
    /// The most groups one request may carry
    max_groups: usize,
    /// The hash of the operation it's compiled from
    hash: u64,
    /// `query(` and the variables all groups share
    head: Box<str>,
    /// The parts every group gets its own copy of
    variables: TemplateSection,
    fields: TemplateSection,
    fragments: TemplateSection,
    /// The original names of the per-group variables
    group_variables: Box<[String]>,
    /// The original response key of every top-level field
    pub response_keys: Box<[String]>,
    /// The fetch's custom-scalar paths, read through the group number of every response key, so
    /// one copy works for any number of groups
    custom_scalar_paths: Option<CustomScalarPaths>,
}

/// Text every group gets a copy of, with the byte offsets where the group number goes.
#[derive(Debug, Clone)]
struct TemplateSection {
    text: Box<str>,
    group_positions: Box<[u32]>,
}

impl TemplateSection {
    fn write(&self, document: &mut String, group: &str) {
        let mut start = 0;
        for &position in &self.group_positions {
            document.push_str(&self.text[start..position as usize]);
            document.push_str(group);
            start = position as usize;
        }
        document.push_str(&self.text[start..]);
    }

    /// The length of one copy, with a group number of `digits` digits.
    fn len(&self, digits: usize) -> usize {
        self.text.len() + self.group_positions.len() * digits
    }
}

impl ContextAliasTemplate {
    /// Writes the operation out for `groups` groups.
    pub fn operation(&self, groups: usize) -> SubgraphFetchOperation {
        let digits = groups.max(1).ilog10() as usize + 1;
        let copy =
            self.variables.len(digits) + self.fields.len(digits) + self.fragments.len(digits);
        let mut document = String::with_capacity(self.head.len() + 3 + groups * (copy + 1));
        let mut number = itoa::Buffer::new();
        document.push_str(&self.head);
        for group in 0..groups {
            self.variables.write(&mut document, number.format(group));
        }
        document.push_str("){");
        for group in 0..groups {
            // fields can end and start with a name, like `a:b c:d`
            if group > 0 {
                document.push(' ');
            }
            self.fields.write(&mut document, number.format(group));
        }
        document.push('}');
        for group in 0..groups {
            self.fragments.write(&mut document, number.format(group));
        }
        // what's written only depends on the template and the group count, so there's no need to
        // hash all of it
        let mut hasher = xxhash_rust::xxh3::Xxh3::new();
        (self.hash, groups).hash(&mut hasher);
        SubgraphFetchOperation {
            hash: hasher.finish(),
            name_write_position: name_write_position(&document),
            document_str: document.into_boxed_str(),
            context_template: None,
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

    /// The custom-scalar paths of the requests it writes, for any number of groups.
    pub fn custom_scalar_paths(&self) -> Option<&CustomScalarPaths> {
        self.custom_scalar_paths.as_ref()
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
    /// Lets up to `max_groups` groups of contextual argument values go in one request, each with
    /// its own copy of `group_variables`, by compiling a [`ContextAliasTemplate`]. Called by the
    /// planner once the operation is final, with the fetch's `custom_scalar_paths`. A `max_groups`
    /// below 2, or an operation that can't be written out per group, leaves aliasing off.
    /// `usize::MAX` means no limit.
    pub fn set_max_context_groups(
        &mut self,
        max_groups: usize,
        group_variables: Vec<String>,
        custom_scalar_paths: Option<&CustomScalarPaths>,
    ) {
        self.context_template = (max_groups >= 2)
            .then(|| {
                let mut template = compile_context_alias_template(
                    &self.document_str,
                    max_groups,
                    group_variables,
                )?;
                template.custom_scalar_paths = custom_scalar_paths.map(|paths| CustomScalarPaths {
                    group_suffixed: true,
                    ..paths.clone()
                });
                Some(Box::new(template))
            })
            .flatten();
    }

    /// The most groups of contextual argument values one request may carry, at least 1.
    pub fn max_context_groups(&self) -> usize {
        self.context_template
            .as_ref()
            .map_or(1, |template| template.max_groups)
    }

    /// The template to write this operation out for several groups, `None` when aliasing is off.
    pub fn context_alias_template(&self) -> Option<&ContextAliasTemplate> {
        self.context_template.as_deref()
    }
}

/// Compiles `document_str` into a [`ContextAliasTemplate`]. The `variables` get a separate copy
/// per group, all other variables are shared.
///
/// Every name that gets a group number is swapped for a placeholder name that isn't a name in the
/// operation, and the result is minified. The placeholders are then found among the tokens of
/// the output, so they only ever match whole names. A string is a single token, so nothing in
/// one is taken for a placeholder.
fn compile_context_alias_template(
    document_str: &str,
    max_groups: usize,
    variables: Vec<String>,
) -> Option<ContextAliasTemplate> {
    use graphql_tools::parser::{spanned_tokens, TokenKind};

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

    // placeholders are short numbered names, skipping any that's already a name in the operation,
    // so each one matches nothing but itself
    let original_tokens = spanned_tokens(document_str).ok()?;
    let taken: HashSet<&str> = original_tokens
        .iter()
        .filter(|(kind, _)| *kind == TokenKind::Name)
        .map(|(_, span)| &document_str[span.clone()])
        .collect();
    // what every placeholder stands for, the group number goes after it
    let mut placeholders: HashMap<String, String> = HashMap::new();
    let mut next = 0usize;
    let mut placeholder = |replacement: String| {
        let name = loop {
            let candidate = format!("__hive_tpl_{next}");
            next += 1;
            if !taken.contains(candidate.as_str()) {
                break candidate;
            }
        };
        placeholders.insert(name.clone(), replacement);
        name
    };

    let mut shared = Vec::new();
    let mut grouped = Vec::new();
    let mut renamed_variables = Vec::new();
    for definition in std::mem::take(&mut query.variable_definitions) {
        if variables.contains(&definition.name) {
            let name = placeholder(format!("{}_", definition.name));
            renamed_variables.push((definition.name.clone(), name.clone()));
            grouped.push(parser::VariableDefinition { name, ..definition });
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
            .is_some_and(|(name, _)| renamed_variables.iter().any(|(v, _)| v == name))
    }) {
        return None;
    }

    let renamed_fragments: Vec<(String, String)> = fragments
        .iter()
        .map(|fragment| {
            (
                fragment.name.clone(),
                placeholder(format!("{}_", fragment.name)),
            )
        })
        .collect();
    let mut response_keys = Vec::with_capacity(query.selection_set.items.len());
    for selection in &mut query.selection_set.items {
        let parser::Selection::Field(field) = selection else {
            return None;
        };
        let key = field.alias.take().unwrap_or_else(|| field.name.clone());
        field.alias = Some(placeholder(format!("{key}_")));
        response_keys.push(key);
        rename_selection(selection, &renamed_variables, &renamed_fragments);
    }
    for (fragment, (_, name)) in fragments.iter_mut().zip(&renamed_fragments) {
        fragment.name = name.clone();
        rename_directives(&mut fragment.directives, &renamed_variables);
        for selection in &mut fragment.selection_set.items {
            rename_selection(selection, &renamed_variables, &renamed_fragments);
        }
    }

    let shared_count = shared.len();
    query.variable_definitions = shared;
    query.variable_definitions.extend(grouped);
    let mut definitions = vec![parser::Definition::Operation(
        parser::OperationDefinition::Query(query),
    )];
    definitions.extend(fragments.into_iter().map(parser::Definition::Fragment));
    let printed = parser::minify_query_document(&parser::Document { definitions });

    // query(<shared><per-group>){<fields>}<fragments>
    let tokens = spanned_tokens(&printed).ok()?;
    let is = |index: usize, text: &str| {
        tokens.get(index).is_some_and(|(kind, span)| {
            *kind == TokenKind::Punctuator && &printed[span.clone()] == text
        })
    };
    let open_variables = (1..=2).find(|index| is(*index, "("))?;
    let close_variables = closing_token(&printed, &tokens, open_variables)?;
    // defaults are constants, so every `$` in here starts a definition
    let grouped_start = (open_variables + 1..close_variables)
        .filter(|index| is(*index, "$"))
        .nth(shared_count)?;
    let open_fields = close_variables + 1;
    if !is(open_fields, "{") {
        return None;
    }
    let close_fields = closing_token(&printed, &tokens, open_fields)?;

    // the text of the tokens in `range`, from the end of the token before it
    let section = |range: std::ops::Range<usize>| -> Option<TemplateSection> {
        let start = tokens[range.start - 1].1.end;
        let end = tokens
            .get(range.end)
            .map_or(printed.len(), |(_, span)| span.start);
        let mut text = String::with_capacity(end - start);
        let mut group_positions = Vec::new();
        let mut copied = start;
        for (kind, span) in &tokens[range] {
            if *kind != TokenKind::Name {
                continue;
            }
            let Some(replacement) = placeholders.get(&printed[span.clone()]) else {
                continue;
            };
            text.push_str(&printed[copied..span.start]);
            text.push_str(replacement);
            group_positions.push(u32::try_from(text.len()).ok()?);
            copied = span.end;
        }
        text.push_str(&printed[copied..end]);
        Some(TemplateSection {
            text: text.into(),
            group_positions: group_positions.into(),
        })
    };
    // nothing shared should have had a placeholder
    if tokens[..grouped_start].iter().any(|(kind, span)| {
        *kind == TokenKind::Name && placeholders.contains_key(&printed[span.clone()])
    }) {
        return None;
    }
    Some(ContextAliasTemplate {
        max_groups,
        hash: hash_minified_query(document_str),
        head: printed[..tokens[grouped_start - 1].1.end].into(),
        variables: section(grouped_start..close_variables)?,
        fields: section(open_fields + 1..close_fields)?,
        fragments: section(close_fields + 1..tokens.len())?,
        group_variables: renamed_variables
            .into_iter()
            .map(|(name, _)| name)
            .collect(),
        response_keys: response_keys.into(),
        custom_scalar_paths: None,
    })
}

/// The index of the token that closes the bracket at `open`.
fn closing_token(
    printed: &str,
    tokens: &[(graphql_tools::parser::TokenKind, std::ops::Range<usize>)],
    open: usize,
) -> Option<usize> {
    let mut depth = 0usize;
    for (index, (kind, span)) in tokens.iter().enumerate().skip(open) {
        if *kind != graphql_tools::parser::TokenKind::Punctuator {
            continue;
        }
        match &printed[span.clone()] {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn renamed<'a>(names: &'a [(String, String)], name: &str) -> Option<&'a String> {
    names
        .iter()
        .find(|(original, _)| original == name)
        .map(|(_, new)| new)
}

/// Points the variables and fragment spreads in `selection` to new names. `variables` and
/// `fragments` pair every old name with its new one.
fn rename_selection(
    selection: &mut parser::Selection<'_, String>,
    variables: &[(String, String)],
    fragments: &[(String, String)],
) {
    let items = match selection {
        parser::Selection::Field(field) => {
            for (_, value) in &mut field.arguments {
                rename_value(value, variables);
            }
            rename_directives(&mut field.directives, variables);
            &mut field.selection_set.items
        }
        parser::Selection::FragmentSpread(spread) => {
            if let Some(name) = renamed(fragments, &spread.fragment_name) {
                spread.fragment_name = name.clone();
            }
            rename_directives(&mut spread.directives, variables);
            return;
        }
        parser::Selection::InlineFragment(fragment) => {
            rename_directives(&mut fragment.directives, variables);
            &mut fragment.selection_set.items
        }
    };
    for item in items {
        rename_selection(item, variables, fragments);
    }
}

fn rename_directives(
    directives: &mut [parser::Directive<'_, String>],
    variables: &[(String, String)],
) {
    for directive in directives {
        for (_, value) in &mut directive.arguments {
            rename_value(value, variables);
        }
    }
}

fn rename_value(value: &mut parser::Value<'_, String>, variables: &[(String, String)]) {
    match value {
        parser::Value::Variable(name) => {
            if let Some(new) = renamed(variables, name) {
                *name = new.clone();
            }
        }
        parser::Value::List(items) => {
            for item in items {
                rename_value(item, variables);
            }
        }
        parser::Value::Object(fields) => {
            for (_, item) in fields {
                rename_value(item, variables);
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
                context_template: None,
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

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

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
            operation.set_max_context_groups(
                usize::MAX,
                strings(&["representations", "__context_0"]),
                None,
            );
            assert!(operation.context_alias_template().is_none());
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
            let mut original = CustomScalarPaths::default();
            original.insert_path([key, "json"]);
            operation.set_max_context_groups(
                2,
                strings(&["representations", "c"]),
                Some(&original),
            );
            let template = operation.context_alias_template().unwrap();
            let paths = template.custom_scalar_paths().unwrap();
            // the group number is never read as part of the key
            assert_eq!(paths.child(key), None);
            // more groups than the limit, the paths don't depend on the count
            for group in [0, 1, 12] {
                let alias = format!("{key}_{group}");
                assert_eq!(paths.child(&alias), original.children.get(key));
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
        assert!(operation.context_alias_template().is_none());
        operation.set_max_context_groups(usize::MAX, strings(&["representations", "c"]), None);
        assert_eq!(operation.max_context_groups(), usize::MAX);

        let template = operation.context_alias_template().unwrap();
        let aliased = template.operation(2);
        insta::assert_snapshot!(aliased.document_str, @"query($shared:Int$representations_0:[_Any!]!$c_0:ID$representations_1:[_Any!]!$c_1:ID){_entities_0:_entities(representations:$representations_0){...F_0...on T@include(if:$c_0){g(a:[{x:$c_0}]b:$shared)}} _entities_1:_entities(representations:$representations_1){...F_1...on T@include(if:$c_1){g(a:[{x:$c_1}]b:$shared)}}}fragment F_0 on T{f(a:$c_0)}fragment F_1 on T{f(a:$c_1)}");
        assert_eq!(aliased.name_write_position, 5);
        // one hash per group count
        assert_eq!(aliased.hash, template.operation(2).hash);
        assert_ne!(aliased.hash, template.operation(3).hash);
        assert_ne!(aliased.hash, operation.hash);
        assert_eq!(template.variable("c", 1).as_deref(), Some("c_1"));
        assert_eq!(template.variable("shared", 1), None);
        assert_eq!(&*template.response_keys, ["_entities".to_string()]);
        assert_eq!(template.response_key("_entities_12"), Some((0, 12)));
        assert_eq!(template.response_key("_entities_"), None);
        assert_eq!(template.response_key("other_1"), None);
        // one template for every group count
        assert!(std::ptr::eq(
            template,
            operation.context_alias_template().unwrap()
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

    /// Writes `document_str` out for `groups` groups by copying its AST for every group, the slow
    /// and obvious way. A template has to come out the same.
    fn expand_by_copying(document_str: &str, variables: &[&str], groups: usize) -> String {
        use super::{parser, rename_directives, rename_selection};

        let document = parser::parse_query::<String>(document_str).unwrap();
        let mut query = None;
        let mut fragments = Vec::new();
        for definition in document.definitions {
            match definition {
                parser::Definition::Operation(parser::OperationDefinition::Query(operation)) => {
                    query = Some(operation)
                }
                parser::Definition::Fragment(fragment) => fragments.push(fragment),
                _ => panic!("only queries and fragments"),
            }
        }
        let mut query = query.unwrap();
        let (grouped, shared): (Vec<_>, Vec<_>) = std::mem::take(&mut query.variable_definitions)
            .into_iter()
            .partition(|definition| variables.contains(&definition.name.as_str()));
        let fields = std::mem::take(&mut query.selection_set.items);
        query.variable_definitions = shared;
        let mut copies = Vec::new();
        for group in 0..groups {
            let suffixed = |name: &str| (name.to_string(), format!("{name}_{group}"));
            let renamed_variables: Vec<_> = grouped.iter().map(|d| suffixed(&d.name)).collect();
            let renamed_fragments: Vec<_> = fragments.iter().map(|f| suffixed(&f.name)).collect();
            for definition in &grouped {
                query.variable_definitions.push(parser::VariableDefinition {
                    name: format!("{}_{group}", definition.name),
                    ..definition.clone()
                });
            }
            for field in &fields {
                let mut selection = field.clone();
                if let parser::Selection::Field(field) = &mut selection {
                    let key = field.alias.take().unwrap_or_else(|| field.name.clone());
                    field.alias = Some(format!("{key}_{group}"));
                }
                rename_selection(&mut selection, &renamed_variables, &renamed_fragments);
                query.selection_set.items.push(selection);
            }
            for fragment in &fragments {
                let mut fragment = fragment.clone();
                fragment.name = format!("{}_{group}", fragment.name);
                rename_directives(&mut fragment.directives, &renamed_variables);
                for selection in &mut fragment.selection_set.items {
                    rename_selection(selection, &renamed_variables, &renamed_fragments);
                }
                copies.push(parser::Definition::Fragment(fragment));
            }
        }
        let mut definitions = vec![parser::Definition::Operation(
            parser::OperationDefinition::Query(query),
        )];
        definitions.extend(copies);
        parser::minify_query_document(&parser::Document { definitions })
    }

    /// Checks that the template's output parses and matches [`expand_by_copying`], ignoring
    /// formatting, for group counts that cross digit boundaries. Returns the template's output.
    fn assert_template_matches_copying(document_str: &str, variables: &[&str]) -> Vec<String> {
        use super::parser;

        let mut operation = parse_subgraph_fetch_operation(document_str).operation;
        operation.set_max_context_groups(usize::MAX, strings(variables), None);
        let template = operation
            .context_alias_template()
            .unwrap_or_else(|| panic!("no template for {document_str}"));
        [1, 2, 3, 9, 10, 11, 99, 100, 101]
            .into_iter()
            .map(|groups| {
                let written = template.operation(groups).document_str.into_string();
                let parsed = parser::parse_query::<String>(&written).unwrap_or_else(|error| {
                    panic!("{groups} groups don't parse: {error}\n{written}")
                });
                assert_eq!(
                    parser::minify_query_document(&parsed),
                    expand_by_copying(&operation.document_str, variables, groups),
                    "{groups} groups of {document_str}"
                );
                written
            })
            .collect()
    }

    /// Every string argument value in `document`, decoded.
    fn string_arguments(document: &str) -> Vec<String> {
        use super::parser;

        fn collect(selection: &parser::Selection<'_, String>, strings: &mut Vec<String>) {
            let items = match selection {
                parser::Selection::Field(field) => {
                    for (_, value) in &field.arguments {
                        if let parser::Value::String(string) = value {
                            strings.push(string.clone());
                        }
                    }
                    &field.selection_set.items
                }
                parser::Selection::InlineFragment(fragment) => &fragment.selection_set.items,
                parser::Selection::FragmentSpread(_) => return,
            };
            for item in items {
                collect(item, strings);
            }
        }
        let mut strings = Vec::new();
        for definition in parser::parse_query::<String>(document).unwrap().definitions {
            let selections = match definition {
                parser::Definition::Operation(parser::OperationDefinition::Query(query)) => {
                    query.selection_set.items
                }
                parser::Definition::Fragment(fragment) => fragment.selection_set.items,
                _ => continue,
            };
            for selection in &selections {
                collect(selection, &mut strings);
            }
        }
        strings
    }

    #[test]
    fn context_alias_template_keeps_control_characters_in_strings() {
        // a newline makes the minifier write a block string, with the control character raw.
        // these used to be the group placeholder and the section markers.
        for character in ['\0', '\u{1}', '\u{2}', '\u{3}'] {
            let value = format!("x{character}y\nz");
            let escaped = format!("\"x\\u000{}y\\nz\"", character as u32);
            let written = assert_template_matches_copying(
                &format!(
                    "query($r: [_Any!]!) {{ _entities(representations: $r) {{ ... on T {{ f(s: {escaped}) }} }} }}"
                ),
                &["r"],
            );
            for written in &written {
                let strings = string_arguments(written);
                assert!(!strings.is_empty());
                assert!(strings.iter().all(|string| *string == value), "{written:?}");
            }
        }
    }

    #[test]
    fn context_alias_template_leaves_names_in_strings_alone() {
        let written = assert_template_matches_copying(
            r#"query($r: [_Any!]!, $c: ID) { _entities(representations: $r) { ...F ... on T { f(a: $c, s: "$c __hive_tpl_0 _entities_0 ...F", t: "$r\n__hive_tpl_1:") } } } fragment F on T { g(s: "fragment F_0 on T __hive_tpl_2") }"#,
            &["r", "c"],
        );
        let strings = string_arguments(&written[1]);
        assert_eq!(
            strings,
            [
                "$c __hive_tpl_0 _entities_0 ...F",
                "$r\n__hive_tpl_1:",
                "$c __hive_tpl_0 _entities_0 ...F",
                "$r\n__hive_tpl_1:",
                "fragment F_0 on T __hive_tpl_2",
                "fragment F_0 on T __hive_tpl_2",
            ]
        );
    }

    #[test]
    fn context_alias_template_avoids_names_that_look_like_placeholders() {
        // a field, an argument, an enum value, a type and a shared variable all start like the
        // first placeholder candidate
        assert_template_matches_copying(
            "query($r: [_Any!]!, $__hive_tpl_3: __hive_tpl_T) { _entities(representations: $r) { ... on T { __hive_tpl_0(__hive_tpl_1: __hive_tpl_2, x: $__hive_tpl_3) __hive_tpl_: a __hive_tpl__0: b } } }",
            &["r"],
        );
    }

    #[test]
    fn context_alias_template_handles_long_names_that_look_like_placeholders() {
        // this used to grow the placeholder prefix one `_` at a time, comparing it to every name
        let alias = format!("__hive_tpl_{}", "_".repeat(200_000));
        let query = format!(
            "query($r: [_Any!]!) {{ _entities(representations: $r) {{ ... on T {{ {alias}: x __hive_tpl_0 }} }} }}"
        );
        let mut operation = parse_subgraph_fetch_operation(&query).operation;
        operation.set_max_context_groups(usize::MAX, strings(&["r"]), None);
        let template = operation.context_alias_template().unwrap();
        // the placeholders stayed short, so the template is no bigger than the operation
        let size = template.head.len()
            + [&template.variables, &template.fields, &template.fragments]
                .iter()
                .map(|section| section.text.len())
                .sum::<usize>();
        assert!(size < operation.document_str.len() + 100);
        let written = template.operation(2).document_str;
        assert_eq!(
            super::parser::minify_query_document(
                &super::parser::parse_query::<String>(&written).unwrap()
            ),
            expand_by_copying(&operation.document_str, &["r"], 2)
        );
    }

    #[test]
    fn context_alias_template_handles_defaults_directives_and_fragments() {
        assert_template_matches_copying(
            r#"query($representations: [_Any!]!, $c: In = {a: [1, {b: "x)", c: [[]]}]}, $shared: In = {x: ["(", {y: 2}]}) { _entities(representations: $representations) { ...F @include(if: $c) ... on T @skip(if: $c) { g(a: [{x: $c, y: [$c, {z: $shared}]}], b: $shared) @dir(a: {b: [$c]}) } } } fragment F on T @dir(a: $c) { f(a: $c) ...G } fragment G on T { h(b: $c, d: $shared) }"#,
            &["representations", "c"],
        );
    }

    #[test]
    fn context_alias_template_handles_several_top_level_fields_and_aliases() {
        assert_template_matches_copying(
            "query($r: [_Any!]!, $s: [_Any!]!) { a: _entities(representations: $r) { ... on T { x } } _entities(representations: $s) { __typename } b: __typename }",
            &["r", "s"],
        );
    }

    #[test]
    fn context_alias_template_falls_back_when_it_cant_be_compiled() {
        for (query, variables) in [
            // a top-level fragment spread has no response key to alias
            ("query($r: [_Any!]!) { ...F } fragment F on Query { _entities(representations: $r) { __typename } }", &["r"][..]),
            // nothing gets its own copy
            ("query($r: [_Any!]!) { _entities(representations: $r) { __typename } }", &["other"][..]),
        ] {
            let mut operation = parse_subgraph_fetch_operation(query).operation;
            operation.set_max_context_groups(15, strings(variables), None);
            assert!(operation.context_alias_template().is_none(), "{query}");
            // so every group goes in a request of its own
            assert_eq!(operation.max_context_groups(), 1, "{query}");
        }
        let mut operation = parse_subgraph_fetch_operation(
            "query($r: [_Any!]!) { _entities(representations: $r) { __typename } }",
        )
        .operation;
        operation.set_max_context_groups(1, strings(&["r"]), None);
        assert!(operation.context_alias_template().is_none());
        operation.set_max_context_groups(15, strings(&["r"]), None);
        assert_eq!(operation.max_context_groups(), 15);
    }

    #[test]
    fn context_alias_template_separates_fields_that_end_with_a_name() {
        let mut operation =
            parse_subgraph_fetch_operation("query($r: [_Any!]!) { a(x: $r) b }").operation;
        operation.set_max_context_groups(usize::MAX, strings(&["r"]), None);
        let template = operation.context_alias_template().unwrap();
        insta::assert_snapshot!(template.operation(2).document_str, @"query($r_0:[_Any!]!$r_1:[_Any!]!){a_0:a(x:$r_0)b_0:b a_1:a(x:$r_1)b_1:b}");
    }
}
