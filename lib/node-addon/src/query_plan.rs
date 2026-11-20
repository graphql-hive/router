use std::{collections::HashSet, sync::Arc};

use graphql_tools::parser::{query, schema};
use hive_router::{
    executor::{
        introspection::schema::SchemaMetadata,
        projection::plan::{
            Condition, ConditionId, FieldRecord, Guard, ProjectionPlan, ShapeFlags, TypeGuardId,
        },
    },
    pipeline::{
        progressive_override::{RequestOverrideContext, StableOverrideContext},
        query_plan::calculate_cache_key,
    },
    query_planner::{
        ast::normalization::{error::NormalizationError, normalize_operation},
        graph::{PlannerOverrideContext, PERCENTAGE_SCALE_FACTOR},
        planner::{
            plan_nodes::planning::{
                BatchFetchNode, DeferDependency, FetchContext, FetchId, FetchNode, FlattenNodePath,
                PlanNode, QueryPlan,
            },
            Planner, PlannerError,
        },
        utils::{cancellation::CancellationToken, parsing::safe_parse_operation},
    },
};
use napi::{Task, Unknown};
use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum QueryPlanError {
    #[error("Failed to parse supergraph SDL: {0}")]
    SchemaParse(#[from] schema::ParseError),
    #[error("Failed to parse query: {0}")]
    QueryParse(#[from] query::ParseError),
    #[error("Failed to normalize operation: {0}")]
    Normalization(#[from] NormalizationError),
    #[error("Failed to plan query: {0}")]
    Planner(#[from] PlannerError),
}

impl From<QueryPlanError> for napi::Error {
    fn from(value: QueryPlanError) -> Self {
        napi::Error::from_reason(value.to_string())
    }
}

pub fn compute_cache_key(
    planner: &Planner,
    query: &str,
    operation_name: Option<&str>,
    active_labels: HashSet<String>,
    percentage_value: f64,
) -> core::result::Result<String, QueryPlanError> {
    let parsed_operation = safe_parse_operation(query)?;

    let normalized_operation =
        normalize_operation(&planner.supergraph, &parsed_operation, operation_name)?;

    let request_override_context = RequestOverrideContext {
        active_flags: active_labels,
        percentage_value: (percentage_value * (PERCENTAGE_SCALE_FACTOR as f64)) as u64,
    };

    let stable_override_context =
        StableOverrideContext::new(&planner.supergraph, &request_override_context);

    let operation_hash = normalized_operation.operation.hash();
    let cache_key = calculate_cache_key(operation_hash, &stable_override_context);

    Ok(cache_key.to_string())
}

pub fn query_plan(
    planner: &Planner,
    metadata: &SchemaMetadata,
    query: &str,
    operation_name: Option<&str>,
    active_labels: HashSet<String>,
    percentage_value: f64,
    cancellation_token: &CancellationToken,
) -> core::result::Result<JsQueryPlan, QueryPlanError> {
    let parsed_operation = safe_parse_operation(query)?;
    let normalized_operation =
        normalize_operation(&planner.supergraph, &parsed_operation, operation_name)?;

    let request_override_context = PlannerOverrideContext::new(
        active_labels,
        (percentage_value * (PERCENTAGE_SCALE_FACTOR as f64)) as u64,
    );

    let plan = planner.plan_from_normalized_operation(
        &normalized_operation.operation,
        request_override_context,
        cancellation_token,
    )?;

    // after planning, it fails for an operation without a root type, `from_operation` panics
    let (root_type, projection) =
        ProjectionPlan::from_operation(&normalized_operation.operation, metadata);

    Ok(JsQueryPlan {
        plan,
        root_type: root_type.to_string(),
        projection,
    })
}

/// The plan as the addon gives it to JS. Serializes like the router's own plan, but every
/// fetch also has its `id` and `dependsOn`, and every batch fetch its `completes`, so JS can
/// start each fetch once the fetches it depends on are done. A fetch with context variables
/// also has its `maxContextGroups`, unless it has no limit. The router's own serialization
/// skips them, so the addon serializes the nodes holding fetches itself. It also has the
/// `projection` the router shapes the operation's response with.
pub struct JsQueryPlan {
    plan: QueryPlan,
    root_type: String,
    projection: ProjectionPlan,
}

impl Serialize for JsQueryPlan {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Plan<'a> {
            kind: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            node: Option<JsPlanNode<'a>>,
            projection: JsProjection<'a>,
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct JsProjection<'a> {
            root_type: &'a str,
            fields: JsProjectionFields<'a>,
        }
        Plan {
            kind: self.plan.kind,
            node: self.plan.node().map(JsPlanNode),
            projection: JsProjection {
                root_type: &self.root_type,
                fields: JsProjectionFields(&self.projection, self.projection.root_fields()),
            },
        }
        .serialize(serializer)
    }
}

struct JsProjectionFields<'a>(&'a ProjectionPlan, &'a [FieldRecord]);

impl Serialize for JsProjectionFields<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let Self(plan, fields) = *self;
        serializer.collect_seq(fields.iter().map(|field| JsProjectionField(plan, field)))
    }
}

