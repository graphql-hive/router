//! How a query plan prints. Three blocks: the variables it uses, every fetch with what it waits
//! for, then each fetch with what it sends and where its data goes.
//!
//! ```text
//! variables
//!   $first: Int, $withStock
//!
//! fetches
//!   fetch               after
//!   [3] products        -
//!   [4] inventory       [3]
//!
//! [3] products
//!   out
//!     { topProducts(first: $first) { __typename upc name } }
//!   vars
//!     $first
//!
//! [4] inventory   after [3]
//!   if
//!     $withStock
//!   merge
//!     topProducts.@
//!   in
//!     ... on Product { __typename upc }
//!   out
//!     ... on Product { inStock }
//! ```
//!
//! Fetches are listed so each one comes after everything it waits for. The order comes from the
//! plan's schedule, the same thing the executor runs on, not from `Sequence`/`Parallel`.

use std::collections::BTreeSet;
use std::fmt::{Display, Formatter, Result as FmtResult, Write};

use graphql_tools::parser::query as parser;

use super::{
    display_run_if, BatchFetchNode, ConditionGroup, FetchContext, FetchNode, FetchNodePathSegment,
    FetchRewrite, FlattenNodePath, PlanNode, PlanState, QueryPlan,
};
use crate::query_planner::{
    ast::{
        fragment::FragmentDefinition, operation::VariableDefinition, selection_item::SelectionItem,
        selection_set::SelectionSet,
    },
    planner::plan_nodes::dependencies::plan_fetches,
    state::supergraph_state::OperationKind,
    utils::pretty_display::PrettyDisplay,
};

/// A line longer than this gets its selection set pretty-printed instead.
const WIDTH: usize = 100;
/// How much deeper what's under a heading goes.
const STEP: &str = "  ";

impl<S: PlanState> Display for QueryPlan<S> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let fetches = self.fetches();
        if fetches.is_empty() {
            return row(f, "", "fetches", "none");
        }

        let variables = plan_variables(&fetches);
        if !variables.is_empty() {
            // one line when it fits, one variable per line otherwise
            let line = variables.join(", ");
            let lines = match STEP.len() + line.len() <= WIDTH {
                true => line,
                false => variables.join("\n"),
            };
            row(f, "", "variables", &lines)?;
            writeln!(f)?;
        }

        let name_width = fetches.iter().map(|fetch| fetch.name().len() + 2).max();
        let name_width = name_width.unwrap_or(0).max(20);
        let mut overview = vec![format!("{:name_width$}after", "fetch")];
        for fetch in &fetches {
            overview.push(format!(
                "{:name_width$}{}",
                fetch.name(),
                fetch.after_list()
            ));
        }
        row(f, "", "fetches", &overview.join("\n"))?;

        for fetch in &fetches {
            writeln!(f)?;
            fetch.fmt(f)?;
        }
        Ok(())
    }
}

/// A fetch of the plan, with where its data goes and what it waits for.
struct Fetch<'a, S: PlanState> {
    kind: FetchKind<'a, S>,
    merge: Option<&'a FlattenNodePath>,
    subscription: bool,
    after: Vec<super::FetchId>,
    operation: Operation,
}

enum FetchKind<'a, S: PlanState> {
    Single(&'a FetchNode<S>),
    Batch(&'a BatchFetchNode<S>),
}

impl<S: PlanState> QueryPlan<S> {
    /// The plan's fetches, each after everything it waits for. A fetch is listed as soon as it
    /// can start, ties keep the plan's order.
    fn fetches(&self) -> Vec<Fetch<'_, S>> {
        let Some(node) = self.node() else {
            return Vec::new();
        };
        // by position, the schedule counts in the order `plan_fetches` lists them
        let nodes = plan_fetches(node);
        let mut waits_for: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
        for index in 0..self.schedule().fetch_count().min(nodes.len()) {
            for &dependent in self.schedule().dependents(index) {
                waits_for[dependent as usize].push(index);
            }
        }
        // how many fetches have to run one after another before this one can start
        let mut depth = vec![0; nodes.len()];
        for index in 0..nodes.len() {
            depth[index] = waits_for[index]
                .iter()
                .map(|&wait| depth[wait] + 1)
                .max()
                .unwrap_or(0);
        }
        let mut order: Vec<usize> = (0..nodes.len()).collect();
        order.sort_by_key(|&index| depth[index]);

        let id = |index: usize| match unwrap(nodes[index], None) {
            Some((FetchKind::Single(fetch), ..)) => Some(fetch.id),
            Some((FetchKind::Batch(batch), ..)) => Some(batch.id),
            None => None,
        };
        order
            .into_iter()
            .filter_map(|index| {
                let (kind, merge, subscription) = unwrap(nodes[index], None)?;
                let after = waits_for[index]
                    .iter()
                    .filter_map(|&wait| id(wait))
                    .collect();
                let document = match &kind {
                    FetchKind::Single(fetch) => &fetch.operation.as_ref().document_str,
                    FetchKind::Batch(batch) => &batch.operation.as_ref().document_str,
                };
                Some(Fetch {
                    operation: Operation::parse(document),
                    kind,
                    merge,
                    subscription,
                    after,
                })
            })
            .collect()
    }
}

