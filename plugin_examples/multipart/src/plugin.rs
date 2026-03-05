use std::collections::HashMap;

use futures_util::StreamExt;
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
};
use multer::{bytes::Bytes, Multipart};
use reqwest::header::CONTENT_TYPE;

#[derive(Default)]
pub struct MultipartPlugin;

pub struct MultipartFile {
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub content: Bytes,
}

pub struct MultipartContext {
    pub file_map: HashMap<String, Vec<String>>,
    pub files: HashMap<String, MultipartFile>,
}

pub async fn form_to_content_type_and_bytes(form: reqwest::multipart::Form) -> (String, Vec<u8>) {
    let content_type = format!("multipart/form-data; boundary={}", form.boundary());
    let form_stream = form.into_stream();

    let mut form_bytes = vec![];
    let mut stream = form_stream;
    while let Some(item) = stream.next().await {
        let chunk = item.expect("Failed to read chunk");
        form_bytes.extend_from_slice(&chunk);
    }

    (content_type, form_bytes)
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
                    let boundary = multer::parse_boundary(content_type_str)
                        .expect("Failed to parse boundary from content type");
                    let body = payload.body.clone();
                    let stream = futures_util::stream::once(async move {
                        Ok::<Bytes, std::io::Error>(Bytes::from(body.to_vec()))
                    });
                    let mut multipart = Multipart::new(stream, boundary);
                    while let Ok(Some(field)) = multipart.next_field().await {
                        let field_name = field.name().map(|s| s.to_string());
                        let filename = field.file_name().map(|s| s.to_string());
                        let content_type = field.content_type().map(|s| s.to_string());
                        let data = field
                            .bytes()
                            .await
                            .expect("Failed to read field data as bytes");
                        match field_name.as_deref() {
                            Some("operations") => {
                                if let Ok(graphql_params) = sonic_rs::from_slice(&data) {
                                    payload = payload.with_graphql_params(graphql_params);
                                }
                            }
                            Some("map") => {
                                if let Ok(file_map) = sonic_rs::from_slice(&data) {
                                    payload.context.insert(MultipartContext {
                                        file_map,
                                        files: HashMap::new(),
                                    });
                                }
                            }
                            Some(field_name) => {
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
                            None => {
                                // Ignore fields without a name
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
        mut payload: OnSubgraphHttpRequestHookPayload<'exec>,
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
                    if let Ok(operations) = String::from_utf8(payload.body) {
                        form = form.text("operations", operations);
                    }
                    if let Ok(file_map_str) = sonic_rs::to_string(&file_map) {
                        form = form.text("map", file_map_str);
                    }
                    for (file_ref, _op_refs) in file_map {
                        if let Some(file_field) = multipart_ctx.files.get(&file_ref) {
                            let mut part =
                                reqwest::multipart::Part::bytes(file_field.content.to_vec());
                            if let Some(file_name) = &file_field.filename {
                                part = part.file_name(file_name.to_string());
                            }
                            if let Some(content_type) = &file_field.content_type {
                                part = part
                                    .mime_str(content_type)
                                    .expect("Invalid content type for multipart file");
                            }
                            form = form.part(file_ref, part);
                        }
                    }
                    let (content_type, form_bytes) = form_to_content_type_and_bytes(form).await;
                    payload.body = form_bytes;
                    let content_type = content_type
                        .try_into()
                        .expect("Failed to create content type header value");
                    payload
                        .execution_request
                        .headers
                        .insert(CONTENT_TYPE, content_type);
                }
            }
        }
        payload.proceed()
    }
}
