use crate::validation::{rules::ValidationRule, validate::ValidationPlan};

use super::{
    FieldsOnCorrectType, FragmentsOnCompositeTypes, KnownArgumentNames, KnownDirectives,
    KnownFragmentNames, KnownTypeNames, LeafFieldSelections, LoneAnonymousOperation,
    NoFragmentsCycle, NoUndefinedVariables, NoUnusedFragments, NoUnusedVariables,
    OverlappingFieldsCanBeMerged, PossibleFragmentSpreads, ProvidedRequiredArguments,
    SingleFieldSubscriptions, UniqueArgumentNames, UniqueDirectivesPerLocation,
    UniqueFragmentNames, UniqueOperationNames, UniqueVariableNames, ValuesOfCorrectType,
    VariablesAreInputTypes, VariablesInAllowedPosition,
};

pub fn default_rules_validation_plan() -> ValidationPlan {
    let rules: Vec<Box<dyn ValidationRule>> = vec![
        Box::new(UniqueOperationNames::new()),
        Box::new(LoneAnonymousOperation::new()),
        Box::new(SingleFieldSubscriptions::new()),
        Box::new(KnownTypeNames::new()),
        Box::new(FragmentsOnCompositeTypes::new()),
        Box::new(VariablesAreInputTypes::new()),
        Box::new(LeafFieldSelections::new()),
        Box::new(FieldsOnCorrectType::new()),
        Box::new(UniqueFragmentNames::new()),
        Box::new(KnownFragmentNames::new()),
        Box::new(NoUnusedFragments::new()),
        Box::new(OverlappingFieldsCanBeMerged::new()),
        Box::new(NoFragmentsCycle::new()),
        Box::new(PossibleFragmentSpreads::new()),
        Box::new(NoUnusedVariables::new()),
        Box::new(NoUndefinedVariables::new()),
        Box::new(KnownArgumentNames::new()),
        Box::new(UniqueArgumentNames::new()),
        Box::new(UniqueVariableNames::new()),
        Box::new(ProvidedRequiredArguments::new()),
        Box::new(KnownDirectives::new()),
        Box::new(VariablesInAllowedPosition::new()),
        Box::new(ValuesOfCorrectType::new()),
        Box::new(UniqueDirectivesPerLocation::new()),
    ];

    ValidationPlan::from(rules)
}
