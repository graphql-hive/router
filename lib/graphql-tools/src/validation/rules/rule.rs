use crate::{ast::OperationVisitorContext, validation::utils::ValidationErrorContext};

pub trait ValidationRule: Send + Sync {
    fn validate(
        &self,
        _ctx: &mut OperationVisitorContext<'_>,
        _error_collector: &mut ValidationErrorContext,
    );

    fn error_code<'a>(&self) -> &'a str;
}
