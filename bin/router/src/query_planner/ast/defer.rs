//! Gets an operation with `@defer` ready for the planner.
//!
//! Each `@defer` gets a [`DeferId`] on its marker, and the planner carries it the way
//! it carries `@skip`/`@include`, but on its own: from the fragment through the query tree to
//! every fetch it plans for it, never mixed with the client's conditions. Fetches of different
//! `@defer`s are never merged or batched, so each fetch belongs to exactly one part.
//! A deferred fragment on an entity goes back into its subgraph through the type's own key, so
//! its fields get an entity call of their own even when the primary part reads the same
//! subgraph. Without a usable key the fields stay in the parent's fetch, and only their delivery
//! waits.
//!
//! One plan covers every value of `@defer(if: $var)`. The fragment is planned twice: once
//! without `@defer`, under `@skip(if: $_d0)`, for when it isn't deferred, and once deferred,
//! under `@include(if: $_d0)`. `$_d0` is a `Boolean!` the router sets from `$var` before running
//! the plan, `true` when `$var` isn't sent, the way `if` defaults. So the fetches of the copy that
//! isn't wanted are skipped, the same way `@skip`/`@include` skip any fetch.

use std::collections::HashMap;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use super::operation::{OperationDefinition, VariableDefinition};
use super::selection_item::SelectionItem;
use super::selection_set::{DeferMarker, InlineFragmentSelection, SelectionSet};
use crate::query_planner::state::supergraph_state::{OperationKind, SupergraphState, TypeNode};

/// One deferred part of an operation: its index in [`DeferLayout::parts`]. Stored one up,
/// so `Option<DeferId>` takes no more room than the id: fetches and path segments carry one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(into = "usize", try_from = "usize")]
pub struct DeferId(NonZeroU32);

impl DeferId {
    pub fn from_index(index: usize) -> Self {
        // an operation that big doesn't get past the request body limit
        Self::try_from(index).expect("fewer than 2^32 deferred parts")
    }

    pub fn index(self) -> usize {
        self.0.get() as usize - 1
    }
}

impl TryFrom<usize> for DeferId {
    type Error = &'static str;

    fn try_from(index: usize) -> Result<Self, Self::Error> {
        u32::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .and_then(NonZeroU32::new)
            .map(Self)
            .ok_or("a deferred part index has to fit in a u32")
    }
}

impl From<DeferId> for usize {
    fn from(id: DeferId) -> Self {
        id.index()
    }
}

impl std::fmt::Display for DeferId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.index())
    }
}

/// The value of a `Boolean` variable, `None` when it isn't sent or isn't a boolean.
pub fn variable_bool(
    variables: &Option<HashMap<String, sonic_rs::Value>>,
    name: &str,
) -> Option<bool> {
    use sonic_rs::JsonValueTrait;
    variables.as_ref()?.get(name)?.as_bool()
}

impl InlineFragmentSelection {
    /// The deferred part the fragment starts, on the operation the planner gets.
    pub fn defer_id(&self) -> Option<DeferId> {
        self.defer.as_ref().and_then(|marker| marker.id)
    }
}

impl DeferMarker {
    /// Whether the fragment is deferred with these variables. `if` defaults to `true`, so a
    /// variable that isn't sent leaves the defer on.
    pub fn is_active(&self, variables: &Option<HashMap<String, sonic_rs::Value>>) -> bool {
        self.if_var
            .as_ref()
            .is_none_or(|var| variable_bool(variables, var).unwrap_or(true))
    }
}

pub struct DeferredOperation {
    /// The operation the planner gets, see [`prepare_for_planning`].
    pub operation: OperationDefinition,
    pub layout: DeferLayout,
}

/// The deferred parts of a plan, for scheduling it, running it and showing it. A part with an
/// `if` is there whatever its value, the router's `if_variables` decide at run time whether its
/// fetches run.
#[derive(Debug, Default)]
pub struct DeferLayout {
    /// The deferred parts, by [`DeferId`], in document order.
    pub parts: Vec<DeferPart>,
    /// Text for showing the plan, running it never reads it.
    pub display: DeferDisplay,
    /// The variables the router sets before running the plan, `(router's, client's)`: the
    /// router's is the client's `@defer(if:)` value, `true` when it isn't sent.
    pub if_variables: Vec<(String, String)>,
}

