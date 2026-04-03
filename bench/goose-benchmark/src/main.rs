use goose::config::{GooseDefault, GooseDefaultType};
use goose::metrics::GooseMetrics;
use goose::prelude::*;
use humantime::parse_duration;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::OnceLock;
use url::Url;
use xxhash_rust::xxh3::xxh3_64;

static BENCHMARK_CONFIG: OnceLock<BenchmarkConfig> = OnceLock::new();

#[derive(Debug)]
struct BenchmarkConfig {
    host: String,
    path: String,
    request_body: String,
    expected_hash: u64,
    vus: usize,
    run_time_label: String,
    run_time_seconds: usize,
    summary_path: Option<PathBuf>,
}

#[derive(Serialize)]
struct SummaryValues {
    rate: f64,
}

#[derive(Serialize)]
struct SummaryHttpReqs {
    values: SummaryValues,
}

#[derive(Serialize)]
struct SummaryMetrics {
    http_reqs: SummaryHttpReqs,
}

#[derive(Serialize)]
struct SummaryOutput {
    metrics: SummaryMetrics,
    vus: usize,
    duration: String,
}

fn benchmark() -> &'static BenchmarkConfig {
    BENCHMARK_CONFIG
        .get()
        .expect("benchmark config initialized")
}

fn hash_bytes(value: &[u8]) -> u64 {
    xxh3_64(value)
}

fn parse_endpoint(endpoint: &str) -> Result<(String, String), GooseError> {
    let parsed = Url::parse(endpoint).map_err(|e| GooseError::InvalidOption {
        option: "ROUTER_ENDPOINT".to_string(),
        value: endpoint.to_string(),
        detail: e.to_string(),
    })?;

    let host = parsed.host_str().ok_or_else(|| GooseError::InvalidOption {
        option: "ROUTER_ENDPOINT".to_string(),
        value: endpoint.to_string(),
        detail: "missing host".to_string(),
    })?;

    let mut path = parsed.path().to_string();
    if path.is_empty() {
        path = "/".to_string();
    }

    if let Some(query) = parsed.query() {
        path.push('?');
        path.push_str(query);
    }

    let host = match parsed.port() {
        Some(port) => format!("{}://{}:{}", parsed.scheme(), host, port),
        None => format!("{}://{}", parsed.scheme(), host),
    };

    Ok((host, path))
}

fn build_benchmark_config() -> Result<BenchmarkConfig, GooseError> {
    let endpoint = std::env::var("ROUTER_ENDPOINT")
        .unwrap_or_else(|_| "http://0.0.0.0:4000/graphql".to_string());
    let (host, path) = parse_endpoint(&endpoint)?;

    let query = include_str!("../../operation.graphql");
    let request_body =
        serde_json::to_string(&serde_json::json!({ "query": query })).map_err(|e| {
            GooseError::InvalidOption {
                option: "request_body".to_string(),
                value: "operation.graphql".to_string(),
                detail: e.to_string(),
            }
        })?;

    let expected_hash = hash_bytes(include_bytes!("../../expected_response.json"));

    let vus = std::env::var("BENCH_VUS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(50);

    let run_time_label = std::env::var("BENCH_OVER_TIME").unwrap_or_else(|_| "30s".to_string());
    let run_time_seconds = parse_duration(&run_time_label)
        .map_err(|e| GooseError::InvalidOption {
            option: "BENCH_OVER_TIME".to_string(),
            value: run_time_label.clone(),
            detail: e.to_string(),
        })?
        .as_secs()
        .max(1) as usize;

    let summary_path = std::env::var("SUMMARY_PATH").ok().map(PathBuf::from);

    Ok(BenchmarkConfig {
        host,
        path,
        request_body,
        expected_hash,
        vus,
        run_time_label,
        run_time_seconds,
        summary_path,
    })
}

async fn run_graphql_request(user: &mut GooseUser) -> TransactionResult {
    let cfg = benchmark();
    let mut goose = user.post(&cfg.path, cfg.request_body.as_str()).await?;

    match goose.response {
        Ok(response) => {
            let status = response.status();
            let headers = response.headers().clone();
            let body = response.bytes().await?;

            if status.as_u16() != 200 {
                return user.set_failure(
                    "response code was not 200",
                    &mut goose.request,
                    Some(&headers),
                    std::str::from_utf8(&body).ok(),
                );
            }

            if body
                .windows(br#""errors""#.len())
                .any(|window| window == br#""errors""#)
            {
                return user.set_failure(
                    "graphql errors in response",
                    &mut goose.request,
                    Some(&headers),
                    std::str::from_utf8(&body).ok(),
                );
            }

            let actual_hash = hash_bytes(&body);
            if actual_hash != cfg.expected_hash {
                return user.set_failure(
                    "response hash mismatch",
                    &mut goose.request,
                    Some(&headers),
                    std::str::from_utf8(&body).ok(),
                );
            }

            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

fn write_summary(metrics: &GooseMetrics) -> Result<(), GooseError> {
    let cfg = benchmark();

    let Some(summary_path) = &cfg.summary_path else {
        return Ok(());
    };

    std::fs::create_dir_all(summary_path).map_err(GooseError::Io)?;

    let total_requests: usize = metrics
        .requests
        .values()
        .map(|request| request.success_count + request.fail_count)
        .sum();

    let duration_seconds = metrics.duration.max(1) as f64;
    let requests_per_second = total_requests as f64 / duration_seconds;

    let summary = SummaryOutput {
        metrics: SummaryMetrics {
            http_reqs: SummaryHttpReqs {
                values: SummaryValues {
                    rate: requests_per_second,
                },
            },
        },
        vus: cfg.vus,
        duration: cfg.run_time_label.clone(),
    };

    let json_path = summary_path.join("goose_summary.json");
    let txt_path = summary_path.join("goose_summary.txt");

    let summary_json = serde_json::to_string_pretty(&summary).map_err(GooseError::Serde)?;

    std::fs::write(&json_path, summary_json).map_err(GooseError::Io)?;
    std::fs::write(&txt_path, format!("{metrics}")).map_err(GooseError::Io)?;

    println!(
        "Writing summary to {}/goose_summary.json and .txt",
        summary_path.display()
    );

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    let cfg = build_benchmark_config()?;

    BENCHMARK_CONFIG
        .set(cfg)
        .map_err(|_| GooseError::InvalidOption {
            option: "benchmark-config".to_string(),
            value: "init".to_string(),
            detail: "benchmark config was initialized more than once".to_string(),
        })?;

    let cfg = benchmark();

    let attack = GooseAttack::initialize()?
        .register_scenario(
            scenario!("router-benchmark")
                .set_host(&cfg.host)
                .register_transaction(transaction!(run_graphql_request)),
        )
        .set_default(GooseDefault::Users, cfg.vus)?
        .set_default(GooseDefault::RunTime, cfg.run_time_seconds)?;

    let metrics = attack.execute().await?;
    write_summary(&metrics)?;

    Ok(())
}
