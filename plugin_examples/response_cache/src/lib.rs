use std::collections::HashMap;

use hive_router::http::HeaderMap;
use hive_router::plugins::hooks::on_execute::{
    OnExecuteEndHookPayload, OnExecuteStartHookPayload, OnExecuteStartHookResult,
};
use hive_router::plugins::hooks::on_plugin_init::{OnPluginInitPayload, OnPluginInitResult};
use hive_router::plugins::hooks::on_supergraph_load::{
    OnSupergraphLoadStartHookPayload, OnSupergraphLoadStartHookResult,
};
use hive_router::plugins::plugin_trait::{
    EarlyHTTPResponse, EndHookPayload, RouterPlugin, StartHookPayload,
};
use hive_router::ArcSwap;
use hive_router::{async_trait, graphql_tools, sonic_rs};
use redis::Commands;
use serde::Deserialize;

use hive_router::tracing::trace;

#[derive(Deserialize)]
pub struct ResponseCachePluginOptions {
    pub redis_url: String,
    #[serde(default = "default_ttl_seconds")]
    pub default_ttl_seconds: u64,
}

fn default_ttl_seconds() -> u64 {
    5
}

pub struct ResponseCachePlugin {
    redis: r2d2::Pool<redis::Client>,
    ttl_per_type: ArcSwap<HashMap<String, u64>>,
    default_ttl_seconds: u64,
}

