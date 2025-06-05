use graphql_parser::query::{
    Definition, Field, InlineFragment, OperationDefinition, Selection, SelectionSet,
};

use crate::ast::normalization::{context::NormalizationContext, error::NormalizationError};

pub fn normalize_fields(ctx: &mut NormalizationContext) -> Result<(), NormalizationError> {
    for def in ctx.document.definitions.iter_mut() {
        match def {
            Definition::Operation(ref mut op) => match op {
                OperationDefinition::Query(ref mut q) => {
                    normalize_selection_set(&mut q.selection_set)?;
                }
                OperationDefinition::Mutation(ref mut m) => {
                    normalize_selection_set(&mut m.selection_set)?;
                }
                OperationDefinition::Subscription(ref mut s) => {
                    normalize_selection_set(&mut s.selection_set)?;
                }
                OperationDefinition::SelectionSet(ref mut s) => {
                    normalize_selection_set(s)?;
                }
            },
            Definition::Fragment(ref mut fr) => {
                normalize_selection_set(&mut fr.selection_set)?;
            }
        }
    }

    Ok(())
}

fn normalize_selection_set<'b, 'a>(
    selection_set: &'b mut SelectionSet<'a, String>,
) -> Result<(), NormalizationError> {
    for item in selection_set.items.iter_mut() {
        match item {
            Selection::Field(ref mut field) => {
                drop_name_equal_alias(field)?;
                sort_arguments(field)?;
                sort_field_directives(field)?;
            }
            Selection::InlineFragment(ref mut fr) => {
                normalize_selection_set(&mut fr.selection_set)?;
                sort_fragment_directives(fr)?;
            }
            Selection::FragmentSpread(_) => {
                // fragment definitions are handled at top level
            }
        }
    }

    Ok(())
}

fn drop_name_equal_alias<'b, 'a>(
    field: &'b mut Field<'a, String>,
) -> Result<(), NormalizationError> {
    if field
        .alias
        .as_ref()
        .is_none_or(|alias| alias != &field.name)
    {
        return Ok(());
    }

    field.alias = None;

    Ok(())
}

fn sort_arguments<'b, 'a>(field: &'b mut Field<'a, String>) -> Result<(), NormalizationError> {
    field.arguments.sort_by(|(a, _), (b, _)| a.cmp(b));

    Ok(())
}

fn sort_field_directives<'b, 'a>(
    field: &'b mut Field<'a, String>,
) -> Result<(), NormalizationError> {
    field.directives.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(())
}

fn sort_fragment_directives<'b, 'a>(
    fragment: &'b mut InlineFragment<'a, String>,
) -> Result<(), NormalizationError> {
    fragment.directives.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(())
}