/// The fetch inside a node, where its data is merged, and whether it's a subscription.
/// `None` for a node that isn't a fetch.
#[allow(clippy::type_complexity)]
fn unwrap<'a, S: PlanState>(
    node: &'a PlanNode<S>,
    merge: Option<&'a FlattenNodePath>,
) -> Option<(FetchKind<'a, S>, Option<&'a FlattenNodePath>, bool)> {
    match node {
        PlanNode::Fetch(fetch) => Some((FetchKind::Single(fetch), merge, false)),
        PlanNode::BatchFetch(batch) => Some((FetchKind::Batch(batch), merge, false)),
        PlanNode::Flatten(flatten) => unwrap(&flatten.node, Some(&flatten.path)),
        PlanNode::Subscription(subscription) => {
            Some((FetchKind::Single(&subscription.primary), merge, true))
        }
        _ => None,
    }
}

impl<S: PlanState> Fetch<'_, S> {
    fn name(&self) -> String {
        let (id, service) = match &self.kind {
            FetchKind::Single(fetch) => (fetch.id, &fetch.service_name),
            FetchKind::Batch(batch) => (batch.id, &batch.service_name),
        };
        let mut name = format!("[{id}] {service}");
        if self.subscription {
            name.push_str(" subscription");
        }
        name
    }

    fn after_list(&self) -> String {
        if self.after.is_empty() {
            return "-".to_string();
        }
        let ids: Vec<String> = self.after.iter().map(|id| format!("[{id}]")).collect();
        ids.join(", ")
    }

    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.name())?;
        if !self.after.is_empty() {
            write!(f, "   after {}", self.after_list())?;
        }
        writeln!(f)?;

        let indent = STEP;
        match &self.kind {
            FetchKind::Single(fetch) => {
                let operation = &self.operation;
                if let Some(run_if) = &fetch.run_if {
                    row(f, indent, "if", &display_run_if(run_if))?;
                }
                if let Some(merge) = self.merge {
                    row(f, indent, "merge", &merge.to_string())?;
                }
                if let Some(requires) = &fetch.requires {
                    let value = selections(requires, false, indent.len() + STEP.len());
                    row(f, indent, "in", &value)?;
                }
                let out = operation.out(None, indent.len() + STEP.len());
                row(f, indent, "out", &out)?;
                let vars = sent_variables(&fetch.variable_usages, &[]);
                if !vars.is_empty() {
                    row(f, indent, "vars", &dollars(&vars))?;
                }
                rewrites(f, indent, &fetch.input_rewrites, &fetch.output_rewrites)?;
                context(f, indent, &fetch.context)?;
                if fetch.operation_kind == Some(OperationKind::Mutation) {
                    writeln!(f, "{indent}mutation")?;
                }
            }
            FetchKind::Batch(batch) => {
                let operation = &self.operation;
                let alias_indent = &format!("{indent}{STEP}");
                let value_start = alias_indent.len() + STEP.len();
                for alias in &batch.entity_batch.aliases {
                    writeln!(f, "{indent}alias {}", alias.alias)?;
                    if let Some(run_if) = &alias.run_if {
                        row(f, alias_indent, "if", &display_run_if(run_if))?;
                    }
                    let paths: Vec<String> = alias
                        .merge_paths
                        .iter()
                        .map(|path| path.to_string())
                        .collect();
                    row(f, alias_indent, "merge", &paths.join("\n"))?;
                    row(
                        f,
                        alias_indent,
                        "in",
                        &selections(&alias.requires, false, value_start),
                    )?;
                    row(
                        f,
                        alias_indent,
                        "out",
                        &operation.alias_out(&alias.alias, value_start),
                    )?;
                    rewrites(
                        f,
                        alias_indent,
                        &alias.input_rewrites,
                        &alias.output_rewrites,
                    )?;
                    context(f, alias_indent, &alias.context)?;
                }
                let fragments = operation.fragments(indent.len() + STEP.len());
                if !fragments.is_empty() {
                    row(f, indent, "fragments", &fragments.join("\n"))?;
                }
                let reps: Vec<&str> = batch
                    .entity_batch
                    .aliases
                    .iter()
                    .map(|alias| alias.representations_variable_name.as_str())
                    .collect();
                let vars = sent_variables(&batch.variable_usages, &reps);
                if !vars.is_empty() {
                    row(f, indent, "vars", &dollars(&vars))?;
                }
            }
        }
        Ok(())
    }
}