/// A `@defer` at one place in the response. A conditional `@defer` nested in another one is
/// two parts, one for each copy of the outer fragment.
#[derive(Clone, Debug)]
pub struct DeferPart {
    pub label: Option<String>,
    /// The deferred part this one is nested in.
    pub parent: Option<DeferId>,
    /// From the root to the object the fragment is on.
    pub path: Vec<PartPathSegment>,
    /// Where the `@defer` is in the document, its `DeferMarker::position`.
    pub position: (usize, usize),
}

/// What each part delivers, as the `subselection` text of the `Defer` nodes in an exposed
/// plan. Only [`defer_display_plan`] reads it, running the plan never does.
///
/// [`defer_display_plan`]: crate::query_planner::planner::plan_nodes::defer::defer_display_plan
#[derive(Debug, Default)]
pub struct DeferDisplay {
    /// The primary part's fields. `None` when everything is deferred.
    pub primary: Option<String>,
    /// Each deferred part's fields, by [`DeferId`], without the parts nested in it.
    pub parts: Vec<Option<String>>,
}

/// One step of a [`DeferPart`]'s path: a response key, or any item of a list.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PartPathSegment {
    Key(String),
    AnyIndex,
}

impl std::fmt::Display for PartPathSegment {
    /// The way `Flatten` paths read, `@` for a list.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PartPathSegment::Key(key) => f.write_str(key),
            PartPathSegment::AnyIndex => f.write_str("@"),
        }
    }
}

/// Where a deferred fragment is: the part it's in, its path and its `@defer`'s position.
type PartKey = (Option<DeferId>, Vec<PartPathSegment>, (usize, usize));

struct Rewrite<'a> {
    supergraph: &'a SupergraphState,
    parts: Vec<DeferPart>,
    /// What each part delivers, from every copy of its fragment.
    delivers: Vec<SelectionSet>,
    /// Copies of one fragment, made by normalization, are one part.
    ids: HashMap<PartKey, DeferId>,
    path: Vec<PartPathSegment>,
    /// Whether to collect `delivers`, see [`prepare_for_planning`].
    display: bool,
}

/// Numbers the `@defer`s in document order, `(0, 0)`, `(1, 0)`, ..., in place of their line and
/// column. Copies of one keep sharing a number. Positions go into the operation's hash, so the
/// same operation formatted differently now gets the same hash and plan.
pub fn number_defers(operation: &mut OperationDefinition) {
    fn collect(set: &SelectionSet, out: &mut Vec<(usize, usize)>) {
        for item in &set.items {
            match item {
                SelectionItem::Field(field) => collect(&field.selections, out),
                SelectionItem::InlineFragment(fragment) => {
                    if let Some(marker) = &fragment.defer {
                        out.push(marker.position);
                    }
                    collect(&fragment.selections, out);
                }
                SelectionItem::FragmentSpread(_) => {}
            }
        }
    }
    fn renumber(set: &mut SelectionSet, sorted: &[(usize, usize)]) {
        for item in &mut set.items {
            match item {
                SelectionItem::Field(field) => renumber(&mut field.selections, sorted),
                SelectionItem::InlineFragment(fragment) => {
                    if let Some(marker) = &mut fragment.defer {
                        let index = sorted
                            .binary_search(&marker.position)
                            .expect("`sorted` has every position");
                        marker.position = (index, 0);
                    }
                    renumber(&mut fragment.selections, sorted);
                }
                SelectionItem::FragmentSpread(_) => {}
            }
        }
    }
    let mut positions = Vec::new();
    collect(&operation.selection_set, &mut positions);
    if positions.is_empty() {
        return;
    }
    positions.sort_unstable();
    positions.dedup();
    renumber(&mut operation.selection_set, &positions);
}

