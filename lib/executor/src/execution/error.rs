use strum::IntoStaticStr;

use crate::{
    headers::errors::HeaderRuleRuntimeError,
    projection::error::ProjectionError,
    response::graphql_error::{GraphQLError, GraphQLErrorExtensions},
};

#[derive(thiserror::Error, Debug, Clone, IntoStaticStr)]
pub enum PlanExecutionErrorKind {
    #[error("Projection faiure: {0}")]
    #[strum(serialize = "PROJECTION_FAILURE")]
    ProjectionFailure(#[from] ProjectionError),

    #[error(transparent)]
    #[strum(serialize = "HEADER_PROPAGATION_FAILURE")]
    HeaderPropagation(#[from] HeaderRuleRuntimeError),

    #[error("{0}")]
    #[strum(serialize = "INTERNAL_ERROR")]
    Internal(String),
}

/// The central error type for all query plan execution failures.
///
/// This struct combines a specific `PlanExecutionErrorKind` with a
/// `PlanExecutionErrorContext` that holds shared, dynamic information
/// like the subgraph name or affected GraphQL path.
#[derive(thiserror::Error, Debug, Clone)]
#[error("{kind}")]
pub struct PlanExecutionError {
    #[source]
    kind: PlanExecutionErrorKind,
    context: PlanExecutionErrorContext,
}

#[derive(Debug, Clone)]
pub struct PlanExecutionErrorContext {
    subgraph_name: Option<String>,
    affected_path: Option<String>,
}

pub struct LazyPlanContext<SN, AP> {
    pub subgraph_name: SN,
    pub affected_path: AP,
}

impl PlanExecutionError {
    pub(crate) fn new<SN, AP>(
        kind: PlanExecutionErrorKind,
        lazy_context: LazyPlanContext<SN, AP>,
    ) -> Self
    where
        SN: FnOnce() -> Option<String>,
        AP: FnOnce() -> Option<String>,
    {
        Self {
            kind,
            context: PlanExecutionErrorContext {
                subgraph_name: (lazy_context.subgraph_name)(),
                affected_path: (lazy_context.affected_path)(),
            },
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: PlanExecutionErrorKind::Internal(message.into()),
            context: PlanExecutionErrorContext {
                subgraph_name: None,
                affected_path: None,
            },
        }
    }

    pub fn error_code(&self) -> &'static str {
        (&self.kind).into()
    }

    pub fn subgraph_name(&self) -> &Option<String> {
        &self.context.subgraph_name
    }

    pub fn affected_path(&self) -> &Option<String> {
        &self.context.affected_path
    }
}

impl From<PlanExecutionError> for GraphQLError {
    fn from(val: PlanExecutionError) -> Self {
        let message = val.to_string();
        GraphQLError {
            extensions: GraphQLErrorExtensions {
                code: Some(val.error_code().into()),
                service_name: val.context.subgraph_name,
                affected_path: val.context.affected_path,
                extensions: Default::default(),
            },
            message,
            locations: None,
            path: None,
        }
    }
}

/// An extension trait for `Result` types that can be converted into a `PlanExecutionError`.
///
/// This trait provides a lazy, performant way to add contextual information to
/// an error, only performing work (like cloning strings) if the `Result` is an `Err`.
pub trait IntoPlanExecutionError<T> {
    fn with_plan_context<SN, AP>(
        self,
        context: LazyPlanContext<SN, AP>,
    ) -> Result<T, PlanExecutionError>
    where
        SN: FnOnce() -> Option<String>,
        AP: FnOnce() -> Option<String>;
}

impl<T> IntoPlanExecutionError<T> for Result<T, ProjectionError> {
    fn with_plan_context<SN, AP>(
        self,
        context: LazyPlanContext<SN, AP>,
    ) -> Result<T, PlanExecutionError>
    where
        SN: FnOnce() -> Option<String>,
        AP: FnOnce() -> Option<String>,
    {
        self.map_err(|source| {
            let kind = PlanExecutionErrorKind::ProjectionFailure(source);
            PlanExecutionError::new(kind, context)
        })
    }
}

impl<T> IntoPlanExecutionError<T> for Result<T, HeaderRuleRuntimeError> {
    fn with_plan_context<SN, AP>(
        self,
        context: LazyPlanContext<SN, AP>,
    ) -> Result<T, PlanExecutionError>
    where
        SN: FnOnce() -> Option<String>,
        AP: FnOnce() -> Option<String>,
    {
        self.map_err(|source| {
            let kind = PlanExecutionErrorKind::HeaderPropagation(source);
            PlanExecutionError::new(kind, context)
        })
    }
}
