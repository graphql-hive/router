use std::collections::hash_map::Entry;
use std::collections::HashMap;

use hive_router_query_planner::ast::{
    selection_item::SelectionItem,
    selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
};

use crate::execution::plan::CoerceVariablesPayload;
use crate::introspection::schema::SchemaMetadata;
use crate::response::graphql_error::GraphQLError;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OperationFilterError {
    #[error("Field `{field_name}` not found on type `{parent_type_name}` in schema")]
    FieldNotFound {
        parent_type_name: String,
        field_name: String,
    },
}

pub struct FieldInfo<'exec> {
    pub response_key: &'exec str,
    pub field_name: &'exec str,
    /// The type that declares the field
    pub parent_type_name: &'exec str,
    pub output_type_name: &'exec str,
    pub is_non_null: bool,
}

pub struct TypeConditionInfo<'exec> {
    /// `User` in `... on User { ... }`
    pub type_condition: &'exec str,
    /// The type the fragment is written inside before it becomes more specific
    /// `Node` in `node { ... on User { ... } }`
    pub parent_type_name: &'exec str,
}

impl Selection<'_> {
    pub const fn keep(&self) -> FilterDecision {
        FilterDecision::Keep
    }

    pub fn reject(&self, error: GraphQLError) -> FilterDecision {
        FilterDecision::Reject { error }
    }
}

pub enum Selection<'exec> {
    Field(FieldInfo<'exec>),
    Fragment(TypeConditionInfo<'exec>),
}

#[derive(Clone)]
pub enum FilterDecision {
    Keep,
    Reject { error: GraphQLError },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NullPropagation {
    /// Parent is unaffected.
    None,
    /// A non-null child was removed, the parent must be nullified too
    Propagate,
}

impl NullPropagation {
    fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::Propagate, _) | (_, Self::Propagate) => Self::Propagate,
            _ => Self::None,
        }
    }

    fn is_propagate(self) -> bool {
        matches!(self, Self::Propagate)
    }
}

#[derive(Default)]
pub struct OperationFilterOutput<'exec> {
    /// Errors to report to the client, one per denied field or fragment.
    pub errors: Vec<GraphQLError>,
    pub rejected_paths: Vec<Vec<&'exec str>>,
}

impl<'exec> OperationFilterOutput<'exec> {
    pub fn has_changes(&self) -> bool {
        !self.errors.is_empty() || !self.rejected_paths.is_empty()
    }
}

pub struct OperationFilter<'exec> {
    schema_metadata: &'exec SchemaMetadata,
}

impl<'exec> OperationFilter<'exec> {
    pub fn new(schema_metadata: &'exec SchemaMetadata) -> Self {
        Self { schema_metadata }
    }

    pub fn filter(
        &self,
        root_type_name: &'exec str,
        selection_set: &'exec SelectionSet,
        // We need variables to be able to evaluate `@skip`/`@include` directives,
        // and ignore selections that are not included.
        variable_payload: &'exec CoerceVariablesPayload,
        mut visitor: impl FnMut(&Selection<'exec>) -> FilterDecision,
    ) -> Result<OperationFilterOutput<'exec>, OperationFilterError> {
        let mut ctx = FilteringContext {
            schema_metadata: self.schema_metadata,
            variable_payload,
            decisions: HashMap::new(),
            path: Vec::new(),
            result: OperationFilterOutput::default(),
        };
        ctx.visit_selection_set(selection_set, root_type_name, &mut visitor)?;
        Ok(ctx.result)
    }
}

struct FilteringContext<'exec> {
    schema_metadata: &'exec SchemaMetadata,
    variable_payload: &'exec CoerceVariablesPayload,
    decisions: HashMap<(&'exec str, &'exec str), FilterDecision>,
    path: Vec<&'exec str>,
    result: OperationFilterOutput<'exec>,
}

