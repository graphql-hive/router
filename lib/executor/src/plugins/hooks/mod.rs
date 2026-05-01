pub mod on_execute;
pub mod on_graphql_error;
pub mod on_graphql_params;
pub mod on_graphql_parse;
pub mod on_graphql_validation;
pub mod on_http_request;
pub mod on_plugin_init;
pub mod on_query_plan;
pub mod on_subgraph_execute;
pub mod on_subgraph_http_request;
pub mod on_supergraph_load;

mod sealed {
    pub trait Sealed {}
}

pub trait HookMarker: sealed::Sealed {}

pub struct OnHttpRequest;
pub struct OnGraphqlParams;
pub struct OnGraphqlParse;
pub struct OnGraphqlValidation;
pub struct OnQueryPlan;
pub struct OnExecute;
pub struct OnSubgraphExecute;
pub struct OnSubgraphHttp;

impl sealed::Sealed for OnHttpRequest {}
impl sealed::Sealed for OnGraphqlParams {}
impl sealed::Sealed for OnGraphqlParse {}
impl sealed::Sealed for OnGraphqlValidation {}
impl sealed::Sealed for OnQueryPlan {}
impl sealed::Sealed for OnExecute {}
impl sealed::Sealed for OnSubgraphExecute {}
impl sealed::Sealed for OnSubgraphHttp {}

impl HookMarker for OnHttpRequest {}
impl HookMarker for OnGraphqlParams {}
impl HookMarker for OnGraphqlParse {}
impl HookMarker for OnGraphqlValidation {}
impl HookMarker for OnQueryPlan {}
impl HookMarker for OnExecute {}
impl HookMarker for OnSubgraphExecute {}
impl HookMarker for OnSubgraphHttp {}
