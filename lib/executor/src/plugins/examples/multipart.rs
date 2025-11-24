use std::collections::HashMap;

use crate::{
    executors::common::HttpExecutionResponse,
    hooks::{
        on_graphql_params::{
            GraphQLParams, OnGraphQLParamsEndPayload, OnGraphQLParamsStartPayload,
        },
        on_subgraph_execute::{OnSubgraphExecuteEndPayload, OnSubgraphExecuteStartPayload},
    },
    plugin_trait::{HookResult, RouterPlugin, RouterPluginWithConfig, StartPayload},
};
use bytes::Bytes;
use dashmap::DashMap;
use multer::Multipart;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct MultipartPluginConfig {
    pub enabled: bool,
}
pub struct MultipartPlugin {}

pub struct MultipartFile {
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub content: Bytes,
}

pub struct MultipartContext {
    pub file_map: HashMap<String, Vec<String>>,
    pub files: DashMap<String, MultipartFile>,
}

#[derive(Serialize)]
struct MultipartOperations<'a> {
    pub query: &'a str,
    pub variables: Option<&'a HashMap<&'a str, &'a sonic_rs::Value>>,
    pub operation_name: Option<&'a str>,
}

impl RouterPluginWithConfig for MultipartPlugin {
    type Config = MultipartPluginConfig;
    fn plugin_name() -> &'static str {
        "multipart_plugin"
    }
    fn from_config(config: MultipartPluginConfig) -> Option<Self> {
        if config.enabled {
            Some(MultipartPlugin {})
        } else {
            None
        }
    }
}

#[async_trait::async_trait]
impl RouterPlugin for MultipartPlugin {
    async fn on_graphql_params<'exec>(
        &'exec self,
        mut payload: OnGraphQLParamsStartPayload<'exec>,
    ) -> HookResult<'exec, OnGraphQLParamsStartPayload<'exec>, OnGraphQLParamsEndPayload> {
        if let Some(content_type) = payload.router_http_request.headers.get("content-type") {
            if let Ok(content_type_str) = content_type.to_str() {
                if content_type_str.starts_with("multipart/form-data") {
                    let boundary = multer::parse_boundary(content_type_str).unwrap();
                    let body = payload.body.clone();
                    let stream = futures_util::stream::once(async move {
                        Ok::<Bytes, std::io::Error>(Bytes::from(body.to_vec()))
                    });
                    let mut multipart = Multipart::new(stream, boundary);
                    while let Some(field) = multipart.next_field().await.unwrap() {
                        let field_name = field.name().unwrap().to_string();
                        let filename = field.file_name().map(|s| s.to_string());
                        let content_type = field.content_type().map(|s| s.to_string());
                        let data = field.bytes().await.unwrap();
                        match field_name.as_str() {
                            "operations" => {
                                let graphql_params: GraphQLParams =
                                    sonic_rs::from_slice(&data).unwrap();
                                payload.graphql_params = Some(graphql_params);
                            }
                            "map" => {
                                let file_map: HashMap<String, Vec<String>> =
                                    sonic_rs::from_slice(&data).unwrap();
                                payload.context.insert(MultipartContext {
                                    file_map,
                                    files: DashMap::new(),
                                });
                            }
                            field_name => {
                                let mut ctx_entry = payload.context.get_mut_entry();
                                let multipart_ctx: Option<&mut MultipartContext> =
                                    ctx_entry.get_ref_mut();
                                if let Some(multipart_ctx) = multipart_ctx {
                                    let multipart_file = MultipartFile {
                                        filename,
                                        content_type,
                                        content: data,
                                    };
                                    multipart_ctx
                                        .files
                                        .insert(field_name.to_string(), multipart_file);
                                }
                            }
                        }
                    }
                }
            }
        }
        payload.cont()
    }

    async fn on_subgraph_execute<'exec>(
        &'exec self,
        mut payload: OnSubgraphExecuteStartPayload<'exec>,
    ) -> HookResult<'exec, OnSubgraphExecuteStartPayload<'exec>, OnSubgraphExecuteEndPayload> {
        if let Some(variables) = &payload.execution_request.variables {
            let ctx_ref = payload.context.get_ref_entry();
            let multipart_ctx: Option<&MultipartContext> = ctx_ref.get_ref();
            if let Some(multipart_ctx) = multipart_ctx {
                let mut file_map: HashMap<String, Vec<String>> = HashMap::new();
                for variable_name in variables.keys() {
                    // Matching variables that are file references
                    for (files_ref, op_refs) in &multipart_ctx.file_map {
                        for op_ref in op_refs {
                            if op_ref.starts_with(format!("variables.{}", variable_name).as_str()) {
                                let op_refs_in_curr_map =
                                    file_map.entry(files_ref.to_string()).or_default();
                                op_refs_in_curr_map.push(op_ref.to_string());
                            }
                        }
                    }
                }
                if !file_map.is_empty() {
                    let mut form = reqwest::multipart::Form::new();
                    let operations_struct = MultipartOperations {
                        query: payload.execution_request.query,
                        variables: payload.execution_request.variables.as_ref(),
                        operation_name: payload.execution_request.operation_name,
                    };
                    let operations = sonic_rs::to_string(&operations_struct).unwrap();
                    form = form.text("operations", operations);
                    let file_map_str: String = sonic_rs::to_string(&file_map).unwrap();
                    form = form.text("map", file_map_str);
                    for (file_ref, _op_refs) in file_map {
                        if let Some(file_field) = multipart_ctx.files.get(&file_ref) {
                            let mut part =
                                reqwest::multipart::Part::bytes(file_field.content.to_vec());
                            if let Some(file_name) = &file_field.filename {
                                part = part.file_name(file_name.to_string());
                            }
                            if let Some(content_type) = &file_field.content_type {
                                part = part.mime_str(&content_type.to_string()).unwrap();
                            }
                            form = form.part(file_ref, part);
                        }
                    }
                    let resp = reqwest::Client::new()
                        .post("http://example.com/graphql")
                        // Using query as endpoint URL
                        .multipart(form)
                        .send()
                        .await
                        .unwrap();
                    let headers = resp.headers().clone();
                    let status = resp.status();
                    let body = resp.bytes().await.unwrap();
                    payload.execution_result = Some(HttpExecutionResponse {
                        body,
                        headers,
                        status,
                    });
                }
            }
        }
        payload.cont()
    }
}
