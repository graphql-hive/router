use crate::ast::operation::OperationDefinition;
use crate::ast::selection_item::SelectionItem;
use crate::ast::selection_set::SelectionSet;

pub fn sort_operation(operation: OperationDefinition) -> OperationDefinition {
    let mut operation = operation;
    sort_selection_set_mut(&mut operation.selection_set);
    operation
}

fn sort_selection_set_mut(selection_set: &mut SelectionSet) {
    selection_set.items.sort();

    for item in &mut selection_set.items {
        sort_selection_item_mut(item);
    }
}

fn sort_selection_item_mut(item: &mut SelectionItem) {
    match item {
        SelectionItem::Field(field) => {
            sort_selection_set_mut(&mut field.selections);
        }
        SelectionItem::InlineFragment(fragment) => {
            sort_selection_set_mut(&mut fragment.selections);
        }
        SelectionItem::FragmentSpread(_) => {
            // No sorting needed for FragmentSpread
        }
    }
}
