use graphql_parser::query::{Field, SelectionSet as ParserSelectionSet};
use graphql_tools::static_graphql::query::Selection as OperationSelectionKind;

use crate::ast::{
    parse_selection_set,
    selection_item::SelectionItem,
    selection_set::{FieldSelection, SelectionSet},
    type_aware_selection::TypeAwareSelection,
};

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
    ) -> Result<TypeAwareSelection, SelectionResolverError> {
        let subgraph_type_def = self
            .subgraph_state
            .definitions
            .get(type_name)
            .ok_or_else(|| SelectionResolverError::DefinitionNotFound(type_name.to_string()))?;

        let selection_set = parse_selection_set(&format!("{{ {} }}", key_fields));
        let selection_nodes =
            self.resolve_selection_set(subgraph_type_def.name(), &selection_set)?;
        let selection =
            TypeAwareSelection::new(subgraph_type_def.name().to_string(), selection_nodes);

        Ok(selection)
    }

    fn resolve_field_selection(
        &self,
        type_name: &str,
        selection_field: &Field<'static, String>,
    ) -> Result<SelectionItem, SelectionResolverError> {
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

        Ok(SelectionItem::Field(FieldSelection {
            name: field_in_type_def.name.clone(),
            is_leaf: selections.is_none(),
            selections: selections.unwrap_or_default(),
            alias: None,
        }))
    }

    fn resolve_selection_set(
        &self,
        type_name: &str,
        selection_set: &ParserSelectionSet<'static, String>,
    ) -> Result<SelectionSet, SelectionResolverError> {
        let mut result: Vec<SelectionItem> = vec![];

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

        Ok(SelectionSet { items: result })
    }
}
