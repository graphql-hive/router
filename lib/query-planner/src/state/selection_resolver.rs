use graphql_parser_hive_fork::query::{Field, SelectionSet};
use graphql_tools::static_graphql::query::Selection as OperationSelectionKind;

use crate::graph::selection::{parse_selection_set, Selection, SelectionNode};

use super::subgraph_state::SubgraphState;

#[derive(Debug)]
pub struct SelectionResolver {
    pub subgraph_state: SubgraphState,
}

#[derive(Debug, thiserror::Error)]
pub enum SelectionResolverError {
    #[error("definition with name '{0}' was not found")]
    DefinitionNotFound(String),
    #[error("definition field '{0}' was not found in '{1}'")]
    DefinitionFieldNotFound(String, String),
}

impl SelectionResolver {
    pub fn new_from_state(subgraph: SubgraphState) -> Self {
        Self {
            subgraph_state: subgraph,
        }
    }

    pub fn resolve(
        &self,
        type_name: &str,
        key_fields: &str,
    ) -> Result<Selection, SelectionResolverError> {
        let subgraph_type_def = self
            .subgraph_state
            .definitions
            .get(type_name)
            .ok_or_else(|| SelectionResolverError::DefinitionNotFound(type_name.to_string()))?;

        let selection_set = parse_selection_set(&format!("{{ {} }}", key_fields));
        let selection_nodes =
            self.resolve_selection_set(subgraph_type_def.name(), &selection_set)?;
        let fields = Selection::new(
            subgraph_type_def.name().to_string(),
            key_fields.to_string(),
            selection_nodes,
        );

        Ok(fields)
    }

    fn resolve_field_selection(
        &self,
        type_name: &str,
        selection_field: &Field<'static, String>,
    ) -> Result<SelectionNode, SelectionResolverError> {
        let type_state = self
            .subgraph_state
            .definitions
            .get(type_name)
            .ok_or_else(|| SelectionResolverError::DefinitionNotFound(type_name.to_string()))?;
        let field_in_type_def = type_state
            .fields()
            .unwrap()
            .iter()
            .find(|f| f.name == selection_field.name)
            .ok_or_else(|| {
                SelectionResolverError::DefinitionFieldNotFound(
                    type_name.to_string(),
                    selection_field.name.to_string(),
                )
            })?;

        let selections = if selection_field.selection_set.items.is_empty() {
            None
        } else {
            Some(self.resolve_selection_set(
                &field_in_type_def.return_type_name,
                &selection_field.selection_set,
            )?)
        };

        Ok(SelectionNode::Field {
            field_name: field_in_type_def.name.clone(),
            type_name: type_name.to_string(),
            selections,
        })
    }

    fn resolve_selection_set(
        &self,
        type_name: &str,
        selection_set: &SelectionSet<'static, String>,
    ) -> Result<Vec<SelectionNode>, SelectionResolverError> {
        let mut result: Vec<SelectionNode> = vec![];

        for selection in &selection_set.items {
            match selection {
                OperationSelectionKind::Field(field) => {
                    let selection_node = self.resolve_field_selection(type_name, field)?;
                    result.push(selection_node);
                }
                OperationSelectionKind::InlineFragment(_fragment) => {
                    unimplemented!("not supported yet")
                }
                OperationSelectionKind::FragmentSpread(_spread) => {
                    unimplemented!("not supported yet")
                }
            }
        }

        result.sort();

        Ok(result)
    }
}
