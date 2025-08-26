use graphql_parser::query::Definition;

use crate::ast::normalization::{context::NormalizationContext, error::NormalizationError};

#[inline]
pub fn drop_fragment_definitions(ctx: &mut NormalizationContext) -> Result<(), NormalizationError> {
    ctx.document.definitions.retain_mut(|def| match def {
        Definition::Operation(_) => true,
        Definition::Fragment(_) => false,
    });

    Ok(())
}