struct JsProjectionField<'a>(&'a ProjectionPlan, &'a FieldRecord);

impl Serialize for JsProjectionField<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Field<'a> {
            name: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            alias: Option<&'a str>,
            #[serde(skip_serializing_if = "std::ops::Not::not")]
            typename: bool,
            #[serde(skip_serializing_if = "std::ops::Not::not")]
            non_null: bool,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            item_non_null: Vec<bool>,
            #[serde(skip_serializing_if = "Option::is_none")]
            parent_types: Option<JsTypeGuard<'a>>,
            #[serde(skip_serializing_if = "Option::is_none")]
            condition: Option<JsProjectionCondition<'a>>,
            #[serde(skip_serializing_if = "Option::is_none")]
            selections: Option<JsProjectionFields<'a>>,
        }
        let Self(plan, field) = *self;
        let name = plan.field_name(field);
        let response_key = plan.response_key(field);
        // a list level's item comes right after it in the shape
        let item_non_null = plan
            .shape(field.nullability())
            .windows(2)
            .take_while(|levels| ShapeFlags::from_bits_retain(levels[0]).contains(ShapeFlags::LIST))
            .map(|levels| ShapeFlags::from_bits_retain(levels[1]).contains(ShapeFlags::NON_NULL))
            .collect();
        // a field the router forces to `null` comes from authorization rewrites, the addon has
        // none
        Field {
            name,
            alias: (response_key != name).then_some(response_key),
            typename: field.is_typename(),
            non_null: field.is_non_null(),
            item_non_null,
            parent_types: field.parent_guard.map(|guard| JsTypeGuard(plan, guard)),
            condition: field
                .condition
                .map(|condition| JsProjectionCondition(plan, condition)),
            selections: field
                .has_children()
                .then(|| JsProjectionFields(plan, plan.fields(field.children))),
        }
        .serialize(serializer)
    }
}

struct JsTypeGuard<'a>(&'a ProjectionPlan, TypeGuardId);

impl Serialize for JsTypeGuard<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let Self(plan, guard) = *self;
        match plan.guard(guard) {
            Guard::Exact(name) => serializer.collect_seq([plan.text(name)]),
            Guard::Set(set) => {
                serializer.collect_seq(plan.members(set).iter().map(|name| plan.text(*name)))
            }
        }
    }
}

struct JsProjectionCondition<'a>(&'a ProjectionPlan, ConditionId);

