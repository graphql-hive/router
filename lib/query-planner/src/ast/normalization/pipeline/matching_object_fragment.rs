use graphql_tools::parser::query::{InlineFragment, Selection, TypeCondition};

#[inline]
pub(super) fn can_flatten_matching_object_fragment<'a>(
    fragment: &InlineFragment<'a, String>,
    object_type_name: &str,
) -> bool {
    if !fragment.directives.is_empty()
        || !matches!(
            fragment.type_condition.as_ref(),
            Some(TypeCondition::On(type_name)) if type_name == object_type_name
        )
    {
        return false;
    }

    // Keep wrappers that directly select `__typename`.
    // Downstream normalization and planner behavior rely on preserving that explicit type-scoped shape.
    !fragment.selection_set.items.iter().any(|selection| {
        matches!(
            selection,
            Selection::Field(field) if field.name == "__typename"
        )
    })
}
