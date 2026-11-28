use ahash::HashMap;
use std::collections::{BTreeSet, VecDeque};
use std::rc::Rc;

use tracing::{instrument, trace};

use crate::query_planner::{
    ast::{
        merge_path::{MergePath, Segment},
        selection_item::SelectionItem,
        selection_set::{FieldSelection, SelectionSet},
    },
    planner::fetch::{selections::FetchStepSelections, state::MultiTypeFetchStep},
    state::{
        subgraph_state::{SubgraphDefinition, SubgraphState},
        supergraph_state::{SubgraphName, SupergraphState, TypeNode},
    },
};

#[derive(Debug)]
pub struct SelectionMismatchFinder<'a> {
    supergraph_state: &'a SupergraphState,
}

/// The root type, the path of the field that needs an alias, and the index of its group's first
/// mismatch in the list: the fields of one group share an alias.
type MismatchesFound = Vec<(String, MergePath, usize)>;

/// A field selected under a response key.
struct Seen<'field, 'schema> {
    parent: &'schema str,
    field: &'field FieldSelection,
    /// `None` when the subgraph doesn't list the field, see `handle_field`
    in_subgraph: Option<&'schema TypeNode>,
}

/// The fields at one level of a selection set.
#[derive(Default)]
struct Level<'field, 'schema> {
    /// The first field under each response key. A field that doesn't match it gets an alias,
    /// see `handle_field`.
    first: HashMap<&'field str, Seen<'field, 'schema>>,
    /// The fields with an alias, with their response key and their group, so the same field
    /// selected again gets the same alias.
    aliased: Vec<(&'field str, Seen<'field, 'schema>, usize)>,
}

impl Seen<'_, '_> {
    /// The same field of the same type with the same arguments, one value in the response
    fn same_field(&self, other: &Seen) -> bool {
        self.parent == other.parent
            && self.field.name == other.field.name
            && self.field.arguments_hash() == other.field.arguments_hash()
    }
}

impl<'a> SelectionMismatchFinder<'a> {
    pub fn new(supergraph_state: &'a SupergraphState) -> Self {
        Self { supergraph_state }
    }

    #[instrument(level = "trace", skip_all, fields(subgraph_name,))]
    pub fn find_mismatches_in_node(
        &self,
        subgraph_name: &SubgraphName,
        selections: &FetchStepSelections<MultiTypeFetchStep>,
    ) -> MismatchesFound {
        let mut mismtaches_found = MismatchesFound::new();
        let subgraph_state = self
            .supergraph_state
            .subgraphs_state
            .get(subgraph_name)
            .unwrap();

        for (definition_name, selection_set) in selections.iter_selections() {
            let entrypoint_type = subgraph_state.definitions.get(definition_name).unwrap();
            let start_path = MergePath::default();

            handle_selection_set(
                definition_name,
                self.supergraph_state,
                subgraph_state,
                entrypoint_type,
                selection_set,
                start_path,
                &mut mismtaches_found,
            );

            trace!("found total of {} mismatches", mismtaches_found.len());
        }

        mismtaches_found
    }
}

/// Handles a selection set by traversing all selection in a specific level.
/// In case of a field: checks for mismatches and collects information.
/// In case of a fragment spread: "flattens" the selections and updates the path.
#[instrument(level = "trace", skip_all, fields(
  parent_def = parent_def.name(),
  selection = format!("{}", selection_set)
))]
fn handle_selection_set<'schema>(
    root_def_type_name: &str,
    supergraph_state: &'schema SupergraphState,
    subgraph_state: &'schema SubgraphState,
    parent_def: &'schema SubgraphDefinition,
    selection_set: &SelectionSet,
    parent_path: MergePath,
    mismatches_found: &mut MismatchesFound,
) {
    let mut level = Level::default();

    // A queue for traversing the selection set. In case of an inline fragment, we "flatten" the selections and update the path.
    // Values are: (ParentType, Path, Selections)
    let mut traversal_queue =
        VecDeque::from([(parent_def, parent_path, selection_set.items.iter())]);

    while let Some((type_def, path, selections_group)) = traversal_queue.pop_front() {
        for selection_item in selections_group {
            match selection_item {
                SelectionItem::Field(field) => {
                    if field.is_introspection_field() {
                        continue;
                    }

                    let next_path = path.push(Segment::Field(
                        Rc::new(field.into()),
                        field.arguments_hash(),
                        field.into(),
                    ));

                    let next_parent_type_name = handle_field(
                        root_def_type_name,
                        supergraph_state,
                        type_def,
                        field,
                        &next_path,
                        &mut level,
                        mismatches_found,
                    );

                    if let Some(next_parent_def) =
                        next_parent_type_name.and_then(|n| subgraph_state.definitions.get(n))
                    {
                        handle_selection_set(
                            root_def_type_name,
                            supergraph_state,
                            subgraph_state,
                            next_parent_def,
                            &field.selections,
                            next_path,
                            mismatches_found,
                        );
                    }
                }
                SelectionItem::FragmentSpread(_) => {
                    unreachable!("fragment spread is not expected at this stage")
                }
                SelectionItem::InlineFragment(fragment) => {
                    // same as for fields below a type the subgraph doesn't declare: skipped
                    let Some(fragment_type) =
                        subgraph_state.definitions.get(&fragment.type_condition)
                    else {
                        continue;
                    };
                    let fragment_enter_path = path.push(Segment::TypeCondition(
                        Rc::new(BTreeSet::from([fragment.type_condition.clone()])),
                        fragment.into(),
                    ));

                    traversal_queue.push_back((
                        fragment_type,
                        fragment_enter_path,
                        fragment.selections.items.iter(),
                    ));
                }
            }
        }
    }
}