#[async_trait]
impl RouterPlugin for ResponseCachePlugin {
    type Config = ResponseCachePluginOptions;
    fn plugin_name() -> &'static str {
        "response_cache_plugin"
    }
    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        let config = payload.config()?;
        let redis_client = redis::Client::open(config.redis_url.as_str())?;
        let pool = r2d2::Pool::builder().build(redis_client)?;
        payload.initialize_plugin(Self {
            redis: pool,
            ttl_per_type: Default::default(),
            default_ttl_seconds: config.default_ttl_seconds,
        })
    }
    async fn on_execute<'exec>(
        &'exec self,
        payload: OnExecuteStartHookPayload<'exec>,
    ) -> OnExecuteStartHookResult<'exec> {
        let key = format!(
            "response_cache:{}:{:?}",
            payload.query_plan, payload.variable_values
        );
        if let Ok(mut conn) = self.redis.get() {
            trace!("Checking cache for key: {}", key);
            let cache_result: Result<Vec<u8>, redis::RedisError> = conn.get(&key);
            match cache_result {
                Ok(body) => {
                    if body.is_empty() {
                        trace!("Cache miss for key: {}", key);
                    } else {
                        trace!(
                            "Cache hit for key: {} -> {}",
                            key,
                            String::from_utf8_lossy(&body)
                        );
                        let mut headers = HeaderMap::new();
                        headers.insert(
                            "X-Cache-Status",
                            "HIT"
                                .parse()
                                .expect("X-Cache-Status and HIT are valid header name and value"),
                        );
                        return payload.end_with_response(EarlyHTTPResponse {
                            body,
                            headers,
                            ..Default::default()
                        });
                    }
                }
                Err(err) => {
                    trace!("Error accessing cache for key {}: {}", key, err);
                }
            }
            return payload.on_end(move |mut payload: OnExecuteEndHookPayload<'exec>| {
                // Do not cache if there are errors
                if !payload.errors.is_empty() {
                    trace!("Not caching response due to errors");
                    return payload.proceed();
                }

                if let Ok(serialized) = sonic_rs::to_vec(&payload.data) {
                    let ttl_per_type = self.ttl_per_type.load();
                    trace!("Caching response for key: {}", key);
                    // Decide on the ttl somehow
                    // Get the type names
                    let mut max_ttl = 0;

                    // Imagine this code is traversing the response data to find type names
                    if let Some(obj) = payload.data.as_object() {
                        if let Some(typename) = obj
                            .iter()
                            .position(|(k, _)| k == &"__typename")
                            .and_then(|idx| obj[idx].1.as_str())
                        {
                            if let Some(ttl) = ttl_per_type.get(typename) {
                                max_ttl = max_ttl.max(*ttl);
                            }
                        }
                    }

                    // If no ttl found, default
                    if max_ttl == 0 {
                        max_ttl = self.default_ttl_seconds;
                    }
                    trace!("Using TTL of {} seconds for key: {}", max_ttl, key);

                    // Insert the ttl into extensions for client awareness
                    payload.add_extension("response_cache_ttl", max_ttl);

                    // Set the cache with the decided ttl
                    let result = conn.set_ex::<&str, Vec<u8>, ()>(&key, serialized, max_ttl);
                    if let Err(err) = result {
                        trace!("Failed to set cache for key {}: {}", key, err);
                    } else {
                        trace!("Cached response for key: {} with TTL: {}", key, max_ttl);
                    }
                }
                payload.proceed()
            });
        }
        payload.proceed()
    }
    fn on_supergraph_reload<'a>(
        &'a self,
        payload: OnSupergraphLoadStartHookPayload,
    ) -> OnSupergraphLoadStartHookResult<'a> {
        let mut ttl_per_type = HashMap::new();
        // Visit the schema and update ttl_per_type based on some directive
        payload.new_ast.definitions.iter().for_each(|def| {
            if let graphql_tools::parser::schema::Definition::TypeDefinition(
                graphql_tools::parser::schema::TypeDefinition::Object(obj_type),
            ) = def
            {
                for directive in &obj_type.directives {
                    if directive.name == "cacheControl" {
                        for arg in &directive.arguments {
                            if arg.0 == "maxAge" {
                                if let graphql_tools::parser::query::Value::Int(max_age) = &arg.1 {
                                    if let Some(max_age) = max_age.as_i64() {
                                        ttl_per_type.insert(obj_type.name.clone(), max_age as u64);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        self.ttl_per_type.store(ttl_per_type.into());

        payload.proceed()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        io::{BufRead, BufReader, Read, Write},
        net::{TcpListener, TcpStream},
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };

    use e2e::testkit::{TestRouter, TestSubgraphs};
    use hive_router::{http::StatusCode, ntex};

    type Store = Arc<Mutex<HashMap<String, (Vec<u8>, Instant)>>>;

    struct TinyRedisServer {
        addr: String,
    }

    impl TinyRedisServer {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0")
                .expect("tiny redis server should bind to an ephemeral port");

            let addr = listener
                .local_addr()
                .expect("tiny redis server should expose local addr");

            let store: Store = Arc::new(Mutex::new(HashMap::new()));
            let store_for_thread = Arc::clone(&store);

            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let stream = match stream {
                        Ok(stream) => stream,
                        Err(_) => break,
                    };

                    let store = Arc::clone(&store_for_thread);

                    std::thread::spawn(move || {
                        handle_connection(stream, store);
                    });
                }
            });

            Self {
                addr: format!("127.0.0.1:{}", addr.port()),
            }
        }

        fn redis_url(&self) -> String {
            format!("redis://{}", self.addr)
        }
    }

    fn read_line(reader: &mut BufReader<TcpStream>) -> Option<String> {
        let mut line = String::new();

        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }

        Some(line.trim_end_matches("\r\n").to_string())
    }

    fn read_command(reader: &mut BufReader<TcpStream>) -> Option<Vec<Vec<u8>>> {
        let header = read_line(reader)?;
        let count: usize = header.strip_prefix('*')?.parse().ok()?;

        let mut parts = Vec::with_capacity(count);

        for _ in 0..count {
            let len_header = read_line(reader)?;
            let len: usize = len_header.strip_prefix('$')?.parse().ok()?;

            let mut bytes = vec![0; len];
            reader.read_exact(&mut bytes).ok()?;

            let mut crlf = [0_u8; 2];
            reader.read_exact(&mut crlf).ok()?;

            if crlf != *b"\r\n" {
                return None;
            }

            parts.push(bytes);
        }

        Some(parts)
    }

    fn write_simple(stream: &mut TcpStream, value: &str) {
        let _ = stream.write_all(format!("+{value}\r\n").as_bytes());
        let _ = stream.flush();
    }

    fn write_error(stream: &mut TcpStream, value: &str) {
        let _ = stream.write_all(format!("-ERR {value}\r\n").as_bytes());
        let _ = stream.flush();
    }

    fn write_bulk(stream: &mut TcpStream, value: Option<&[u8]>) {
        match value {
            Some(bytes) => {
                let _ = stream.write_all(format!("${}\r\n", bytes.len()).as_bytes());
                let _ = stream.write_all(bytes);
                let _ = stream.write_all(b"\r\n");
            }
            None => {
                let _ = stream.write_all(b"$-1\r\n");
            }
        }

        let _ = stream.flush();
    }

    fn clean_expired(store: &mut HashMap<String, (Vec<u8>, Instant)>) {
        let now = Instant::now();
        store.retain(|_, (_, expires_at)| *expires_at > now);
    }

    fn is_command(value: &[u8], expected: &[u8]) -> bool {
        value.eq_ignore_ascii_case(expected)
    }

    fn handle_connection(mut stream: TcpStream, store: Store) {
        let read_stream = stream
            .try_clone()
            .expect("tiny redis server should clone stream");

        let mut reader = BufReader::new(read_stream);

        while let Some(parts) = read_command(&mut reader) {
            let Some(command) = parts.first() else {
                write_error(&mut stream, "empty command");
                continue;
            };

            if is_command(command, b"PING") {
                write_simple(&mut stream, "PONG");
                continue;
            }

            if is_command(command, b"CLIENT") {
                // redis-rs may send CLIENT SETINFO / CLIENT SETNAME.
                // We don't care in this test.
                write_simple(&mut stream, "OK");
                continue;
            }

            if is_command(command, b"GET") {
                if parts.len() != 2 {
                    write_error(&mut stream, "wrong number of arguments for GET");
                    continue;
                }

                let key = String::from_utf8_lossy(&parts[1]).to_string();

                let mut store = store.lock().expect("tiny redis store lock should succeed");
                clean_expired(&mut store);

                let value = store.get(&key).map(|(value, _)| value.clone());
                write_bulk(&mut stream, value.as_deref());

                continue;
            }

            if is_command(command, b"SETEX") {
                if parts.len() != 4 {
                    write_error(&mut stream, "wrong number of arguments for SETEX");
                    continue;
                }

                let key = String::from_utf8_lossy(&parts[1]).to_string();

                let ttl_seconds = match String::from_utf8_lossy(&parts[2]).parse::<u64>() {
                    Ok(value) => value,
                    Err(_) => {
                        write_error(&mut stream, "invalid expire time");
                        continue;
                    }
                };

                let value = parts[3].clone();
                let expires_at = Instant::now() + Duration::from_secs(ttl_seconds);

                let mut store = store.lock().expect("tiny redis store lock should succeed");
                clean_expired(&mut store);
                store.insert(key, (value, expires_at));

                write_simple(&mut stream, "OK");
                continue;
            }

            write_error(
                &mut stream,
                &format!("unsupported command {}", String::from_utf8_lossy(command)),
            );
        }
    }

    #[ntex::test]
    async fn test_caching_with_default_ttl() {
        let redis_server = TinyRedisServer::start();
        let redis_url = redis_server.redis_url();

        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(format!(
                r#"
                  supergraph:
                    source: file
                    path: ../../e2e/supergraph.graphql
                  plugins:
                    response_cache_plugin:
                      enabled: true
                      config:
                          redis_url: "{}"
                          default_ttl_seconds: 2
                "#,
                redis_url
            ))
            .register_plugin::<super::ResponseCachePlugin>()
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        assert_eq!(res.status(), StatusCode::OK, "first request should succeed");

        let subgraph_requests = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests to accounts subgraph");
        assert_eq!(subgraph_requests.len(), 1, "expected one subgraph request");

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        assert!(res.status().is_success(), "second request should succeed");

        let subgraph_requests = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests to accounts subgraph");
        assert_eq!(
            subgraph_requests.len(),
            1,
            "expected only one subgraph request due to caching"
        );

        ntex::time::sleep(std::time::Duration::from_secs(3)).await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        assert!(
            res.status().is_success(),
            "third request should succeed after cache expiry"
        );

        let subgraph_requests = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests to accounts subgraph");
        assert_eq!(
            subgraph_requests.len(),
            2,
            "expected a second subgraph request after cache expiry"
        );
    }
}