/// Whether there's any `@defer` in the operation.
pub fn has_defer(set: &SelectionSet) -> bool {
    set.items.iter().any(|item| match item {
        SelectionItem::Field(field) => has_defer(&field.selections),
        SelectionItem::InlineFragment(fragment) => {
            fragment.defer.is_some() || has_defer(&fragment.selections)
        }
        SelectionItem::FragmentSpread(_) => false,
    })
}

/// The operation the planner gets: every `@defer` has its [`DeferId`], and each
/// `@defer(if: $var)` is split in two, see the module docs. `display` builds the text an exposed
/// plan shows, only needed when plans can be exposed.
pub fn prepare_for_planning(
    operation: &OperationDefinition,
    supergraph: &SupergraphState,
    display: bool,
) -> DeferredOperation {
    let root_type = match operation.operation_kind {
        Some(OperationKind::Mutation) => supergraph.mutation_type.clone(),
        Some(OperationKind::Subscription) => supergraph.subscription_type.clone(),
        _ => None,
    }
    .unwrap_or_else(|| supergraph.query_type.clone());
    let mut operation = operation.clone();
    let mut if_variables = IfVariables {
        declared: operation
            .variable_definitions
            .iter()
            .flatten()
            .map(|definition| definition.name.clone())
            .collect(),
        pairs: Vec::new(),
    };
    split_conditional_defers(&mut operation.selection_set, &mut if_variables);
    if !if_variables.pairs.is_empty() {
        let definitions = operation.variable_definitions.get_or_insert_with(Vec::new);
        for (router, _) in &if_variables.pairs {
            definitions.push(VariableDefinition {
                name: router.clone(),
                variable_type: TypeNode::NonNull(Box::new(TypeNode::Named("Boolean".to_string()))),
                default_value: None,
            });
        }
    }
    let mut rewrite = Rewrite {
        supergraph,
        parts: Vec::new(),
        delivers: Vec::new(),
        ids: HashMap::new(),
        path: Vec::new(),
        display,
    };
    // shown with the client's own `if` variables, `$_d0` is `$var` with its default
    let shown = |mut set: SelectionSet| {
        with_client_variables(&mut set, &if_variables.pairs);
        subselection(set)
    };
    let primary = display
        .then(|| shown(rewrite.delivered(&operation.selection_set, &root_type)))
        .flatten();
    rewrite.selection_set(&mut operation.selection_set, &root_type, None);
    let Rewrite {
        parts, delivers, ..
    } = rewrite;
    // without `display`, `delivers` is empty
    let mut shown_parts: Vec<Option<String>> = delivers.into_iter().map(shown).collect();
    shown_parts.resize(parts.len(), None);
    DeferredOperation {
        operation,
        layout: DeferLayout {
            parts,
            display: DeferDisplay {
                primary,
                parts: shown_parts,
            },
            if_variables: if_variables.pairs,
        },
    }
}

/// Puts the client's variable back where the router's `@defer(if:)` variable is, in `@skip`
/// and `@include`. Only for showing: `$_d0` is `$var`, `true` when `$var` isn't sent, so
/// `@skip(if: $var)` reads the same with `if`'s default.
fn with_client_variables(set: &mut SelectionSet, pairs: &[(String, String)]) {
    let client = |var: &mut Option<String>| {
        if let Some((_, client)) = var
            .as_ref()
            .and_then(|name| pairs.iter().find(|(router, _)| router == name))
        {
            *var = Some(client.clone());
        }
    };
    for item in &mut set.items {
        match item {
            SelectionItem::Field(field) => {
                client(&mut field.skip_if);
                client(&mut field.include_if);
                with_client_variables(&mut field.selections, pairs);
            }
            SelectionItem::InlineFragment(fragment) => {
                client(&mut fragment.skip_if);
                client(&mut fragment.include_if);
                with_client_variables(&mut fragment.selections, pairs);
            }
            SelectionItem::FragmentSpread(_) => {}
        }
    }
}

