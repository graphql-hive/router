use std::collections::{HashMap, HashSet};

use super::ValidationRule;
use crate::ast::{
    visit_document, AstNodeWithName, OperationVisitor, OperationVisitorContext, ValueExtension,
};
use crate::static_graphql::query::{self, OperationDefinition};
use crate::validation::utils::{ValidationError, ValidationErrorContext};

/// No unused fragments
///
/// A GraphQL operation is only valid if all variables defined by an operation
/// are used, either directly or within a spread fragment.
///
/// See https://spec.graphql.org/draft/#sec-All-Variables-Used
pub struct NoUnusedVariables<'a> {
    current_scope: Option<NoUnusedVariablesScope<'a>>,
    defined_variables: HashMap<Option<&'a str>, HashSet<&'a str>>,
    used_variables: HashMap<NoUnusedVariablesScope<'a>, Vec<&'a str>>,
    spreads: HashMap<NoUnusedVariablesScope<'a>, Vec<&'a str>>,
}

impl<'a> Default for NoUnusedVariables<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> NoUnusedVariables<'a> {
    pub fn new() -> Self {
        Self {
            current_scope: None,
            defined_variables: HashMap::new(),
            used_variables: HashMap::new(),
            spreads: HashMap::new(),
        }
    }
}

impl<'a> NoUnusedVariables<'a> {
    fn find_used_vars(
        &self,
        from: &NoUnusedVariablesScope<'a>,
        defined: &HashSet<&str>,
        used: &mut HashSet<&'a str>,
        visited: &mut HashSet<NoUnusedVariablesScope<'a>>,
    ) {
        if visited.contains(from) {
            return;
        }

        visited.insert(from.clone());

        if let Some(used_vars) = self.used_variables.get(from) {
            for var in used_vars {
                if defined.contains(var) {
                    used.insert(var);
                }
            }
        }

        if let Some(spreads) = self.spreads.get(from) {
            for spread in spreads {
                self.find_used_vars(
                    &NoUnusedVariablesScope::Fragment(spread),
                    defined,
                    used,
                    visited,
                );
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NoUnusedVariablesScope<'a> {
    Operation(Option<&'a str>),
    Fragment(&'a str),
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for NoUnusedVariables<'a> {
    fn enter_operation_definition(
        &mut self,
        _: &mut OperationVisitorContext,
        _: &mut ValidationErrorContext,
        operation_definition: &'a OperationDefinition,
    ) {
        let op_name = operation_definition.node_name();
        self.current_scope = Some(NoUnusedVariablesScope::Operation(op_name));
        self.defined_variables.insert(op_name, HashSet::new());
    }

    fn enter_fragment_definition(
        &mut self,
        _: &mut OperationVisitorContext,
        _: &mut ValidationErrorContext,
        fragment_definition: &'a query::FragmentDefinition,
    ) {
        self.current_scope = Some(NoUnusedVariablesScope::Fragment(&fragment_definition.name));
    }

    fn enter_fragment_spread(
        &mut self,
        _: &mut OperationVisitorContext,
        _: &mut ValidationErrorContext,
        fragment_spread: &'a query::FragmentSpread,
    ) {
        if let Some(scope) = &self.current_scope {
            self.spreads
                .entry(scope.clone())
                .or_default()
                .push(&fragment_spread.fragment_name);
        }
    }

    fn enter_variable_definition(
        &mut self,
        _: &mut OperationVisitorContext,
        _: &mut ValidationErrorContext,
        variable_definition: &'a query::VariableDefinition,
    ) {
        if let Some(NoUnusedVariablesScope::Operation(ref name)) = self.current_scope {
            if let Some(vars) = self.defined_variables.get_mut(name) {
                vars.insert(&variable_definition.name);
            }
        }
    }

    fn enter_argument(
        &mut self,
        _: &mut OperationVisitorContext,
        _: &mut ValidationErrorContext,
        (_arg_name, arg_value): &'a (String, query::Value),
    ) {
        if let Some(ref scope) = self.current_scope {
            self.used_variables
                .entry(scope.clone())
                .or_default()
                .append(&mut arg_value.variables_in_use());
        }
    }

    fn leave_document(
        &mut self,
        _: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        _: &query::Document,
    ) {
        for (op_name, def_vars) in &self.defined_variables {
            let mut used = HashSet::new();
            let mut visited = HashSet::new();

            self.find_used_vars(
                &NoUnusedVariablesScope::Operation(*op_name),
                def_vars,
                &mut used,
                &mut visited,
            );

            def_vars
                .iter()
                .filter(|var| !used.contains(*var))
                .for_each(|var| {
                    user_context.report_error(ValidationError {
                        error_code: self.error_code(),
                        message: error_message(var, op_name),
                        locations: vec![],
                    })
                })
        }
    }
}

fn error_message(var_name: &str, op_name: &Option<&str>) -> String {
    if let Some(op_name) = op_name {
        format!(
            r#"Variable "${}" is never used in operation "{}"."#,
            var_name, op_name
        )
    } else {
        format!(r#"Variable "${}" is never used."#, var_name)
    }
}

impl<'n> ValidationRule for NoUnusedVariables<'n> {
    fn error_code<'a>(&self) -> &'a str {
        "NoUnusedVariables"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext,
        error_collector: &mut ValidationErrorContext,
    ) {
        visit_document(
            &mut NoUnusedVariables::new(),
            ctx.operation,
            ctx,
            error_collector,
        );
    }
}

#[test]
fn use_all_variables() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(NoUnusedVariables::new()));
    let errors = test_operation_with_schema(
        "query ($a: String, $b: String, $c: String) {
        field(a: $a, b: $b, c: $c)
      }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn use_all_variables_deeply() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(NoUnusedVariables::new()));
    let errors = test_operation_with_schema(
        "query Foo($a: String, $b: String, $c: String) {
      field(a: $a) {
        field(b: $b) {
          field(c: $c)
        }
      }
    }
  ",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn use_all_variables_deeply_in_inline_fragments() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(NoUnusedVariables::new()));
    let errors = test_operation_with_schema(
        " query Foo($a: String, $b: String, $c: String) {
      ... on Type {
        field(a: $a) {
          field(b: $b) {
            ... on Type {
              field(c: $c)
            }
          }
        }
      }
    }
  ",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn use_all_variables_in_fragments() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(NoUnusedVariables::new()));
    let errors = test_operation_with_schema(
        "query Foo($a: String, $b: String, $c: String) {
      ...FragA
    }
    fragment FragA on Type {
      field(a: $a) {
        ...FragB
      }
    }
    fragment FragB on Type {
      field(b: $b) {
        ...FragC
      }
    }
    fragment FragC on Type {
      field(c: $c)
    }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn variables_used_by_fragment_in_multiple_operations() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(NoUnusedVariables::new()));
    let errors = test_operation_with_schema(
        "query Foo($a: String) {
      ...FragA
    }
    query Bar($b: String) {
      ...FragB
    }
    fragment FragA on Type {
      field(a: $a)
    }
    fragment FragB on Type {
      field(b: $b)
    }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn variables_used_by_recursive_fragment() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(NoUnusedVariables::new()));
    let errors = test_operation_with_schema(
        "query Foo($a: String) {
      ...FragA
    }
    fragment FragA on Type {
      field(a: $a) {
        ...FragA
      }
    }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn variables_not_used() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(NoUnusedVariables::new()));
    let errors = test_operation_with_schema(
        "query ($a: String, $b: String, $c: String) {
          field(a: $a, b: $b)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);

