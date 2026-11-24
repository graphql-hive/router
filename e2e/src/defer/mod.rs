//! `@defer` end-to-end tests, grouped by what they check:
//! - `transport`: response formats, `Accept` negotiation, SSE, WebSocket, when `@defer` is refused
//! - `conditions`: `@defer(if:)`, `@skip`/`@include` around deferred fragments
//! - `shapes`: one query shape after another, the merged parts must equal the plain response
//! - `paths`: where parts land (root, lists, nesting, siblings, abstract types, `__typename`)
//! - `errors`: subgraph errors, nulls stopping at the fragment, authorization
//! - `fetching`: which subgraph calls run, and when
//! - `plugins`: plugin hooks with deferred operations
//!
//! Most tests go through [`Setup`]: the snapshot shows the parts the router sends (one JSON
//! per line), then every subgraph request, and the merged parts are checked against the same
//! operation without `@defer`. Tests that answer the subgraphs themselves use `supergraph.graphql`
//! next to this file: `accounts`, `products`, `reviews` and `inventory`.

use bytes::Bytes;
use graphql_differential::merge_incremental;
use serde_json::{json, Value};

use crate::testkit::{
    mock_subgraphs::mock_subgraphs, some_header_map, ClientResponseExt, RequestLike, ResponseLike,
    Started, TestRouter, TestSubgraphs, TestSubgraphsBuilder,
};

mod conditions;
mod errors;
mod fetching;
mod paths;
mod plugins;
mod shapes;
mod transport;

pub type OnRequest = Box<dyn Fn(RequestLike) -> Option<ResponseLike> + Send + Sync>;

pub const SUPERGRAPH: &str = "src/defer/supergraph.graphql";

pub const LEGACY: &str = "multipart/mixed;deferSpec=20220824";

pub struct Setup {
    pub subgraphs: TestSubgraphs<Started>,
    pub router: TestRouter<Started>,
}

/// The demo supergraph and its subgraphs.
pub async fn demo() -> Setup {
    start_full("supergraph.graphql", None, "").await
}

/// [`SUPERGRAPH`], its subgraphs answered by `mock_subgraphs`.
pub async fn start(mocks: Value) -> Setup {
    start_with(mock_subgraphs(mocks)).await
}

/// [`SUPERGRAPH`], its subgraphs answered by `on_request`.
pub async fn start_with(
    on_request: impl Fn(RequestLike) -> Option<ResponseLike> + Send + Sync + 'static,
) -> Setup {
    start_full(SUPERGRAPH, Some(Box::new(on_request)), "").await
}

/// Any supergraph path, any subgraphs, extra router config. `@defer` is on unless
/// `extra_config` says otherwise.
pub async fn start_full(
    supergraph: &str,
    on_request: Option<OnRequest>,
    extra_config: &str,
) -> Setup {
    let mut builder = TestSubgraphs::builder();
    if let Some(on_request) = on_request {
        builder = builder.with_on_request(on_request);
    }
    start_on(supergraph, builder, extra_config).await
}

/// Like [`start_full`], for subgraphs that need more than `on_request`, like a slow one.
pub async fn start_on(
    supergraph: &str,
    subgraphs: TestSubgraphsBuilder,
    extra_config: &str,
) -> Setup {
    let subgraphs = subgraphs.build().start().await;
    let defer = if extra_config.contains("defer:") {
        ""
    } else {
        "defer:\n  enabled: true\n"
    };
    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            "supergraph:\n  source: file\n  path: {supergraph}\n{defer}{extra_config}"
        ))
        .build()
        .start()
        .await;
    Setup { subgraphs, router }
}

pub fn mocks(config: Value) -> Option<OnRequest> {
    Some(Box::new(mock_subgraphs(config)))
}

/// A JSON response with this body.
pub fn respond(body: Value) -> Option<ResponseLike> {
    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("application/json"),
    );
    Some(ResponseLike {
        status: axum::http::StatusCode::OK,
        headers,
        body: Some(Bytes::from(body.to_string())),
    })
}

/// Answers a subgraph request with the first body whose subgraph matches and whose snippet is
/// in the query. Anything else gets an error that shows the query, so a miss is easy to see.
pub fn canned(rules: Vec<(&'static str, &'static str, Value)>) -> Option<OnRequest> {
    Some(Box::new(move |req: RequestLike| {
        let name = req.path.trim_start_matches('/').to_string();
        let body: Value = serde_json::from_slice(req.body.as_ref()?).ok()?;
        let query = body["query"].as_str().unwrap_or_default().to_string();
        let response = rules
            .iter()
            .find(|(subgraph, snippet, _)| *subgraph == name && query.contains(snippet))
            .map(|(_, _, response)| response.clone())
            .unwrap_or_else(|| json!({ "errors": [{ "message": format!("no canned response for {name}: {query}") }] }));
        respond(response)
    }))
}

