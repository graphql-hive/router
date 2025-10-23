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
}

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

pub struct ErrorContext<SN, AP> {
    pub subgraph_name: SN,
    pub affected_path: AP,
}

pub trait IntoPlanExecutionError<T> {
    fn with_plan_context<SN, AP>(
        self,
        context: ErrorContext<SN, AP>,
    ) -> Result<T, PlanExecutionError>
    where
        SN: FnOnce() -> Option<String>,
        AP: FnOnce() -> Option<String>;
}

impl<T> IntoPlanExecutionError<T> for Result<T, ProjectionError> {
    fn with_plan_context<SN, AP>(
        self,
        context: ErrorContext<SN, AP>,
    ) -> Result<T, PlanExecutionError>
    where
        SN: FnOnce() -> Option<String>,
        AP: FnOnce() -> Option<String>,
    {
        self.map_err(|source| PlanExecutionError {
            kind: PlanExecutionErrorKind::ProjectionFailure(source),
            context: context.into(),
        })
    }
}

impl<T> IntoPlanExecutionError<T> for Result<T, HeaderRuleRuntimeError> {
    fn with_plan_context<SN, AP>(
        self,
        context: ErrorContext<SN, AP>,
    ) -> Result<T, PlanExecutionError>
    where
        SN: FnOnce() -> Option<String>,
        AP: FnOnce() -> Option<String>,
    {
        self.map_err(|source| PlanExecutionError {
            kind: PlanExecutionErrorKind::HeaderPropagation(source),
            context: context.into(),
        })
    }
}

impl<SN: FnOnce() -> Option<String>, AP: FnOnce() -> Option<String>> From<ErrorContext<SN, AP>>
    for PlanExecutionErrorContext
{
    fn from(context: ErrorContext<SN, AP>) -> Self {
        PlanExecutionErrorContext {
            subgraph_name: (context.subgraph_name)(),
            affected_path: (context.affected_path)(),
        }
    }
}

impl PlanExecutionError {
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

impl From<&PlanExecutionError> for GraphQLError {
    fn from(val: &PlanExecutionError) -> Self {
        GraphQLError {
            extensions: GraphQLErrorExtensions {
                code: Some(val.error_code().into()),
                service_name: val.subgraph_name().clone(),
                affected_path: val.affected_path().clone(),
                extensions: Default::default(),
            },
            message: val.to_string(),
            locations: None,
            path: None,
        }
    }
}

impl From<PlanExecutionError> for GraphQLError {
    fn from(val: PlanExecutionError) -> Self {
        (&val).into()
    }
}
