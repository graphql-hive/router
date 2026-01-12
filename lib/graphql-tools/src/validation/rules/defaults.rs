use crate::validation::validate::ValidationPlan;

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
    let mut plan = ValidationPlan { rules: vec![] };

    plan.add_rule(Box::new(UniqueOperationNames::new()));
    plan.add_rule(Box::new(LoneAnonymousOperation::new()));
    plan.add_rule(Box::new(SingleFieldSubscriptions::new()));
    plan.add_rule(Box::new(KnownTypeNames::new()));
    plan.add_rule(Box::new(FragmentsOnCompositeTypes::new()));
    plan.add_rule(Box::new(VariablesAreInputTypes::new()));
    plan.add_rule(Box::new(LeafFieldSelections::new()));
    plan.add_rule(Box::new(FieldsOnCorrectType::new()));
    plan.add_rule(Box::new(UniqueFragmentNames::new()));
    plan.add_rule(Box::new(KnownFragmentNames::new()));
    plan.add_rule(Box::new(NoUnusedFragments::new()));
    plan.add_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    plan.add_rule(Box::new(NoFragmentsCycle::new()));
    plan.add_rule(Box::new(PossibleFragmentSpreads::new()));
    plan.add_rule(Box::new(NoUnusedVariables::new()));
    plan.add_rule(Box::new(NoUndefinedVariables::new()));
    plan.add_rule(Box::new(KnownArgumentNames::new()));
    plan.add_rule(Box::new(UniqueArgumentNames::new()));
    plan.add_rule(Box::new(UniqueVariableNames::new()));
    plan.add_rule(Box::new(ProvidedRequiredArguments::new()));
    plan.add_rule(Box::new(KnownDirectives::new()));
    plan.add_rule(Box::new(VariablesInAllowedPosition::new()));
    plan.add_rule(Box::new(ValuesOfCorrectType::new()));
    plan.add_rule(Box::new(UniqueDirectivesPerLocation::new()));

    plan
}
