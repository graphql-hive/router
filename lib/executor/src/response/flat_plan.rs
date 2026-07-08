use std::sync::Arc;

use crate::introspection::schema::FieldNullability;
use crate::projection::plan::ProjectionValueSource;
use crate::response::flat_store::{FlatValueId, ResponseKeyId, ResponseKeys};

/// Compiled response write plan: maps subgraph fetch operations to flat store targets.
#[derive(Debug, Clone)]
pub struct ResponseWritePlan {
    pub fetches: ResponseWritePlanRegistry,
}

impl ResponseWritePlan {
    pub fn empty() -> Self {
        Self {
            fetches: ResponseWritePlanRegistry::default(),
        }
    }
}

/// Registry of fetch write plans, keyed by fetch node id.
#[derive(Debug, Clone, Default)]
pub struct ResponseWritePlanRegistry {
    pub plans_by_fetch_id: Vec<(i64, Arc<FetchWritePlan>)>,
}

impl ResponseWritePlanRegistry {
    pub fn get(&self, fetch_id: i64) -> Option<&Arc<FetchWritePlan>> {
        self.plans_by_fetch_id
            .iter()
            .find_map(|(id, plan)| (*id == fetch_id).then_some(plan))
    }
}

/// Compiled plan for a single subgraph fetch.
#[derive(Debug, Clone)]
pub struct FetchWritePlan {
    pub fetch_id: i64,
    pub data: ValueWritePlan,
    pub target: FetchTarget,
    pub keys: Arc<ResponseKeys>,
}

/// What part of the response this fetch targets.
#[derive(Debug, Clone)]
pub enum FetchTarget {
    Root,
}

/// A compiled plan for writing a GraphQL value into the flat store.
#[derive(Debug, Clone)]
pub enum ValueWritePlan {
    Leaf(LeafWritePlan),
    Object(ObjectWritePlan),
    List(ListWritePlan),
    Skip,
}