/// `me.favoriteProduct` from `accounts`, then its `relatedProducts` from `products`.
pub fn related_products_mocks(related: Value) -> Value {
    // each related product is an entity too, for the deferred fields fetched by key
    let mut entities =
        vec![json!({ "__typename": "Product", "id": "0", "relatedProducts": related })];
    entities.extend(related.as_array().cloned().unwrap_or_default());
    json!({
        "accounts": {
            "query": { "me": { "favoriteProduct": { "__typename": "Product", "id": "0" } } }
        },
        "products": { "entities": entities }
    })
}

/// A user from `accounts`, their reviews from `reviews`.
pub fn user_reviews_mocks() -> Value {
    json!({
        "accounts": {
            "query": { "user": { "__typename": "User", "id": "1", "name": "Ada", "email": "ada@example.com" } }
        },
        "reviews": {
            "entities": [{
                "__typename": "User",
                "id": "1",
                "reviews": [
                    { "title": "Great", "body": "Works well" },
                    { "title": "Okay", "body": "Does the job" },
                    { "title": "Bad", "body": "Broke in a week" }
                ]
            }]
        }
    })
}

/// What the merged parts get compared against.
pub enum Plain<'a> {
    /// The same query with `@defer` stripped.
    Stripped,
    /// A hand-written query, for when the stripped one wouldn't validate (unused `$shouldDefer`).
    Query(&'a str),
    /// Nothing. A null or an error inside a deferred fragment stops at the fragment, while the
    /// plain response lets it bubble further up, so the two can't match.
    Skip,
}

/// The parts of a response, and what gets snapshotted: the parts, one per line, then every
/// subgraph request.
pub struct Run {
    pub parts: Vec<Value>,
    pub text: String,
}

impl Setup {
    /// Sends `query` asking for `multipart/mixed`, checks the merged parts against `plain`, and
    /// returns the snapshot text.
    pub async fn run(&self, query: &str, variables: Option<Value>, plain: Plain<'_>) -> String {
        let deferred = send_deferred(&self.router, query, variables.clone()).await;
        self.report(query, variables, plain, deferred).await.text
    }

    /// [`Self::run`] comparing against the query without `@defer`.
    pub async fn check(&self, query: &str, variables: Option<Value>) -> Run {
        let deferred = send_deferred(&self.router, query, variables.clone()).await;
        self.report(query, variables, Plain::Stripped, deferred)
            .await
    }

    /// [`Self::run`] without comparing.
    pub async fn send(&self, query: &str, variables: Option<Value>) -> Run {
        let deferred = send_deferred(&self.router, query, variables.clone()).await;
        self.report(query, variables, Plain::Skip, deferred).await
    }

    /// Like [`Self::run`], but also accepts plain JSON
    /// (`application/graphql-response+json`, `application/json;q=0.9`, ...), for the cases where
    /// a non-streamed response is expected.
    pub async fn run_accepting_json(
        &self,
        query: &str,
        variables: Option<Value>,
        plain: Plain<'_>,
    ) -> String {
        let (content_type, body) = self
            .send_raw(
                query,
                variables.clone(),
                "multipart/mixed, application/graphql-response+json, application/json;q=0.9",
            )
            .await;
        let parts = if content_type.starts_with("multipart/mixed") {
            multipart_parts(&body)
        } else {
            vec![serde_json::from_str(&body).unwrap_or_else(|e| panic!("{e}: {body}"))]
        };
        let merged = merge_parts(&parts);
        let run = self
            .report(query, variables, plain, Deferred { parts, merged })
            .await;
        format!("content-type: {content_type}\n{}", run.text)
    }

    /// Sends with any `Accept` and gives back the content type and the body as is.
    pub async fn send_raw(
        &self,
        query: &str,
        variables: Option<Value>,
        accept: &str,
    ) -> (String, String) {
        let variables = variables.map(|v| sonic_rs::from_str(&v.to_string()).unwrap());
        let response = self
            .router
            .send_graphql_request(
                query,
                variables,
                some_header_map! { http::header::ACCEPT => accept },
            )
            .await;
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        (content_type, response.string_body().await)
    }

    /// Status, content type and body, with `accept` (or no `Accept` at all). A v0.2 multipart
    /// body still goes through the merge checks.
    pub async fn raw(&self, query: &str, variables: Option<Value>, accept: Option<&str>) -> String {
        let mut request = self
            .router
            .serv()
            .post(self.router.graphql_path())
            .header(http::header::CONTENT_TYPE, "application/json");
        if let Some(accept) = accept {
            request = request.header(http::header::ACCEPT, accept);
        }
        let response = request
            .send_json(&json!({ "query": query, "variables": variables }))
            .await
            .unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = response.string_body().await;
        if content_type.starts_with("multipart/mixed") && !content_type.contains("deferSpec") {
            merge_parts(&multipart_parts(&body));
        }
        format!("{status}\n{content_type}\n{body}")
    }

