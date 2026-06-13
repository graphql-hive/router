use crate::ast::{
    selection_item::SelectionItem,
    selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
};

pub trait SemanticEq<Rhs = Self> {
    fn semantic_eq(&self, other: &Rhs) -> bool;
}

impl SemanticEq for SelectionSet {
    fn semantic_eq(&self, other: &Self) -> bool {
        if self.items.len() != other.items.len() {
            return false;
        }

        let mut matched = vec![false; other.items.len()];

        self.items.iter().all(|left| {
            if let Some(index) = other
                .items
                .iter()
                .enumerate()
                .position(|(index, right)| !matched[index] && left.semantic_eq(right))
            {
                matched[index] = true;
                true
            } else {
                false
            }
        })
    }
}

impl SemanticEq for FieldSelection {
    fn semantic_eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.alias == other.alias
            && self.arguments() == other.arguments()
            && self.skip_if == other.skip_if
            && self.include_if == other.include_if
            && self.omit_from_response == other.omit_from_response
            && self.selections.semantic_eq(&other.selections)
    }
}

impl SemanticEq for InlineFragmentSelection {
    fn semantic_eq(&self, other: &Self) -> bool {
        self.type_condition == other.type_condition
            && self.skip_if == other.skip_if
            && self.include_if == other.include_if
            && self.selections.semantic_eq(&other.selections)
    }
}

impl SemanticEq for SelectionItem {
    fn semantic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (SelectionItem::Field(left), SelectionItem::Field(right)) => left.semantic_eq(right),
            (SelectionItem::InlineFragment(left), SelectionItem::InlineFragment(right)) => {
                left.semantic_eq(right)
            }
            (SelectionItem::FragmentSpread(left), SelectionItem::FragmentSpread(right)) => {
                left == right
            }
            _ => false,
        }
    }
}
