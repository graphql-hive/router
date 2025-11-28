use std::collections::HashMap;

use bytes::Bytes;
use hive_router_plan_executor::{
    execution::plan::PlanExecutionOutput,
    executors::dedupe::SharedResponse,
    hooks::{
        on_graphql_params::{
            GraphQLParams, OnGraphQLParamsEndPayload, OnGraphQLParamsStartPayload,
        },
        on_subgraph_http_request::{OnSubgraphHttpRequestPayload, OnSubgraphHttpResponsePayload},
    },
    plugin_trait::{HookResult, RouterPlugin, RouterPluginWithConfig, StartPayload},
};
use multer::Multipart;
use serde::Deserialize;
use serde_json::json;
use tracing::error;

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
    pub files: HashMap<String, MultipartFile>,
}

impl RouterPluginWithConfig for MultipartPlugin {
    type Config = MultipartPluginConfig;
    fn plugin_name() -> &'static str {
        "multipart"
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
                                    files: HashMap::new(),
                                });
                            }
                            field_name => {
                                let multipart_ctx = payload.context.get_mut::<MultipartContext>();
                                if let Some(mut multipart_ctx) = multipart_ctx {
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

    async fn on_subgraph_http_request<'exec>(
        &'exec self,
        mut payload: OnSubgraphHttpRequestPayload<'exec>,
    ) -> HookResult<'exec, OnSubgraphHttpRequestPayload<'exec>, OnSubgraphHttpResponsePayload> {
        if let Some(variables) = &payload.execution_request.variables {
            let multipart_ctx = payload.context.get_ref::<MultipartContext>();
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
                    form = form.text(
                        "operations",
                        String::from_utf8(payload.body.clone()).unwrap(),
                    );
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
                        .post(payload.endpoint.to_string())
                        // Using query as endpoint URL
                        .multipart(form)
                        .send()
                        .await;
                    match resp {
                        Ok(resp) => {
                            payload.response = Some(SharedResponse {
                                status: resp.status(),
                                headers: resp.headers().clone(),
                                body: resp.bytes().await.unwrap(),
                            });
                        }
                        Err(err) => {
                            error!("Failed to send multipart request to subgraph: {}", err);
                            let body = json!({
                                "errors": [{
                                    "message": format!("Failed to send multipart request to subgraph: {}", err)
                                }]
                            });
                            return payload.end_response(PlanExecutionOutput {
                                status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                                headers: reqwest::header::HeaderMap::new(),
                                body: serde_json::to_vec(&body).unwrap(),
                            });
                        }
                    }
                }
            }
        }
        payload.cont()
    }
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;
    use hive_router::PluginRegistry;
    use ntex::web::test;

    use crate::testkit::{init_router_from_config_inline, wait_for_readiness, SubgraphsServer};

    #[ntex::test]
    async fn forward_files() {
        let subgraphs_server = SubgraphsServer::start().await;

        let app = init_router_from_config_inline(
            r#"
            plugins:
              multipart:
                enabled: true
            "#,
            Some(PluginRegistry::new().register::<super::MultipartPlugin>()),
        )
        .await
        .expect("Router should initialize successfully");
        wait_for_readiness(&app.app).await;

        let form = reqwest::multipart::Form::new()
            .text("operations", r#"{"query":"mutation ($file: Upload) { upload(file: $file) }","variables":{"file":null}}"#)
            .text("map", r#"{"0":["variables.file"]}"#)
            .part(
                "0",
                reqwest::multipart::Part::bytes("file content".as_bytes().to_vec())
                    .file_name("test.txt")
                    .mime_str("text/plain")
                    .unwrap(),
            );

        let boundary = form.boundary().to_string();
        let form_stream = form.into_stream();

        let mut form_bytes = vec![];
        let mut stream = form_stream;
        while let Some(item) = stream.next().await {
            let chunk = item.expect("Failed to read chunk");
            form_bytes.extend_from_slice(&chunk);
        }

        let req = test::TestRequest::post()
            .uri("/graphql")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={}", boundary),
            )
            .set_payload(form_bytes);

        let resp = test::call_service(&app.app, req.to_request()).await;

        let body = test::read_body(resp).await;
        let body_str = String::from_utf8_lossy(&body);
        let body_json = serde_json::from_str::<serde_json::Value>(&body_str).unwrap();
        let upload_file_path = &body_json["data"]["upload"].as_str().unwrap();
        assert!(
            upload_file_path.contains("test.txt"),
            "Response should contain the filename"
        );
        let file_content = tokio::fs::read(upload_file_path).await.unwrap();
        assert_eq!(
            file_content, b"file content",
            "File content should match the uploaded content"
        );
        assert_eq!(
            subgraphs_server
                .get_subgraph_requests_log("products")
                .await
                .unwrap()
                .len(),
            1,
            "Expected 1 request to products subgraph"
        );
    }
}
