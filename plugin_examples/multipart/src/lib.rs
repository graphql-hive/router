use std::collections::HashMap;

use hive_router::{
    async_trait,
    plugins::{
        hooks::{
            on_graphql_params::{OnGraphQLParamsStartHookPayload, OnGraphQLParamsStartHookResult},
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
            on_subgraph_http_request::{
                OnSubgraphHttpRequestHookPayload, OnSubgraphHttpRequestHookResult,
            },
        },
        plugin_trait::{RouterPlugin, StartHookPayload},
    },
    sonic_rs,
    tracing::error,
    GraphQLError, SubgraphHttpResponse,
};
use multer::{bytes::Bytes, Multipart};
use reqwest::StatusCode;

#[derive(Default)]
pub struct MultipartPlugin {
    client: reqwest::Client,
}

pub struct MultipartFile {
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub content: Bytes,
}

pub struct MultipartContext {
    pub file_map: HashMap<String, Vec<String>>,
    pub files: HashMap<String, MultipartFile>,
}

#[async_trait]
impl RouterPlugin for MultipartPlugin {
    type Config = ();
    fn plugin_name() -> &'static str {
        "multipart"
    }
    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        payload.initialize_plugin_with_defaults()
    }
    async fn on_graphql_params<'exec>(
        &'exec self,
        mut payload: OnGraphQLParamsStartHookPayload<'exec>,
    ) -> OnGraphQLParamsStartHookResult<'exec> {
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
                                payload = payload
                                    .with_graphql_params(sonic_rs::from_slice(&data).unwrap());
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
        payload.proceed()
    }

    async fn on_subgraph_http_request<'exec>(
        &'exec self,
        payload: OnSubgraphHttpRequestHookPayload<'exec>,
    ) -> OnSubgraphHttpRequestHookResult<'exec> {
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
                    return match self
                        .client
                        .post(payload.endpoint.to_string())
                        // Using query as endpoint URL
                        .multipart(form)
                        .send()
                        .await
                    {
                        Ok(resp) => payload.end_with_response(SubgraphHttpResponse {
                            status: resp.status(),
                            headers: resp.headers().clone().into(),
                            body: resp.bytes().await.unwrap(),
                        }),
                        Err(err) => {
                            error!("Failed to send multipart request to subgraph: {}", err);
                            payload.end_with_graphql_error(
                                GraphQLError::from("Failed to send multipart request to subgraph"),
                                StatusCode::BAD_REQUEST,
                            )
                        }
                    };
                }
            }
        }
        payload.proceed()
    }
}

#[cfg(test)]
mod tests {
    use e2e::testkit::{ClientResponseExt, TestRouterBuilder, TestSubgraphsBuilder};
    use futures_util::StreamExt;
    use hive_router::{ntex, sonic_rs::JsonValueTrait};

    #[ntex::test]
    async fn forward_files() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;

        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("../plugin_examples/multipart/router.config.yaml")
            .register_plugin::<super::MultipartPlugin>()
            .build()
            .start()
            .await;

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

        let res = router
            .serv()
            .post(router.graphql_path())
            .header(
                "content-type",
                format!("multipart/form-data; boundary={}", boundary),
            )
            .send_body(form_bytes)
            .await
            .unwrap();

        let body_str = res.string_body().await;
        let body_json = hive_router::sonic_rs::from_str::<hive_router::sonic_rs::Value>(&body_str).unwrap();
        let upload_file_path = body_json["data"]["upload"].as_str().unwrap();
        assert!(
            upload_file_path.contains("test.txt"),
            "Response should contain the filename"
        );
        let file_content = std::fs::read(upload_file_path).unwrap();
        assert_eq!(
            file_content, b"file content",
            "File content should match the uploaded content"
        );
        assert_eq!(
            subgraphs
                .get_requests_log("products")
                .unwrap()
                .len(),
            1,
            "Expected 1 request to products subgraph"
        );
    }
}
