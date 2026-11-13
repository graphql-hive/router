use ahash::HashSet;

use petgraph::graph::NodeIndex;
use tracing::{instrument, trace};

use crate::query_planner::{
    ast::{
        flat::{
            arena::{Arena, ListId},
            Kind,
        },
        merge_path::{MergePath, Segment},
        mismatch_finder::SelectionMismatchFinder,
    },
    planner::{
        fetch::{error::FetchGraphError, fetch_graph::FetchGraph, state::MultiTypeFetchStep},
        plan_nodes::{FetchNodePathSegment, FetchRewrite, KeyRenamer},
    },
    state::supergraph_state::SupergraphState,
};

impl FetchGraph<MultiTypeFetchStep> {
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn fix_conflicting_type_mismatches(
        &mut self,
        supergraph: &SupergraphState,
    ) -> Result<(), FetchGraphError> {
        let mut pending_patches = Vec::<(NodeIndex, Vec<(String, MergePath)>)>::new();

        for (node_index, node) in self.all_nodes() {
            if self.root_index.is_some_and(|v| v == node_index) {
                continue;
            }

            trace!(
                "looking for type conflict mismatches in node [{}]",
                node_index.index()
            );

            let finder = SelectionMismatchFinder::new(supergraph);
            let mismatches_paths =
                finder.find_mismatches_in_node(&node.service_name, &node.output, &self.arena);

            if !mismatches_paths.is_empty() {
                pending_patches.push((node_index, mismatches_paths));
            }
        }

        let mut pending_output_rewrites = Vec::<(NodeIndex, FetchRewrite)>::new();

        for (node_index, mismatches_paths) in pending_patches {
            let (node, arena) = self.step_and_arena_mut(node_index)?;

            trace!(
                "fixing {} mismatch conflicts in node [{}] by using aliases",
                mismatches_paths.len(),
                node_index.index()
            );

            for (root_def_name, mismatch_path) in mismatches_paths {
                if let Some(Segment::Field(field_seg, args_hash_lookup, condition)) =
                    mismatch_path.last()
                {
                    let lookup_path = &mismatch_path.without_last();
                    let root_def_list = node
                        .output
                        .selections_for_definition(&root_def_name)
                        .expect("missing definition in step");

                    let next_alias = free_alias(arena, root_def_list, lookup_path);
                    let Some(list) = arena.find_by_path(root_def_list, lookup_path) else {
                        continue;
                    };
                    let field_to_alias = arena.items(list).iter().copied().find(|&id| {
                        let field = arena.node(id);
                        field.kind == Kind::Field
                            && arena.response_key(id) == field_seg.response_key()
                            && arena.args_hash(field.args) == *args_hash_lookup
                            && arena.condition_is(id, condition.as_ref())
                    });

                    if let Some(field_to_alias) = field_to_alias {
                        let original_response_key = arena.response_key(field_to_alias).to_string();

                        trace!(
                            "applying alias '{}' to existing field '{}' at path '{}'",
                            next_alias,
                            arena.str(arena.node(field_to_alias).name),
                            lookup_path
                        );

                        let mut output_rewrite_path: Vec<FetchNodePathSegment> = lookup_path.into();
                        output_rewrite_path.push(FetchNodePathSegment::Key(next_alias.clone()));

                        pending_output_rewrites.push((
                            node_index,
                            FetchRewrite::KeyRenamer(KeyRenamer {
                                rename_key_to: original_response_key,
                                path: output_rewrite_path,
                            }),
                        ));

                        // the list is this step's own, so it changes in place
                        let alias = arena.intern(&next_alias);
                        arena.node_mut(field_to_alias).alias = alias;
                    }
                }
            }
        }

        for (node_index, output_rewrite) in pending_output_rewrites {
            let node = self.get_step_data_mut(node_index)?;

            trace!(
                "adding output rewrite to node [{}]: {:?}",
                node_index.index(),
                output_rewrite
            );

            node.add_output_rewrite(output_rewrite);
        }

        Ok(())
    }
}

/// An alias no field of the object at `path` uses yet. The object's keys are all of its fields,
/// in every fragment, not just the ones next to the field: `TypeB` and `TypeC` fragments on one
/// object can't both get `_internal_qp_alias_0`, and neither can a field next to a client's
/// `_internal_qp_alias_0: __typename`.
fn free_alias(arena: &Arena, selections: ListId, path: &MergePath) -> String {
    fn collect<'a>(arena: &'a Arena, list: ListId, taken: &mut HashSet<&'a str>) {
        for &id in arena.items(list) {
            match arena.node(id).kind {
                Kind::Field => {
                    taken.insert(arena.response_key(id));
                }
                Kind::InlineFragment => collect(arena, arena.node(id).children, taken),
                Kind::FragmentSpread => {}
            }
        }
    }

    let mut object_path = path.clone();
    while matches!(object_path.last(), Some(Segment::TypeCondition(..))) {
        object_path = object_path.without_last();
    }
    let mut taken = HashSet::default();
    if let Some(object) = arena.find_by_path(selections, &object_path) {
        collect(arena, object, &mut taken);
    }

    (0..)
        .map(|index| format!("_internal_qp_alias_{index}"))
        .find(|alias| !taken.contains(alias.as_str()))
        .unwrap()
}
