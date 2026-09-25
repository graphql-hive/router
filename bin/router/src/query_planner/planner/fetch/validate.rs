//! A check, in debug builds, that every fetch asks its subgraph only for what the subgraph
//! has, and that its fields fit in one operation. It catches a planner bug in the test that
//! hits it, instead of in the subgraph that gets the operation.

use crate::query_planner::{
    ast::{
        safe_merge::SafeSelectionSetMerger, selection_item::SelectionItem,
        selection_set::SelectionSet,
    },
    planner::fetch::{error::FetchGraphError, fetch_graph::FetchGraph},
    state::{subgraph_state::SubgraphState, supergraph_state::SupergraphState},
};

impl FetchGraph {
    pub(crate) fn validate_operations(
        &self,
        supergraph: &SupergraphState,
    ) -> Result<(), FetchGraphError> {
        let merger = SafeSelectionSetMerger::new(supergraph);
        for index in self.step_indices() {
            if self.root_index == Some(index) {
                continue;
            }
            let step = self.get_step_data(index)?;
            let subgraph = supergraph
                .subgraph_state(&step.service_name)
                .map_err(|err| FetchGraphError::Internal(err.to_string()))?;
            for (type_name, selections) in step.output.iter_selections() {
                let problem = match check_selection_set(subgraph, type_name, selections) {
                    Err(problem) => Some(problem),
                    Ok(()) => merger.check_conflicts(selections).err().map(|conflict| {
                        format!("fields with one response key don't fit together: {conflict}")
                    }),
                };
                if let Some(problem) = problem {
                    return Err(FetchGraphError::Internal(format!(
                        "invalid operation for subgraph \"{}\": {problem}, in {step}",
                        step.service_name
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Is every field of `selection_set` a field of `type_name` in `subgraph`, and every fragment
/// on a type it has?
fn check_selection_set(
    subgraph: &SubgraphState,
    type_name: &str,
    selection_set: &SelectionSet,
) -> Result<(), String> {
    let definition = subgraph
        .definitions
        .get(type_name)
        .ok_or_else(|| format!("no type {type_name}"))?;
    for item in &selection_set.items {
        match item {
            SelectionItem::Field(field) if field.name == "__typename" => {}
            SelectionItem::Field(field) => {
                let field_definition = definition
                    .field(&field.name)
                    .ok_or_else(|| format!("no field {type_name}.{}", field.name))?;
                if !field.selections.is_empty() {
                    let field_type = field_definition
                        .join_field
                        .as_ref()
                        .and_then(|join_field| join_field.type_in_graph.as_ref())
                        .unwrap_or(&field_definition.field_type);
                    check_selection_set(subgraph, field_type.inner_type(), &field.selections)?;
                }
            }
            SelectionItem::InlineFragment(fragment) => {
                check_selection_set(subgraph, &fragment.type_condition, &fragment.selections)?
            }
            SelectionItem::FragmentSpread(_) => {}
        }
    }
    Ok(())
}