    assert_eq!(messages.len(), 1);
    assert!(messages.contains(&&"Variable \"$c\" is never used.".to_owned()));
}

#[test]
fn multiple_variables_not_used() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(NoUnusedVariables::new()));
    let errors = test_operation_with_schema(
        "query Foo($a: String, $b: String, $c: String) {
          field(b: $b)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);

    assert_eq!(messages.len(), 2);
    assert!(messages.contains(&&"Variable \"$a\" is never used in operation \"Foo\".".to_owned()));
    assert!(messages.contains(&&"Variable \"$c\" is never used in operation \"Foo\".".to_owned()));
}

#[test]
fn variables_not_used_in_fragments() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(NoUnusedVariables::new()));
    let errors = test_operation_with_schema(
        "query Foo($a: String, $b: String, $c: String) {
          ...FragA
        }
        fragment FragA on Type {
          field(a: $a) {
            ...FragB
          }
        }
        fragment FragB on Type {
          field(b: $b) {
            ...FragC
          }
        }
        fragment FragC on Type {
          field
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);

    assert_eq!(messages.len(), 1);
    assert!(messages.contains(&&"Variable \"$c\" is never used in operation \"Foo\".".to_owned()));
}

#[test]
fn multiple_variables_not_used_in_fragments() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(NoUnusedVariables::new()));
    let errors = test_operation_with_schema(
        "query Foo($a: String, $b: String, $c: String) {
          ...FragA
        }
        fragment FragA on Type {
          field {
            ...FragB
          }
        }
        fragment FragB on Type {
          field(b: $b) {
            ...FragC
          }
        }
        fragment FragC on Type {
          field
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);

    assert_eq!(messages.len(), 2);
    assert!(messages.contains(&&"Variable \"$a\" is never used in operation \"Foo\".".to_owned()));
    assert!(messages.contains(&&"Variable \"$c\" is never used in operation \"Foo\".".to_owned()));
}

#[test]
fn variables_not_used_by_unreferences_fragment() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(NoUnusedVariables::new()));
    let errors = test_operation_with_schema(
        "query Foo($b: String) {
          ...FragA
        }
        fragment FragA on Type {
          field(a: $a)
        }
        fragment FragB on Type {
          field(b: $b)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);

    assert_eq!(messages.len(), 1);
    assert!(messages.contains(&&"Variable \"$b\" is never used in operation \"Foo\".".to_owned()));
}

#[test]
fn variables_not_used_by_fragment_used_by_other_operation() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(NoUnusedVariables::new()));
    let errors = test_operation_with_schema(
        "query Foo($b: String) {
          ...FragA
        }
        query Bar($a: String) {
          ...FragB
        }
        fragment FragA on Type {
          field(a: $a)
        }
        fragment FragB on Type {
          field(b: $b)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);

    assert_eq!(messages.len(), 2);
    assert!(messages.contains(&&"Variable \"$b\" is never used in operation \"Foo\".".to_owned()));
    assert!(messages.contains(&&"Variable \"$a\" is never used in operation \"Bar\".".to_owned()));
}

#[test]
fn should_also_check_directives_usage() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(NoUnusedVariables::new()));
    let errors = test_operation_with_schema(
        "query foo($skip: Boolean!) {
          field @skip(if: $skip)
        }
        ",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn nested_variable_should_work_as_well() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(NoUnusedVariables::new()));
    let errors = test_operation_with_schema(
        "query foo($t: Boolean!) {
          field(boop: { test: $t})
        }
        ",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}
