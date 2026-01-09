use std::collections::hash_map::Entry;
use std::collections::HashMap;

use crate::parser::Pos;

use super::ValidationRule;
use crate::ast::{visit_document, OperationVisitor, OperationVisitorContext};
use crate::static_graphql::query::*;
use crate::validation::utils::{ValidationError, ValidationErrorContext};

/// Unique variable names
///
/// A GraphQL operation is only valid if all its variables are uniquely named.
///
/// See https://spec.graphql.org/draft/#sec-Variable-Uniqueness
#[derive(Default)]
pub struct UniqueVariableNames<'a> {
    found_records: HashMap<&'a str, Pos>,
}

impl<'a> UniqueVariableNames<'a> {
    pub fn new() -> Self {
        UniqueVariableNames {
            found_records: HashMap::new(),
        }
    }
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for UniqueVariableNames<'a> {
    fn enter_operation_definition(
        &mut self,
        _: &mut OperationVisitorContext,
        _: &mut ValidationErrorContext,
        _operation_definition: &OperationDefinition,
    ) {
        self.found_records.clear();
    }

    fn enter_variable_definition(
        &mut self,
        _: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        variable_definition: &'a VariableDefinition,
    ) {
        let error_code = self.error_code();
        match self.found_records.entry(&variable_definition.name) {
            Entry::Occupied(entry) => user_context.report_error(ValidationError {
                error_code,
                locations: vec![*entry.get(), variable_definition.position],
                message: format!(
                    "There can only be one variable named \"${}\".",
                    variable_definition.name
                ),
            }),
            Entry::Vacant(entry) => {
                entry.insert(variable_definition.position);
            }
        };
    }
}

impl<'v> ValidationRule for UniqueVariableNames<'v> {
    fn error_code<'a>(&self) -> &'a str {
        "UniqueVariableNames"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext,
        error_collector: &mut ValidationErrorContext,
    ) {
        visit_document(
            &mut UniqueVariableNames::new(),
            ctx.operation,
            ctx,
            error_collector,
        );
    }
}

#[test]
fn unique_variable_names() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueVariableNames::new()));
    let errors = test_operation_with_schema(
        "query A($x: Int, $y: String) { __typename }
        query B($x: String, $y: Int) { __typename }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn duplicate_variable_names() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueVariableNames::new()));
    let errors = test_operation_with_schema(
        "query A($x: Int, $x: Int, $x: String) { __typename }
        query B($y: String, $y: Int) { __typename }
        query C($z: Int, $z: Int) { __typename }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);

    assert_eq!(messages.len(), 4);
    assert!(messages.contains(&&"There can only be one variable named \"$x\".".to_owned()));
    assert!(messages.contains(&&"There can only be one variable named \"$y\".".to_owned()));
    assert!(messages.contains(&&"There can only be one variable named \"$z\".".to_owned()));
}