/// Names for the router's `@defer(if:)` variables: `_d0`, `_d1`, ..., skipping the ones the
/// operation already declares.
struct IfVariables {
    declared: Vec<String>,
    /// `(router's, client's)`, one per client variable.
    pairs: Vec<(String, String)>,
}

impl IfVariables {
    fn router_name(&mut self, client: &str) -> String {
        if let Some((router, _)) = self.pairs.iter().find(|(_, c)| c == client) {
            return router.clone();
        }
        let router = (0..)
            .map(|i| format!("_d{i}"))
            .find(|name| !self.declared.contains(name))
            .expect("a free name");
        self.declared.push(router.clone());
        self.pairs.push((router.clone(), client.to_string()));
        router
    }
}

/// Splits each `@defer(if: $var)` fragment in two, see the module docs. Nested ones are split
/// first, so both copies have them split too: a conditional `@defer` k levels deep in other
/// conditional ones ends up in 2^k copies, siblings don't multiply.
fn split_conditional_defers(set: &mut SelectionSet, names: &mut IfVariables) {
    for item in std::mem::take(&mut set.items) {
        let item = match item {
            SelectionItem::Field(mut field) => {
                split_conditional_defers(&mut field.selections, names);
                SelectionItem::Field(field)
            }
            SelectionItem::InlineFragment(mut fragment) => {
                split_conditional_defers(&mut fragment.selections, names);
                match fragment
                    .defer
                    .as_ref()
                    .and_then(|marker| marker.if_var.clone())
                {
                    None => SelectionItem::InlineFragment(fragment),
                    Some(client) => {
                        let router = names.router_name(&client);
                        let mut inline = fragment.clone();
                        inline.defer = None;
                        set.items.push(SelectionItem::InlineFragment(under(
                            inline,
                            Skip,
                            router.clone(),
                        )));
                        SelectionItem::InlineFragment(under(fragment, Include, router))
                    }
                }
            }
            item => item,
        };
        set.items.push(item);
    }
}

enum Directive {
    Skip,
    Include,
}
use Directive::{Include, Skip};

/// `fragment` with `@skip(if: $var)` or `@include(if: $var)`, in a fragment of its own when it
/// already has that directive.
fn under(
    mut fragment: InlineFragmentSelection,
    directive: Directive,
    var: String,
) -> InlineFragmentSelection {
    let slot = match directive {
        Skip => &mut fragment.skip_if,
        Include => &mut fragment.include_if,
    };
    if slot.is_none() {
        *slot = Some(var);
        return fragment;
    }
    let mut outer = InlineFragmentSelection {
        type_condition: fragment.type_condition.clone(),
        selections: SelectionSet::default(),
        skip_if: None,
        include_if: None,
        defer: None,
    };
    match directive {
        Skip => outer.skip_if = Some(var),
        Include => outer.include_if = Some(var),
    }
    outer
        .selections
        .items
        .push(SelectionItem::InlineFragment(fragment));
    outer
}

/// `{ a { b } }`, spaced out so it's easy to read in a plan.
// ponytail: spaces string arguments with braces too, it's only for showing the plan
fn subselection(set: SelectionSet) -> Option<String> {
    if set.items.is_empty() {
        return None;
    }
    let text = set
        .to_string()
        .replace("...on ", "... on ")
        .replace('{', " { ")
        .replace('}', " } ");
    Some(text.split_whitespace().collect::<Vec<_>>().join(" "))
}

