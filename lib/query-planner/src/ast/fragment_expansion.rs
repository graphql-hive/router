use super::{
    fragment::FragmentDefinition,
    selection_item::SelectionItem,
    selection_set::{InlineFragmentSelection, SelectionSet},
};

#[derive(Debug, Clone, thiserror::Error)]
pub enum FragmentExpansionError {
    #[error("Could not find fragment definition '{0}'")]
    MissingFragment(String),
    #[error("Detected cycle while expanding fragment spread '{0}'")]
    Cycle(String),
}

impl SelectionSet {
    pub fn inline_fragment_spreads(
        &self,
        fragments: &[FragmentDefinition],
    ) -> Result<SelectionSet, FragmentExpansionError> {
        let mut visiting = Vec::new();
        inline_fragment_spreads_inner(self, fragments, &mut visiting)
    }
}

fn inline_fragment_spreads_inner(
    selection_set: &SelectionSet,
    fragments: &[FragmentDefinition],
    visiting: &mut Vec<String>,
) -> Result<SelectionSet, FragmentExpansionError> {
    let mut items = Vec::with_capacity(selection_set.items.len());

    for item in &selection_set.items {
        match item {
            SelectionItem::Field(field) => {
                let expanded_selections =
                    inline_fragment_spreads_inner(&field.selections, fragments, visiting)?;
                items.push(SelectionItem::Field(
                    field.with_new_selections(expanded_selections),
                ));
            }
            SelectionItem::InlineFragment(fragment) => {
                let expanded_selections =
                    inline_fragment_spreads_inner(&fragment.selections, fragments, visiting)?;
                items.push(SelectionItem::InlineFragment(
                    fragment.with_new_selections(expanded_selections),
                ));
            }
            SelectionItem::FragmentSpread(fragment_name) => {
                if visiting.contains(fragment_name) {
                    return Err(FragmentExpansionError::Cycle(fragment_name.clone()));
                }

                let fragment = fragments
                    .iter()
                    .find(|fragment| fragment.name == *fragment_name)
                    .ok_or_else(|| {
                        FragmentExpansionError::MissingFragment(fragment_name.clone())
                    })?;

                visiting.push(fragment_name.clone());
                let expanded_selections =
                    inline_fragment_spreads_inner(&fragment.selection_set, fragments, visiting)?;
                visiting.pop();

                items.push(SelectionItem::InlineFragment(InlineFragmentSelection {
                    type_condition: fragment.type_condition.clone(),
                    selections: expanded_selections,
                    skip_if: None,
                    include_if: None,
                }));
            }
        }
    }

    Ok(SelectionSet { items })
}