/// `key` as a heading, with each line of `value` under it, one step deeper.
fn row(f: &mut Formatter<'_>, indent: &str, key: &str, value: &str) -> FmtResult {
    writeln!(f, "{indent}{key}")?;
    for line in value.lines() {
        writeln!(f, "{indent}{STEP}{line}")?;
    }
    Ok(())
}

/// A selection set starting at column `start`. On one line when it fits, otherwise its items go
/// on their own lines, and each of them gets the same treatment, so only the levels that are too
/// wide are broken up. `braces` wraps it in `{ }`, for a root selection set.
fn selections(set: &impl PrettyDisplay, braces: bool, start: usize) -> String {
    // read the pretty-printed text back as a tree, one item per line, `x {` opens a block
    let pretty = Pretty(set, 0).to_string();
    let mut items = parse_items(&mut pretty.lines());
    if braces {
        items = vec![Item::Block(String::new(), items)];
    }
    let flat: Vec<String> = items.iter().map(Item::one_line).collect();
    let flat = flat.join(" ");
    if start + flat.len() <= WIDTH {
        return flat;
    }
    let mut lines = Vec::new();
    for item in &items {
        item.lines(start, 0, &mut lines);
    }
    lines.join("\n")
}

/// A selection: a field or fragment spread on its own, or something with a selection set.
enum Item {
    Leaf(String),
    /// What comes before ` {`, empty for a bare `{ }`.
    Block(String, Vec<Item>),
}

fn parse_items<'a>(lines: &mut impl Iterator<Item = &'a str>) -> Vec<Item> {
    let mut items = Vec::new();
    while let Some(line) = lines.next() {
        let line = line.trim();
        if line == "}" {
            break;
        }
        match line.strip_suffix('{') {
            Some(head) => items.push(Item::Block(head.trim_end().to_string(), parse_items(lines))),
            None => items.push(Item::Leaf(line.to_string())),
        }
    }
    items
}

impl Item {
    fn one_line(&self) -> String {
        match self {
            Item::Leaf(text) => text.clone(),
            Item::Block(head, items) => {
                let items: Vec<String> = items.iter().map(Item::one_line).collect();
                let open = if head.is_empty() {
                    "{".to_string()
                } else {
                    format!("{head} {{")
                };
                format!("{open} {} }}", items.join(" "))
            }
        }
    }

    /// Pushes this item's lines, indented by `depth` levels, the first one starting at `start`.
    fn lines(&self, start: usize, depth: usize, out: &mut Vec<String>) {
        let pad = "  ".repeat(depth);
        let flat = self.one_line();
        let Item::Block(head, items) = self else {
            out.push(pad + &flat);
            return;
        };
        if start + pad.len() + flat.len() <= WIDTH {
            out.push(pad + &flat);
            return;
        }
        out.push(match head.is_empty() {
            true => format!("{pad}{{"),
            false => format!("{pad}{head} {{"),
        });
        for item in items {
            item.lines(start, depth + 1, out);
        }
        out.push(pad + "}");
    }
}

struct Pretty<'a, T: PrettyDisplay>(&'a T, usize);

impl<T: PrettyDisplay> Display for Pretty<'_, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        self.0.pretty_fmt(f, self.1)
    }
}

/// A subgraph operation read back from the text the plan sends, so planning and executable
/// plans print the same.
enum Operation {
    Parsed {
        selection_set: SelectionSet,
        fragments: Vec<FragmentDefinition>,
        variables: Vec<VariableDefinition>,
    },
    /// Never expected, the planner wrote it. Then it's shown as it is.
    Raw(String),
}