    async fn report(
        &self,
        query: &str,
        variables: Option<Value>,
        plain: Plain<'_>,
        deferred: Deferred,
    ) -> Run {
        // each test runs one deferred query on fresh subgraphs, so the logs only hold its
        // requests at this point
        let mut out = canonical_parts(&deferred.parts);
        for (name, requests) in self.subgraphs.requests_log() {
            let mut lines: Vec<String> = requests
                .iter()
                .map(|request| {
                    let body: Value =
                        serde_json::from_slice(request.body.as_deref().unwrap()).unwrap();
                    let mut line = format!("{name}: {}", body["query"].as_str().unwrap());
                    if let Some(vars) = body.get("variables").filter(|v| !v.is_null()) {
                        // the router writes variables in no fixed order, sort them so the
                        // snapshot stays put
                        let vars: std::collections::BTreeMap<String, Value> =
                            serde_json::from_value(vars.clone()).unwrap();
                        line.push_str(&format!(" {}", serde_json::to_string(&vars).unwrap()));
                    }
                    line
                })
                .collect();
            // calls to one subgraph can run in parallel, so their order isn't fixed
            lines.sort();
            out.extend(lines);
        }
        let text = out.join("\n");

        let plain = match plain {
            Plain::Stripped => Some(query),
            Plain::Query(plain) => Some(plain),
            Plain::Skip => None,
        };
        if let Some(plain) = plain {
            let plain_response = send_plain(&self.router, plain, variables).await;
            assert_eq!(
                normalize(&deferred.merged),
                normalize(&plain_response),
                "merged parts differ from the response without @defer\n{text}"
            );
        }
        Run {
            parts: deferred.parts,
            text,
        }
    }
}

/// The parts, one JSON per line, in an order that doesn't depend on timing. Sibling fragments
/// go out as soon as their own data is in, so their parts come in any order. Parts are sorted by
/// how deep they are (a part comes after the one that announced it) and then by what they hold,
/// ids are renumbered in that order, and `hasNext` is left out after the primary part, since
/// [`merge_parts`] already checks that only the last part says `false`.
pub fn canonical_parts(parts: &[Value]) -> Vec<String> {
    let Some((primary, rest)) = parts.split_first() else {
        return Vec::new();
    };
    let ids = |part: &Value, key: &str| -> Vec<String> {
        part[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item["id"].as_str().map(str::to_string))
            .collect()
    };
    // how deep each part is, from the order they really came in
    let mut depth_of_id: std::collections::HashMap<String, usize> = ids(primary, "pending")
        .into_iter()
        .map(|id| (id, 1))
        .collect();
    let mut rest: Vec<(usize, Value)> = rest
        .iter()
        .map(|part| {
            let depth = ids(part, "completed")
                .iter()
                .chain(&ids(part, "incremental"))
                .filter_map(|id| depth_of_id.get(id).copied())
                .max()
                .unwrap_or(1);
            for id in ids(part, "pending") {
                depth_of_id.insert(id, depth + 1);
            }
            let mut part = part.clone();
            if let Some(object) = part.as_object_mut() {
                object.remove("hasNext");
            }
            (depth, part)
        })
        .collect();

    // only the ids of `pending`, `incremental` and `completed` entries, not `id` fields in data
    fn rename_ids(part: &mut Value, rename: &mut dyn FnMut(&str) -> String) {
        for key in ["pending", "incremental", "completed"] {
            let items = part.get_mut(key).and_then(Value::as_array_mut);
            for item in items.into_iter().flatten() {
                if let Some(Value::String(id)) = item.get_mut("id") {
                    *id = rename(id);
                }
            }
        }
    }
    let renumber = |primary: &Value, rest: &[(usize, Value)]| {
        let mut new_ids: std::collections::HashMap<String, String> = Default::default();
        let mut parts: Vec<Value> = std::iter::once(primary.clone())
            .chain(rest.iter().map(|(_, part)| part.clone()))
            .collect();
        for part in &mut parts {
            for id in ids(part, "pending") {
                let next = new_ids.len().to_string();
                new_ids.entry(id).or_insert(next);
            }
            rename_ids(part, &mut |id| new_ids.get(id).cloned().unwrap_or_default());
        }
        parts
    };
    // sort ignoring ids first, then again with the renumbered ids to break ties
    rest.sort_by_cached_key(|(depth, part)| {
        let mut part = part.clone();
        rename_ids(&mut part, &mut |_| String::new());
        (*depth, part.to_string())
    });
    let numbered = renumber(primary, &rest);
    let mut order: Vec<usize> = (0..rest.len()).collect();
    order.sort_by_key(|index| {
        (
            rest[*index].0,
            {
                let mut part = numbered[index + 1].clone();
                rename_ids(&mut part, &mut |_| String::new());
                part.to_string()
            },
            numbered[index + 1].to_string(),
        )
    });
    let rest: Vec<(usize, Value)> = order.into_iter().map(|index| rest[index].clone()).collect();
    renumber(primary, &rest)
        .iter()
        .map(Value::to_string)
        .collect()
}

