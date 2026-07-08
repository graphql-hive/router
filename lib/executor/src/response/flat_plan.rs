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
}

#[derive(Debug, Clone)]
pub struct FieldWritePlan {
    pub source_key: Box<str>,
    pub response_key: Box<str>,
    pub response_key_id: ResponseKeyId,
    pub value: ValueWritePlan,
    pub nullability: FieldNullability,
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
            }
        })
        .collect();

    let root_object = ObjectWritePlan {
        response_key: "data".into(),
        nullability: FieldNullability::leaf(true),
        fields: root_fields,
        object_type_name: "Query".into(),
    };

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
                        }
                    })
                    .collect();

                let object = ObjectWritePlan {
                    response_key: plan.response_key.clone().into_boxed_str(),
                    nullability: plan.nullability.clone(),
                    fields,
                    object_type_name: plan.output_type_name.clone().into_boxed_str(),
                };

                ValueWritePlan::Object(object)
            }
        }
    }
}