/// Handles a field by finding it's subgraph-specific type (or the default type if none is set),
/// and then checks if we've already encountered this field in the current selection set level.
/// In case the field encountered, we check if the field is compatible or not (have a mismatch).
///
/// Returns the return type of the selection, if the inner selection needs to be processed (in case nested selections are defined).
fn handle_field<'field, 'schema>(
    root_def_type_name: &str,
    state: &'schema SupergraphState,
    parent_def: &'schema SubgraphDefinition,
    field: &'field FieldSelection,
    field_path: &MergePath,
    level: &mut Level<'field, 'schema>,
    mismatches_found: &mut MismatchesFound,
) -> Option<&'schema str> {
    let field_name = field.name.as_str();
    let field_identifier = field.selection_identifier();
    let in_subgraph = parent_def.fields().into_iter().flatten().find_map(|f| {
        (f.name == field_name).then(|| {
            f.join_field
                .as_ref()
                .and_then(|jf| jf.type_in_graph.as_ref())
                .unwrap_or(&f.field_type)
        })
    });
    // a supergraph can leave out the `external: true` entry of a provided field, so the
    // subgraph's type doesn't list it even though the subgraph serves it. Its supergraph type
    // is good enough to walk into, but it says nothing about what the subgraph declares, so a
    // field like that only matches the same field selected again under its response key. Any
    // other one gets an alias then.
    let field_type = in_subgraph.or_else(|| {
        let definition = state.definitions.get(parent_def.name())?;
        Some(&definition.fields().get(field_name)?.field_type)
    })?;

    let this = Seen {
        parent: parent_def.name(),
        field,
        in_subgraph,
    };
    if let Some(first) = level.first.get(field_identifier) {
        let matching = matches!(
            (first.in_subgraph, in_subgraph),
            (Some(first), Some(this)) if same_response_shape(state, first, this)
        ) || first.same_field(&this);
        if !matching {
            trace!(
              "found a conflicting type for a selection field '{}', conflict is: '{:?}' <-> '{}', path: {}",
              field_identifier,
              first.in_subgraph,
              field_type,
              field_path,
          );

            // the same field selected again joins the group of the first one, its value is
            // one object in the subgraph's response
            let group = match level
                .aliased
                .iter()
                .find(|(key, seen, _)| *key == field_identifier && seen.same_field(&this))
            {
                Some(&(_, _, group)) => group,
                None => {
                    level
                        .aliased
                        .push((field_identifier, this, mismatches_found.len()));
                    mismatches_found.len()
                }
            };
            mismatches_found.push((root_def_type_name.to_string(), field_path.clone(), group));
        }
    } else {
        level.first.insert(field_identifier, this);
    }

    if field.is_leaf() {
        None
    } else {
        Some(field_type.inner_type())
    }
}

/// GraphQL's "same response shape" (https://spec.graphql.org/draft/#SameResponseShape()):
/// lists and non-nulls have to line up level by level, and the named types have to be equal,
/// unless both are composite (their fields are compared on their own). `[Item]` and `Item`
/// can't share a response key, even though both are objects.
fn same_response_shape(state: &SupergraphState, left: &TypeNode, right: &TypeNode) -> bool {
    match (left, right) {
        (TypeNode::List(left), TypeNode::List(right))
        | (TypeNode::NonNull(left), TypeNode::NonNull(right)) => {
            same_response_shape(state, left, right)
        }
        (TypeNode::Named(left), TypeNode::Named(right)) => {
            let is_composite = |name: &String| {
                state
                    .definitions
                    .get(name)
                    .is_some_and(|definition| definition.is_composite_type())
            };
            left == right || (is_composite(left) && is_composite(right))
        }
        _ => false,
    }
}