impl Operation {
    fn parse(document: &str) -> Self {
        let Ok(parsed) = parser::parse_query::<String>(document) else {
            return Self::Raw(document.to_string());
        };
        let mut selection_set = None;
        let mut fragments = Vec::new();
        let mut variables = Vec::new();
        for definition in parsed.definitions {
            match definition {
                parser::Definition::Operation(operation) => {
                    let (set, definitions) = match operation {
                        parser::OperationDefinition::SelectionSet(set) => (set, Vec::new()),
                        parser::OperationDefinition::Query(query) => {
                            (query.selection_set, query.variable_definitions)
                        }
                        parser::OperationDefinition::Mutation(mutation) => {
                            (mutation.selection_set, mutation.variable_definitions)
                        }
                        parser::OperationDefinition::Subscription(subscription) => (
                            subscription.selection_set,
                            subscription.variable_definitions,
                        ),
                    };
                    selection_set = Some(set);
                    variables = definitions.into_iter().map(Into::into).collect();
                }
                parser::Definition::Fragment(fragment) => fragments.push(fragment.into()),
            }
        }
        match selection_set {
            Some(set) => Self::Parsed {
                selection_set: set.into(),
                fragments,
                variables,
            },
            None => Self::Raw(document.to_string()),
        }
    }

    /// What's asked for: inside `_entities` for an entity fetch, the whole selection set for a
    /// root one. Fragments follow on their own lines.
    fn out(&self, alias: Option<&str>, start: usize) -> String {
        let Self::Parsed { selection_set, .. } = self else {
            return self.raw();
        };
        let entities = selection_set.items.iter().find_map(|item| match item {
            SelectionItem::Field(field)
                if field.name == "_entities" && field.alias.as_deref() == alias =>
            {
                Some(&field.selections)
            }
            _ => None,
        });
        let mut out = match (entities, alias) {
            (Some(entities), _) => selections(entities, false, start),
            (None, None) => selections(selection_set, true, start),
            // a batch alias that isn't in the operation, never expected
            (None, Some(_)) => return self.raw(),
        };
        if alias.is_none() {
            for fragment in self.fragments(start) {
                out.push('\n');
                out.push_str(&fragment);
            }
        }
        out
    }

    fn alias_out(&self, alias: &str, start: usize) -> String {
        self.out(Some(alias), start)
    }

    /// Each fragment, starting at column `start`.
    fn fragments(&self, start: usize) -> Vec<String> {
        let Self::Parsed { fragments, .. } = self else {
            return Vec::new();
        };
        fragments
            .iter()
            .map(|fragment| {
                let head = format!("fragment {} on {} ", fragment.name, fragment.type_condition);
                let set = selections(&fragment.selection_set, true, start + head.len());
                head + &set
            })
            .collect()
    }

    /// `$name: Type = default`, as this operation declares it.
    fn declaration(&self, name: &str) -> Option<String> {
        let Self::Parsed { variables, .. } = self else {
            return None;
        };
        let variable = variables.iter().find(|variable| variable.name == name)?;
        Some(match &variable.default_value {
            Some(default) => format!("${name}: {} = {default}", variable.variable_type),
            None => format!("${name}: {}", variable.variable_type),
        })
    }

    fn raw(&self) -> String {
        match self {
            Self::Raw(document) => document.clone(),
            Self::Parsed { .. } => String::new(),
        }
    }
}

/// The variables a fetch sends, without the representations.
fn sent_variables<'a>(usages: &'a Option<BTreeSet<String>>, reps: &[&str]) -> Vec<&'a str> {
    usages
        .iter()
        .flatten()
        .map(String::as_str)
        .filter(|name| *name != "representations" && !reps.contains(name))
        .collect()
}