impl Rewrite<'_> {
    fn output_type(&self, parent_type: &str, field: &str) -> String {
        self.supergraph
            .definitions
            .get(parent_type)
            .and_then(|definition| definition.fields().get(field))
            .map_or("", |definition| definition.field_type.inner_type())
            .to_string()
    }

    /// `set` without the deferred fragments, and without fields left empty by that.
    fn delivered(&self, set: &SelectionSet, parent_type: &str) -> SelectionSet {
        let mut out = SelectionSet::default();
        for item in &set.items {
            match item {
                SelectionItem::Field(field) if !field.selections.items.is_empty() => {
                    let output_type = self.output_type(parent_type, &field.name);
                    let selections = self.delivered(&field.selections, &output_type);
                    if !selections.items.is_empty() {
                        let mut field = field.clone();
                        field.selections = selections;
                        out.items.push(SelectionItem::Field(field));
                    }
                }
                SelectionItem::InlineFragment(fragment) => {
                    if fragment.defer.is_some() {
                        continue;
                    }
                    out.items
                        .extend(self.fragment_delivered(fragment, parent_type).items);
                }
                item => out.items.push(item.clone()),
            }
        }
        out
    }

    /// What `fragment` delivers, its fields alone when it's on the parent type and has no
    /// conditions.
    fn fragment_delivered(
        &self,
        fragment: &InlineFragmentSelection,
        parent_type: &str,
    ) -> SelectionSet {
        let selections = self.delivered(&fragment.selections, &fragment.type_condition);
        if selections.items.is_empty()
            || (fragment.type_condition == parent_type
                && fragment.skip_if.is_none()
                && fragment.include_if.is_none())
        {
            return selections;
        }
        SelectionSet {
            items: vec![SelectionItem::InlineFragment(InlineFragmentSelection {
                type_condition: fragment.type_condition.clone(),
                selections,
                skip_if: fragment.skip_if.clone(),
                include_if: fragment.include_if.clone(),
                defer: None,
            })],
        }
    }

    fn selection_set(
        &mut self,
        set: &mut SelectionSet,
        parent_type: &str,
        parent: Option<DeferId>,
    ) {
        for item in &mut set.items {
            match item {
                SelectionItem::Field(field) => {
                    let field_type = self
                        .supergraph
                        .definitions
                        .get(parent_type)
                        .and_then(|definition| definition.fields().get(&field.name))
                        .map(|definition| &definition.field_type);
                    let lists = field_type.map_or(0, |t| t.list_depth());
                    let output_type = field_type.map_or("", |t| t.inner_type()).to_string();
                    let len = self.path.len();
                    self.path.push(PartPathSegment::Key(
                        field.selection_identifier().to_string(),
                    ));
                    self.path
                        .extend(std::iter::repeat_n(PartPathSegment::AnyIndex, lists));
                    self.selection_set(&mut field.selections, &output_type, parent);
                    self.path.truncate(len);
                }
                SelectionItem::InlineFragment(fragment) => {
                    let type_condition = fragment.type_condition.clone();
                    let Some(marker) = fragment.defer.as_mut() else {
                        self.selection_set(&mut fragment.selections, &type_condition, parent);
                        continue;
                    };
                    let key = (parent, self.path.clone(), marker.position);
                    let id = *self.ids.entry(key).or_insert_with(|| {
                        let id = DeferId::from_index(self.parts.len());
                        self.parts.push(DeferPart {
                            label: marker.label.clone(),
                            parent,
                            path: self.path.clone(),
                            position: marker.position,
                        });
                        if self.display {
                            self.delivers.push(SelectionSet::default());
                        }
                        id
                    });
                    marker.id = Some(id);
                    // before the nested markers are dropped, so their fields are left out
                    if self.display {
                        let delivers = self.fragment_delivered(fragment, parent_type);
                        self.delivers[id.index()].items.extend(delivers.items);
                    }
                    self.selection_set(&mut fragment.selections, &type_condition, Some(id));
                }
                SelectionItem::FragmentSpread(_) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DeferId;

    #[test]
    fn defer_ids_are_their_index_and_small() {
        let id = DeferId::from_index(0);
        assert_eq!(id.index(), 0);
        assert_eq!(id.to_string(), "0");
        assert_eq!(sonic_rs::to_string(&id).unwrap(), "0");
        assert_eq!(sonic_rs::from_str::<DeferId>("7").unwrap().index(), 7);
        assert!(DeferId::try_from(u32::MAX as usize).is_err());
        assert_eq!(size_of::<Option<DeferId>>(), 4);
    }
}