/// A deferred response split into its parts, and those parts merged back into one response.
pub struct Deferred {
    pub parts: Vec<Value>,
    pub merged: Value,
}

/// Sends `query` asking for `multipart/mixed`. A response that isn't multipart comes back as
/// its only part.
pub async fn send_deferred(
    router: &TestRouter<Started>,
    query: &str,
    variables: Option<Value>,
) -> Deferred {
    let variables = variables.map(|v| sonic_rs::from_str(&v.to_string()).unwrap());
    let response = router
        .send_graphql_request(
            query,
            variables,
            some_header_map! { http::header::ACCEPT => "multipart/mixed" },
        )
        .await;
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let body = response.string_body().await;
    let parts = if content_type.starts_with("multipart/mixed") {
        multipart_parts(&body)
    } else {
        vec![serde_json::from_str(&body).unwrap_or_else(|e| panic!("{e}: {body}"))]
    };
    let merged = merge_parts(&parts);
    Deferred { parts, merged }
}

/// Sends `query` without any `@defer`, as plain JSON.
pub async fn send_plain(
    router: &TestRouter<Started>,
    query: &str,
    variables: Option<Value>,
) -> Value {
    let (query, off) = strip_defer(query);
    let mut variables = variables.unwrap_or_else(|| json!({}));
    for name in off {
        variables[name] = json!(false);
    }
    let variables = sonic_rs::from_str(&variables.to_string()).unwrap();
    let response = router
        .send_graphql_request(&query, Some(variables), None)
        .await;
    let body = response.string_body().await;
    serde_json::from_str(&body).unwrap_or_else(|e| panic!("{e}: {body}"))
}

/// The JSON bodies of a `multipart/mixed` response with our `-` boundary.
pub fn multipart_parts(body: &str) -> Vec<Value> {
    body.split("\r\n---")
        .flat_map(|chunk| chunk.split("\n---"))
        .filter_map(|chunk| {
            let (_, json) = chunk
                .split_once("\r\n\r\n")
                .or_else(|| chunk.split_once("\n\n"))?;
            let json = json.trim();
            (!json.is_empty() && json != "--")
                .then(|| serde_json::from_str(json).unwrap_or_else(|e| panic!("{e}: {json}")))
        })
        .collect()
}

/// Merges incremental parts (`incrementalSpec=v0.2`) into one response, and panics when they
/// don't follow the spec, see [`merge_incremental`].
pub fn merge_parts(parts: &[Value]) -> Value {
    merge_incremental(parts).unwrap_or_else(|e| panic!("{e}"))
}

/// Field order isn't compared, and neither is the order of errors. Nor is an error's
/// `affectedPath`: it's where the failed fetch sits, and the plan without `@defer` has other
/// fetches.
pub fn normalize(response: &Value) -> Value {
    let mut response = response.clone();
    if let Some(Value::Array(errors)) = response.get_mut("errors") {
        for error in errors.iter_mut() {
            if let Some(Value::Object(extensions)) = error.get_mut("extensions") {
                extensions.remove("affectedPath");
            }
        }
        errors.sort_by_key(|e| e.to_string());
    }
    response
}

/// The same operation without `@defer`. A `@defer(if: $v)` stays as `@defer(if: $v)`, so `$v`
/// is still used, and comes back in the list of variables to send as `false`.
pub fn strip_defer(query: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(query.len());
    let mut off = Vec::new();
    let mut rest = query;
    while let Some(start) = rest.find("@defer") {
        out.push_str(&rest[..start]);
        rest = &rest[start + "@defer".len()..];
        let trimmed = rest.trim_start();
        if trimmed.starts_with('(') {
            let close = trimmed.find(')').expect("unclosed @defer arguments");
            if let Some((_, var)) = trimmed[..close].split_once('$') {
                let var: String = var
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                out.push_str(&format!("@defer(if: ${var})"));
                off.push(var);
            }
            rest = &trimmed[close + 1..];
        }
    }
    out.push_str(rest);
    (out, off)
}

#[test]
fn strip_defer_works() {
    assert_eq!(
        strip_defer(
            r#"query($d: Boolean!) { a ... @defer(label: "x", if: $d) { b } ...F @defer c }"#
        ),
        (
            "query($d: Boolean!) { a ... @defer(if: $d) { b } ...F  c }".to_string(),
            vec!["d".to_string()]
        )
    );
}