/// The client's variables the plan reads: the ones fetches send and the ones their conditions
/// check. The plan doesn't keep the client's declarations, so a type comes from a subgraph
/// operation that declares the variable, they copy the client's. A variable only a fetch's `if`
/// checks is never sent, so it has no type here.
fn plan_variables<S: PlanState>(fetches: &[Fetch<'_, S>]) -> Vec<String> {
    let mut names = BTreeSet::new();
    fn add(
        names: &mut BTreeSet<String>,
        usages: &Option<BTreeSet<String>>,
        run_if: &Option<Vec<ConditionGroup>>,
        context: &Option<Box<FetchContext>>,
    ) {
        // context variables are the router's own, filled from the response
        let own = FetchContext::group_variables(context);
        for name in usages.iter().flatten() {
            if name != "representations" && !own.contains(&name.as_str()) {
                names.insert(name.clone());
            }
        }
        for group in run_if.iter().flatten() {
            names.extend(group.include_if.iter().cloned());
            names.extend(group.skip_if.iter().cloned());
        }
    }
    for fetch in fetches {
        match &fetch.kind {
            FetchKind::Single(fetch) => add(
                &mut names,
                &fetch.variable_usages,
                &fetch.run_if,
                &fetch.context,
            ),
            FetchKind::Batch(batch) => {
                for alias in &batch.entity_batch.aliases {
                    add(&mut names, &None, &alias.run_if, &alias.context);
                }
                let reps: Vec<&str> = batch
                    .entity_batch
                    .aliases
                    .iter()
                    .map(|alias| alias.representations_variable_name.as_str())
                    .collect();
                let own: Vec<&str> = batch
                    .entity_batch
                    .aliases
                    .iter()
                    .flat_map(|alias| FetchContext::group_variables(&alias.context))
                    .collect();
                for name in sent_variables(&batch.variable_usages, &reps) {
                    if !own.contains(&name) {
                        names.insert(name.to_string());
                    }
                }
            }
        }
    }
    names
        .into_iter()
        .map(|name| {
            fetches
                .iter()
                .find_map(|fetch| fetch.operation.declaration(&name))
                .unwrap_or_else(|| format!("${name}"))
        })
        .collect()
}

fn dollars(names: &[impl AsRef<str>]) -> String {
    let names: Vec<String> = names
        .iter()
        .map(|name| format!("${}", name.as_ref()))
        .collect();
    names.join(", ")
}

fn rewrites(
    f: &mut Formatter<'_>,
    indent: &str,
    input: &Option<Vec<FetchRewrite>>,
    output: &Option<Vec<FetchRewrite>>,
) -> FmtResult {
    for (key, rewrites) in [("rewrite in", input), ("rewrite out", output)] {
        if let Some(rewrites) = rewrites.as_ref().filter(|rewrites| !rewrites.is_empty()) {
            let lines: Vec<String> = rewrites.iter().map(rewrite).collect();
            row(f, indent, key, &lines.join("\n"))?;
        }
    }
    Ok(())
}

/// `path = "value"` to set a value, `path -> name` to rename a key.
fn rewrite(rewrite: &FetchRewrite) -> String {
    match rewrite {
        FetchRewrite::ValueSetter(setter) => {
            format!("{} = \"{}\"", path(&setter.path), setter.set_value_to)
        }
        FetchRewrite::KeyRenamer(renamer) => {
            format!("{} -> {}", path(&renamer.path), renamer.rename_key_to)
        }
    }
}

/// Where values for `@fromContext` arguments come from: `$variable = path`, the path starting
/// at the entity, `..` a level up.
fn context(f: &mut Formatter<'_>, indent: &str, context: &Option<Box<FetchContext>>) -> FmtResult {
    let Some(context) = context else {
        return Ok(());
    };
    let rewrites = context.rewrites.as_deref().unwrap_or_default();
    let mut lines: Vec<String> = rewrites
        .iter()
        .map(|renamer| format!("${} = {}", renamer.rename_key_to, path(&renamer.path)))
        .collect();
    for variable in &context.variables {
        if !rewrites
            .iter()
            .any(|renamer| &renamer.rename_key_to == variable)
        {
            lines.push(format!("${variable}"));
        }
    }
    if !lines.is_empty() {
        row(f, indent, "context", &lines.join("\n"))?;
    }
    Ok(())
}

/// A rewrite path, relative to the object it starts at: `/` between levels, `..` a level up,
/// `|[A]` only when the object is an `A`, like `../..|[Outer]/code` or `[Book]/__typename`.
fn path(segments: &[FetchNodePathSegment]) -> String {
    let mut out = String::new();
    for segment in segments {
        match segment {
            FetchNodePathSegment::TypenameEquals(names) => {
                let names: Vec<&str> = names.iter().map(String::as_str).collect();
                // the type check belongs to the level before it, if there's one
                let bar = if out.is_empty() { "" } else { "|" };
                let _ = write!(out, "{bar}[{}]", names.join("|"));
            }
            FetchNodePathSegment::Key(key) => push_level(&mut out, key),
            FetchNodePathSegment::Parent => push_level(&mut out, ".."),
        }
    }
    out
}

fn push_level(out: &mut String, level: &str) {
    if !out.is_empty() {
        out.push('/');
    }
    out.push_str(level);
}