impl Serialize for JsProjectionCondition<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        enum Tagged<'a> {
            Include(&'a str),
            Skip(&'a str),
            ParentType(JsTypeGuard<'a>),
            FieldType(JsTypeGuard<'a>),
            EnumValues(Vec<&'a str>),
            And([JsProjectionCondition<'a>; 2]),
            Or([JsProjectionCondition<'a>; 2]),
        }
        let Self(plan, condition) = *self;
        match plan.condition(condition) {
            Condition::Include(variable) => Tagged::Include(plan.text(variable)),
            Condition::Skip(variable) => Tagged::Skip(plan.text(variable)),
            Condition::ParentType(guard) => Tagged::ParentType(JsTypeGuard(plan, guard)),
            Condition::FieldType(guard) => Tagged::FieldType(JsTypeGuard(plan, guard)),
            Condition::EnumValues(set) => Tagged::EnumValues(
                plan.members(set)
                    .iter()
                    .map(|value| plan.text(*value))
                    .collect(),
            ),
            Condition::And(left, right) => Tagged::And([
                JsProjectionCondition(plan, left),
                JsProjectionCondition(plan, right),
            ]),
            Condition::Or(left, right) => Tagged::Or([
                JsProjectionCondition(plan, left),
                JsProjectionCondition(plan, right),
            ]),
        }
        .serialize(serializer)
    }
}

struct JsPlanNode<'a>(&'a PlanNode);

struct JsPlanNodes<'a>(&'a [PlanNode]);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsFetchNode<'a> {
    #[serde(flatten)]
    fetch: &'a FetchNode,
    id: FetchId,
    depends_on: &'a [FetchId],
    #[serde(skip_serializing_if = "Option::is_none")]
    max_context_groups: Option<u32>,
}

impl<'a> From<&'a FetchNode> for JsFetchNode<'a> {
    fn from(fetch: &'a FetchNode) -> Self {
        let contextual = !FetchContext::variables(&fetch.context).is_empty();
        Self {
            fetch,
            id: fetch.id,
            depends_on: &fetch.depends_on,
            max_context_groups: contextual
                .then(|| max_context_groups(fetch.operation.operation.max_context_groups()))
                .flatten(),
        }
    }
}

/// The most context groups one request may carry, `None` when there's no limit. Configured alias
/// limits fit in a `u32`, so only no limit gives more: `usize::MAX`, or that divided by the
/// aliases of a batch.
fn max_context_groups(groups: usize) -> Option<u32> {
    u32::try_from(groups).ok()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsBatchFetchNode<'a> {
    #[serde(flatten)]
    batch: &'a BatchFetchNode,
    id: FetchId,
    depends_on: &'a [FetchId],
    completes: &'a [FetchId],
    #[serde(skip_serializing_if = "Option::is_none")]
    max_context_groups: Option<u32>,
}

#[derive(Serialize)]
struct JsDeferPrimary<'a> {
    subselection: &'a Option<String>,
    node: Option<JsPlanNode<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsDeferredNode<'a> {
    depends: &'a [DeferDependency],
    #[serde(skip_serializing_if = "Option::is_none")]
    label: &'a Option<String>,
    query_path: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    subselection: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    node: Option<JsPlanNode<'a>>,
}

// same shape as the router's `PlanNode`, only with ids on the fetches
#[derive(Serialize)]
#[serde(tag = "kind")]
enum JsTaggedPlanNode<'a> {
    Fetch(JsFetchNode<'a>),
    BatchFetch(JsBatchFetchNode<'a>),
    Sequence {
        nodes: JsPlanNodes<'a>,
    },
    Parallel {
        nodes: JsPlanNodes<'a>,
    },
    Flatten {
        path: &'a FlattenNodePath,
        node: JsPlanNode<'a>,
    },
    Subscription {
        primary: JsFetchNode<'a>,
    },
    Defer {
        primary: JsDeferPrimary<'a>,
        deferred: Vec<JsDeferredNode<'a>>,
    },
}

impl Serialize for JsPlanNode<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            PlanNode::Fetch(fetch) => JsTaggedPlanNode::Fetch(fetch.as_ref().into()),
            PlanNode::BatchFetch(batch) => JsTaggedPlanNode::BatchFetch(JsBatchFetchNode {
                batch,
                id: batch.id,
                depends_on: &batch.depends_on,
                completes: &batch.completes,
                max_context_groups: batch
                    .entity_batch
                    .aliases
                    .iter()
                    .any(|alias| !FetchContext::variables(&alias.context).is_empty())
                    .then(|| max_context_groups(batch.operation.operation.max_context_groups()))
                    .flatten(),
            }),
            PlanNode::Sequence(sequence) => JsTaggedPlanNode::Sequence {
                nodes: JsPlanNodes(&sequence.nodes),
            },
            PlanNode::Parallel(parallel) => JsTaggedPlanNode::Parallel {
                nodes: JsPlanNodes(&parallel.nodes),
            },
            PlanNode::Flatten(flatten) => JsTaggedPlanNode::Flatten {
                path: &flatten.path,
                node: JsPlanNode(&flatten.node),
            },
            PlanNode::Subscription(subscription) => JsTaggedPlanNode::Subscription {
                primary: (&subscription.primary).into(),
            },
            PlanNode::Defer(defer) => JsTaggedPlanNode::Defer {
                primary: JsDeferPrimary {
                    subselection: &defer.primary.subselection,
                    node: defer.primary.node.as_deref().map(JsPlanNode),
                },
                deferred: defer
                    .deferred
                    .iter()
                    .map(|deferred| JsDeferredNode {
                        depends: &deferred.depends,
                        label: &deferred.label,
                        query_path: &deferred.query_path,
                        subselection: &deferred.subselection,
                        node: deferred.node.as_deref().map(JsPlanNode),
                    })
                    .collect(),
            },
        }
        .serialize(serializer)
    }
}

impl Serialize for JsPlanNodes<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter().map(JsPlanNode))
    }
}

pub struct QueryPlanTask<'a> {
    pub planner: &'a Planner,
    pub metadata: &'a SchemaMetadata,
    pub query: String,
    pub operation_name: Option<String>,
    pub active_labels: HashSet<String>,
    pub percentage_value: f64,
    pub cancellation_token: Arc<CancellationToken>,
}

impl<'a> Task for QueryPlanTask<'a> {
    type Output = JsQueryPlan;
    type JsValue = Unknown<'a>;

    fn compute(&mut self) -> Result<Self::Output, napi::Error> {
        Ok(query_plan(
            self.planner,
            self.metadata,
            &self.query,
            self.operation_name.as_deref(),
            std::mem::take(&mut self.active_labels),
            self.percentage_value,
            &self.cancellation_token,
        )?)
    }

    fn resolve(
        &mut self,
        env: napi::Env,
        output: Self::Output,
    ) -> Result<Self::JsValue, napi::Error> {
        env.to_js_value(&output)
    }
}