impl<'exec> FilteringContext<'exec> {
    fn visit_selection_set(
        &mut self,
        selection_set: &'exec SelectionSet,
        parent_type_name: &'exec str,
        visitor: &mut impl FnMut(&Selection<'exec>) -> FilterDecision,
    ) -> Result<NullPropagation, OperationFilterError> {
        let mut null_propagation = NullPropagation::None;
        for item in &selection_set.items {
            match item {
                SelectionItem::Field(field) => {
                    if self.is_ignored(&field.skip_if, &field.include_if) {
                        continue;
                    }
                    null_propagation =
                        null_propagation.or(self.visit_field(field, parent_type_name, visitor)?);
                }
                SelectionItem::InlineFragment(fragment) => {
                    if self.is_ignored(&fragment.skip_if, &fragment.include_if) {
                        continue;
                    }
                    null_propagation = null_propagation.or(self.visit_inline_fragment(
                        fragment,
                        parent_type_name,
                        visitor,
                    )?);
                }
                SelectionItem::FragmentSpread(_) => {
                    // Fragment spreads are expected to have been expanded
                    // before this traversal runs.
                }
            }
        }
        Ok(null_propagation)
    }

    fn visit_field(
        &mut self,
        field: &'exec FieldSelection,
        parent_type_name: &'exec str,
        visitor: &mut impl FnMut(&Selection<'exec>) -> FilterDecision,
    ) -> Result<NullPropagation, OperationFilterError> {
        let field_name: &'exec str = field.name.as_str();
        let response_key: &'exec str = field.alias.as_deref().unwrap_or(field_name);

        if field_name == "__typename" {
            return Ok(NullPropagation::None);
        }

        let field_type_info = self
            .schema_metadata
            .get_type_fields(parent_type_name)
            .and_then(|fields| fields.get(field_name))
            .ok_or_else(|| OperationFilterError::FieldNotFound {
                parent_type_name: parent_type_name.to_string(),
                field_name: field_name.to_string(),
            })?;
        let output_type_name = field_type_info.output_type_name.as_str();
        let is_non_null = field_type_info.nullability.is_non_null();

        let decision = match self.decisions.entry((parent_type_name, field_name)) {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => {
                let decision = visitor(&Selection::Field(FieldInfo {
                    response_key,
                    field_name,
                    parent_type_name,
                    output_type_name,
                    is_non_null,
                }));
                entry.insert(decision.clone());
                decision
            }
        };

        self.path.push(response_key);
        let rejected = match decision {
            FilterDecision::Reject { error } => {
                self.record_rejected_path();
                self.record_error(error);
                true
            }
            FilterDecision::Keep => {
                let propagation = if field.selections.is_empty() {
                    NullPropagation::None
                } else {
                    self.visit_selection_set(&field.selections, output_type_name, visitor)?
                };
                if propagation.is_propagate() {
                    self.record_rejected_path();
                }
                propagation.is_propagate()
            }
        };
        self.path.pop();

        Ok(if rejected && is_non_null {
            NullPropagation::Propagate
        } else {
            NullPropagation::None
        })
    }

    fn visit_inline_fragment(
        &mut self,
        fragment: &'exec InlineFragmentSelection,
        parent_type_name: &'exec str,
        visitor: &mut impl FnMut(&Selection<'exec>) -> FilterDecision,
    ) -> Result<NullPropagation, OperationFilterError> {
        let decision = visitor(&Selection::Fragment(TypeConditionInfo {
            type_condition: &fragment.type_condition,
            parent_type_name,
        }));

        match decision {
            FilterDecision::Keep => {
                self.visit_selection_set(&fragment.selections, &fragment.type_condition, visitor)
            }
            FilterDecision::Reject { error } => {
                self.record_error(error);
                self.collect_all_field_paths(&fragment.selections);
                Ok(NullPropagation::None)
            }
        }
    }

    fn collect_all_field_paths(&mut self, selection_set: &'exec SelectionSet) {
        for item in &selection_set.items {
            match item {
                SelectionItem::Field(field) => {
                    let response_key: &'exec str =
                        field.alias.as_deref().unwrap_or(field.name.as_str());
                    self.path.push(response_key);
                    self.record_rejected_path();
                    self.path.pop();
                }
                SelectionItem::InlineFragment(fragment) => {
                    self.collect_all_field_paths(&fragment.selections);
                }
                SelectionItem::FragmentSpread(_) => {}
            }
        }
    }

    fn record_rejected_path(&mut self) {
        self.result.rejected_paths.push(self.path.clone());
    }

    fn record_error(&mut self, mut error: GraphQLError) {
        if error.extensions.affected_path.as_ref().is_none() {
            error = error.add_affected_path(self.path.join("."));
        }
        self.result.errors.push(error);
    }

    fn is_ignored(&self, skip_if: &Option<String>, include_if: &Option<String>) -> bool {
        if let Some(variable_name) = skip_if {
            if self.variable_payload.variable_equals_true(variable_name) {
                return true;
            }
        }

        if let Some(variable_name) = include_if {
            if !self.variable_payload.variable_equals_true(variable_name) {
                return true;
            }
        }

        false
    }
}
