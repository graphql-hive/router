//! Runtime variable coercion tests: the spec's "Coercing Variable Values" (September 2025,
//! §6.1.2) and the input coercion rules of §3.
//!
//! The tests describe the target behavior, so some of them fail until the router's coercion is
//! fixed. Expected error messages use the graphql-js v17 wording (commit ee5ce41).
//!
//! Many test vectors and expected messages are ported from graphql-js
//! (https://github.com/graphql/graphql-js), Copyright (c) GraphQL Contributors, MIT License.
//! Each ported test names its upstream source in its doc comment.

mod enums;
mod harness;
mod input_objects;
mod lists;
mod oneof;
mod scalars;
mod variable_level;

#[test]
fn allow_null_values_for_nullable_scalar_types() {
    let schema_metadata = crate::executor::introspection::schema::SchemaMetadata::default();

    let scalars = vec!["String", "Int", "Float", "Boolean", "ID"];
    for scalar in scalars {
        let type_node = crate::executor::variables::TypeNode::Named(scalar.to_string());

        let value = sonic_rs::ValueRef::Null;

        let result = super::validate_runtime_value(value, &type_node, &schema_metadata);
        assert_eq!(result, Ok(()));
    }
}
#[test]
fn allow_null_values_for_nullable_list_types() {
    let schema_metadata = crate::executor::introspection::schema::SchemaMetadata::default();
    let type_node = crate::executor::variables::TypeNode::List(Box::new(
        crate::executor::variables::TypeNode::Named("String".to_string()),
    ));
    let value = sonic_rs::ValueRef::Null;
    let result = super::validate_runtime_value(value, &type_node, &schema_metadata);
    assert_eq!(result, Ok(()));
}
#[test]
fn allow_matching_non_list_values_for_list_types() {
    let schema_metadata = crate::executor::introspection::schema::SchemaMetadata::default();
    let type_node = crate::executor::variables::TypeNode::List(Box::new(
        crate::executor::variables::TypeNode::Named("String".to_string()),
    ));
    let value = sonic_rs::ValueRef::String("not a list");
    let result = super::validate_runtime_value(value, &type_node, &schema_metadata);
    assert_eq!(result, Ok(()));
}
#[test]
fn disallow_non_matching_non_list_values_for_list_types() {
    let schema_metadata = crate::executor::introspection::schema::SchemaMetadata::default();
    let type_node = crate::executor::variables::TypeNode::List(Box::new(
        crate::executor::variables::TypeNode::Named("String".to_string()),
    ));
    let value = sonic_rs::ValueRef::Number(sonic_rs::Number::from(123));
    let result = super::validate_runtime_value(value, &type_node, &schema_metadata);
    assert!(result.is_err());
}
