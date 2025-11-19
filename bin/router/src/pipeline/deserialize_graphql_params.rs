use std::collections::HashMap;

use hive_router_plan_executor::hooks::on_graphql_params::GraphQLParams;
use http::Method;
use ntex::util::Bytes;
use ntex::web::types::Query;
use ntex::web::HttpRequest;
use tracing::{trace, warn};

use crate::pipeline::error::PipelineErrorVariant;
use crate::pipeline::header::AssertRequestJson;

#[derive(serde::Deserialize, Debug)]
struct GETQueryParams {
    pub query: Option<String>,
    #[serde(rename = "camelCase")]
    pub operation_name: Option<String>,
    pub variables: Option<String>,
    pub extensions: Option<String>,
}

impl TryInto<GraphQLParams> for GETQueryParams {
    type Error = PipelineErrorVariant;

    fn try_into(self) -> Result<GraphQLParams, Self::Error> {
        let variables = match self.variables.as_deref() {
            Some(v_str) if !v_str.is_empty() => match sonic_rs::from_str(v_str) {
                Ok(vars) => vars,
                Err(e) => {
                    return Err(PipelineErrorVariant::FailedToParseVariables(e));
                }
            },
            _ => HashMap::new(),
        };

        let extensions = match self.extensions.as_deref() {
            Some(e_str) if !e_str.is_empty() => match sonic_rs::from_str(e_str) {
                Ok(exts) => Some(exts),
                Err(e) => {
                    return Err(PipelineErrorVariant::FailedToParseExtensions(e));
                }
            },
            _ => None,
        };

        let execution_request = GraphQLParams {
            query: self.query,
            operation_name: self.operation_name,
            variables,
            extensions,
        };

        Ok(execution_request)
    }
}

pub trait GetQueryStr {
    fn get_query(&self) -> Result<&str, PipelineErrorVariant>;
}

impl GetQueryStr for GraphQLParams {
    fn get_query(&self) -> Result<&str, PipelineErrorVariant> {
        self.query
            .as_deref()
            .ok_or(PipelineErrorVariant::GetMissingQueryParam("query"))
    }
}

#[inline]
pub fn deserialize_graphql_params(
    req: &HttpRequest,
    body_bytes: Bytes,
) -> Result<GraphQLParams, PipelineErrorVariant> {
    let http_method = req.method();
    let graphql_params: GraphQLParams = match *http_method {
        Method::GET => {
            trace!("processing GET GraphQL operation");
            let query_params_str = req
                .uri()
                .query()
                .ok_or_else(|| PipelineErrorVariant::GetInvalidQueryParams)?;
            let query_params = Query::<GETQueryParams>::from_query(query_params_str)
                .map_err(PipelineErrorVariant::GetUnprocessableQueryParams)?
                .0;

            trace!("parsed GET query params: {:?}", query_params);

            query_params.try_into()?
        }
        Method::POST => {
            trace!("Processing POST GraphQL request");

            req.assert_json_content_type()?;

            let execution_request = unsafe {
                sonic_rs::from_slice_unchecked::<GraphQLParams>(&body_bytes).map_err(|e| {
                    warn!("Failed to parse body: {}", e);
                    PipelineErrorVariant::FailedToParseBody(e)
                })?
            };

            execution_request
        }
        _ => {
            warn!("unsupported HTTP method: {}", http_method);

            return Err(PipelineErrorVariant::UnsupportedHttpMethod(
                http_method.to_owned(),
            ));
        }
    };

    Ok(graphql_params)
}
