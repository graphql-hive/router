use super::{
    rules::ValidationRule,
    utils::{ValidationError, ValidationErrorContext},
};

use crate::{
    ast::OperationVisitorContext,
    static_graphql::{query, schema},
};

pub struct ValidationPlan {
    pub rules: Vec<Box<dyn ValidationRule>>,
}

impl ValidationPlan {
    pub fn new() -> Self {
        Self { rules: vec![] }
    }

    pub fn from(rules: Vec<Box<dyn ValidationRule>>) -> Self {
        Self { rules }
    }

    pub fn add_rule(&mut self, rule: Box<dyn ValidationRule>) {
        self.rules.push(rule);
    }
}

impl Default for ValidationPlan {
    fn default() -> Self {
        Self::new()
    }
}

pub fn validate<'a>(
    schema: &'a schema::Document,
    operation: &'a query::Document,
    validation_plan: &'a ValidationPlan,
) -> Vec<ValidationError> {
    let mut error_collector = ValidationErrorContext::new();
    let mut validation_context = OperationVisitorContext::new(operation, schema);

    validation_plan
        .rules
        .iter()
        .for_each(|rule| rule.validate(&mut validation_context, &mut error_collector));

    error_collector.errors
}

#[test]
fn cyclic_fragment_should_never_loop() {
    use crate::validation::rules::default_rules_validation_plan;
    use crate::validation::test_utils::*;

    let mut default_plan = default_rules_validation_plan();
    let errors = test_operation_with_schema(
        "
        {
          dog {
            nickname
            ...bark
            ...parents
          }
        }
        
        fragment bark on Dog {
          barkVolume
          ...parents
        }
        
        fragment parents on Dog {
          mother {
            ...bark
          }
        }
        
    ",
        TEST_SCHEMA,
        &mut default_plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(errors[0].error_code, "NoFragmentsCycle");
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Cannot spread fragment \"bark\" within itself via \"parents\"."]
    )
}

#[test]
fn simple_self_reference_fragment_should_not_loop() {
    use crate::validation::rules::default_rules_validation_plan;
    use crate::validation::test_utils::*;

    let mut default_plan = default_rules_validation_plan();
    let errors = test_operation_with_schema(
        "
        query dog {
          dog {
            ...DogFields
          }
        }
        
        fragment DogFields on Dog {
          mother {
            ...DogFields
          }
          father {
            ...DogFields
          }
        }
    ",
        TEST_SCHEMA,
        &mut default_plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages,
        vec![
            "Cannot spread fragment \"DogFields\" within itself.",
            "Cannot spread fragment \"DogFields\" within itself."
        ]
    )
}

#[test]
fn fragment_loop_through_multiple_frags() {
    use crate::validation::rules::default_rules_validation_plan;
    use crate::validation::test_utils::*;

    let mut default_plan = default_rules_validation_plan();
    let errors = test_operation_with_schema(
        "
        query dog {
          dog {
            ...DogFields1
          }
        }
        
        fragment DogFields1 on Dog {
          barks
          ...DogFields2
        }

        fragment DogFields2 on Dog {
          barkVolume
          ...DogFields3
        }

        fragment DogFields3 on Dog {
          name
          ...DogFields1
        }
    ",
        TEST_SCHEMA,
        &mut default_plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec![
      "Cannot spread fragment \"DogFields1\" within itself via \"DogFields2\", \"DogFields3\"."
    ]
    )
}