impl ValueWritePlan {
    /// Estimate how many flat values this plan subtree will contribute.
    /// List items are unknown at plan time, so we use a modest default.
    pub fn value_count_hint(&self) -> usize {
        match self {
            ValueWritePlan::Leaf(_) => 1,
            ValueWritePlan::Skip => 0,
            ValueWritePlan::Object(obj) => {
                1 + obj
                    .fields
                    .iter()
                    .map(|f| f.value.value_count_hint())
                    .sum::<usize>()
            }
            ValueWritePlan::List(list) => {
                let item_per_list = 16;
                1 + item_per_list + list.item.value_count_hint() * item_per_list
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct LeafWritePlan {
    pub response_key: Box<str>,
    pub nullability: FieldNullability,
    pub custom_scalar: bool,
}

#[derive(Debug, Clone)]
pub struct ObjectWritePlan {
    pub response_key: Box<str>,
    pub nullability: FieldNullability,
    pub fields: Vec<FieldWritePlan>,
    pub object_type_name: Box<str>,
    source_key_index: Box<[(Box<str>, usize)]>,
}

impl ObjectWritePlan {
    pub fn new(
        response_key: Box<str>,
        nullability: FieldNullability,
        fields: Vec<FieldWritePlan>,
        object_type_name: Box<str>,
    ) -> Self {
        let mut source_key_index: Vec<_> = fields
            .iter()
            .enumerate()
            .map(|(index, field)| (field.source_key.clone(), index))
            .collect();
        source_key_index.sort_unstable_by(|left, right| {
            left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1))
        });

        Self {
            response_key,
            nullability,
            fields,
            object_type_name,
            source_key_index: source_key_index.into_boxed_slice(),
        }
    }

    pub fn field_index_for_source_key(&self, key: &str) -> Option<usize> {
        let mut lookup_index = self
            .source_key_index
            .binary_search_by(|(source_key, _)| source_key.as_ref().cmp(key))
            .ok()?;

        // Preserve the old first-field-wins behavior if a malformed plan ever
        // contains duplicate source keys.
        while lookup_index > 0 && self.source_key_index[lookup_index - 1].0.as_ref() == key {
            lookup_index -= 1;
        }

        Some(self.source_key_index[lookup_index].1)
    }

    pub fn field_index_for_source_key_from(&self, key: &str, cursor: &mut usize) -> Option<usize> {
        let len = self.fields.len();
        if len == 0 {
            return None;
        }

        let index = (*cursor).min(len);
        if index < len && self.fields[index].source_key.as_ref() == key {
            *cursor = index + 1;
            return Some(index);
        }

        let index = self.field_index_for_source_key(key)?;
        *cursor = index + 1;
        Some(index)
    }
}

#[derive(Debug, Clone)]
pub struct FieldWritePlan {
    pub source_key: Box<str>,
    pub response_key: Box<str>,
    pub response_key_id: ResponseKeyId,
    pub value: ValueWritePlan,
    pub nullability: FieldNullability,
    pub output_key: Option<ResponseKeyId>,
    pub output_position: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct ListWritePlan {
    pub response_key: Box<str>,
    pub nullability: FieldNullability,
    pub item: Box<ValueWritePlan>,
}

/// Result of writing a decoded subgraph value into the flat store.
#[derive(Debug, Clone)]
pub struct WriteResult {
    pub value_id: FlatValueId,
    pub propagated_null: bool,
}

impl WriteResult {
    pub fn ok(value_id: FlatValueId) -> Self {
        Self {
            value_id,
            propagated_null: false,
        }
    }

    pub fn propagate_null() -> Self {
        Self {
            value_id: FlatValueId::new(0),
            propagated_null: true,
        }
    }
}

/// Compile a response write plan from the final projection plan and subgraph operations.
///
/// For now, this is a placeholder compiled from a single root fetch.
pub fn compile_response_write_plan(
    projection_plan: &[crate::projection::plan::FieldProjectionPlan],
    _schema_metadata: &crate::introspection::schema::SchemaMetadata,
) -> ResponseWritePlan {
    let mut fetches = ResponseWritePlanRegistry::default();
    let mut keys = ResponseKeys::default();

    let root_fields: Vec<FieldWritePlan> = projection_plan
        .iter()
        .map(|plan| {
            let response_key: Box<str> = plan.response_key.clone().into_boxed_str();
            let response_key_id = keys.intern(&response_key);
            FieldWritePlan {
                source_key: plan.field_name.clone().into_boxed_str(),
                response_key,
                response_key_id,
                value: compile_value_write_plan(plan, &mut keys),
                nullability: plan.nullability.clone(),
                output_key: None,
                output_position: None,
            }
        })
        .collect();

    let root_object = ObjectWritePlan::new(
        "data".into(),
        FieldNullability::leaf(true),
        root_fields,
        "Query".into(),
    );

    let root_plan = Arc::new(FetchWritePlan {
        fetch_id: 0,
        data: ValueWritePlan::Object(root_object),
        target: FetchTarget::Root,
        keys: Arc::new(keys),
    });

    fetches.plans_by_fetch_id.push((0, root_plan));

    ResponseWritePlan { fetches }
}

fn compile_value_write_plan(
    plan: &crate::projection::plan::FieldProjectionPlan,
    keys: &mut ResponseKeys,
) -> ValueWritePlan {
    match &plan.value {
        ProjectionValueSource::Null => ValueWritePlan::Skip,
        ProjectionValueSource::ResponseData { selections } => {
            let child_selections: Option<&Vec<_>> = selections.as_ref().map(|s| s.as_ref());
            if child_selections.is_none_or(|s| s.is_empty()) {
                ValueWritePlan::Leaf(LeafWritePlan {
                    response_key: plan.response_key.clone().into_boxed_str(),
                    nullability: plan.nullability.clone(),
                    custom_scalar: false,
                })
            } else {
                let fields: Vec<FieldWritePlan> = child_selections
                    .unwrap()
                    .iter()
                    .map(|child| {
                        let response_key: Box<str> = child.response_key.clone().into_boxed_str();
                        let response_key_id = keys.intern(&response_key);
                        FieldWritePlan {
                            source_key: child.field_name.clone().into_boxed_str(),
                            response_key,
                            response_key_id,
                            value: compile_value_write_plan(child, keys),
                            nullability: child.nullability.clone(),
                            output_key: None,
                            output_position: None,
                        }
                    })
                    .collect();

                let object = ObjectWritePlan::new(
                    plan.response_key.clone().into_boxed_str(),
                    plan.nullability.clone(),
                    fields,
                    plan.output_type_name.clone().into_boxed_str(),
                );

                ValueWritePlan::Object(object)
            }
        }
    }
}
