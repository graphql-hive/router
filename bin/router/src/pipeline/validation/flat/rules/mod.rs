//! The `graphql_tools` validation rules and the router's limits, on the flat document. Each
//! file mirrors the tree rule of the same name; the error code is what ties the two together.

use graphql_tools::validation::rules::ValidationOptions;

use super::visit::FlatRule;

pub mod fields_on_correct_type;
pub mod fragments_on_composite_types;
pub mod known_argument_names;
pub mod known_directives;
pub mod known_fragment_names;
pub mod known_type_names;
pub mod leaf_field_selections;
pub mod lone_anonymous_operation;
pub mod max_aliases;
pub mod max_depth;
pub mod max_directives;
pub mod no_fragments_cycle;
pub mod no_undefined_variables;
pub mod no_unused_fragments;
pub mod no_unused_variables;
pub mod overlapping_fields_can_be_merged;
pub mod possible_fragment_spreads;
pub mod provided_required_arguments;
pub mod single_field_subscriptions;
pub mod unique_argument_names;
pub mod unique_directives_per_location;
pub mod unique_fragment_names;
pub mod unique_input_field_names;
pub mod unique_operation_names;
pub mod unique_variable_names;
pub mod values_of_correct_type;
pub mod variables_are_input_types;
pub mod variables_in_allowed_position;

/// `default_rules_validation_plan_with_options`, flat, in the same order.
pub fn defaults(options: ValidationOptions) -> Vec<Box<dyn FlatRule>> {
    vec![
        Box::new(unique_operation_names::UniqueOperationNames),
        Box::new(lone_anonymous_operation::LoneAnonymousOperation),
        Box::new(single_field_subscriptions::SingleFieldSubscriptions),
        Box::new(known_type_names::KnownTypeNames),
        Box::new(fragments_on_composite_types::FragmentsOnCompositeTypes),
        Box::new(variables_are_input_types::VariablesAreInputTypes),
        Box::new(leaf_field_selections::LeafFieldSelections),
        Box::new(fields_on_correct_type::FieldsOnCorrectType),
        Box::new(unique_fragment_names::UniqueFragmentNames),
        Box::new(known_fragment_names::KnownFragmentNames),
        Box::new(no_unused_fragments::NoUnusedFragments),
        Box::new(overlapping_fields_can_be_merged::OverlappingFieldsCanBeMerged),
        Box::new(no_fragments_cycle::NoFragmentsCycle),
        Box::new(possible_fragment_spreads::PossibleFragmentSpreads),
        Box::new(no_unused_variables::NoUnusedVariables),
        Box::new(no_undefined_variables::NoUndefinedVariables),
        Box::new(known_argument_names::KnownArgumentNames),
        Box::new(unique_argument_names::UniqueArgumentNames),
        Box::new(unique_variable_names::UniqueVariableNames),
        Box::new(provided_required_arguments::ProvidedRequiredArguments),
        Box::new(known_directives::KnownDirectives),
        Box::new(variables_in_allowed_position::VariablesInAllowedPosition),
        Box::new(values_of_correct_type::ValuesOfCorrectType { options }),
        Box::new(unique_input_field_names::UniqueInputFieldNames),
        Box::new(unique_directives_per_location::UniqueDirectivesPerLocation),
    ]
}
