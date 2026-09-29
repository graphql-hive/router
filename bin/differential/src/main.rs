use graphql_differential::{GeneratorConfig, QueryGenerator};
use graphql_tools::parser::schema::parse_schema;
use reqwest::Client;
use serde_json::Value as JsonValue;
use std::time::Duration;

async fn execute_query(
    client: &Client,
    url: &str,
    query: &str,
    variables: &str,
) -> Result<JsonValue, reqwest::Error> {
    let body = serde_json::json!({
        "query": query,
        "variables": serde_json::from_str::<JsonValue>(variables).unwrap_or(serde_json::json!({}))
    });

    let res = client
        .post(url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    res.json::<JsonValue>().await
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!(
            "Usage: {} <baseline-endpoint> <candidate-endpoint> <schema.graphql>",
            args[0]
        );
        eprintln!(
            "Example: {} http://localhost:4300/graphql http://localhost:4000/graphql bench/schema.graphql",
            args[0]
        );
        return;
    }

    let baseline_endpoint = &args[1];
    let candidate_endpoint = &args[2];

    let schema_str = match std::fs::read_to_string(&args[3]) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("Failed to read schema file {}: {}", args[3], e);
            return;
        }
    };

    let schema = match parse_schema::<String>(&schema_str) {
        Ok(doc) => doc.into_static(),
        Err(e) => {
            eprintln!("Failed to parse schema: {}", e);
            return;
        }
    };

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    let mut differences = 0;
    let mut num_queries = 100;

    if let Ok(val) = std::env::var("GRAPHQL_DIFF_QUERIES") {
        num_queries = val.parse().unwrap_or(10);
    }

    println!("Running {} differential queries against:", num_queries);
    println!("  baseline: {}", baseline_endpoint);
    println!("  candidate: {}", candidate_endpoint);
    println!("--------------------------------------------------");

    for i in 0..num_queries {
        let seed = i as u64 + 42;
        let case = QueryGenerator::new(&schema, seed, GeneratorConfig::default()).generate();

        println!("Query #{}:", i + 1);

        let res1_future = execute_query(
            &client,
            baseline_endpoint,
            &case.document,
            &case.variables_json,
        );
        let res2_future = execute_query(
            &client,
            candidate_endpoint,
            &case.document,
            &case.variables_json,
        );

        let (res1, res2) = tokio::join!(res1_future, res2_future);

        match (res1, res2) {
            (Ok(res1), Ok(res2)) => {
                // The comparison should happen on `result.data` and not `result.errors`.
                // The errors part may differ, so we should just check wether errors are present or not.
                let (data1, errors1) = match &res1 {
                    JsonValue::Object(res) => {
                        (res.get("data").cloned(), res.get("errors").cloned())
                    }
                    _ => panic!("Not a graphql response"),
                };
                let (data2, errors2) = match &res2 {
                    JsonValue::Object(res) => {
                        (res.get("data").cloned(), res.get("errors").cloned())
                    }
                    _ => panic!("Not a graphql response"),
                };

                if data1 != data2 || errors1.is_some() != errors2.is_some() {
                    differences += 1;

                    let dir = format!("./failed-tests/case-{}", i + 1);
                    std::fs::create_dir_all(&dir).expect("to create a directory");
                    std::fs::write(format!("{}/query.graphql", dir), case.document.clone())
                        .expect("to create query.graphql");
                    std::fs::write(
                        format!("{}/variables.json", dir),
                        case.variables_json.clone(),
                    )
                    .expect("to create query.graphql");
                    std::fs::write(
                        format!("{}/endpoint-1.json", dir),
                        serde_json::to_string_pretty(&res1).unwrap(),
                    )
                    .expect("to create endpoint-1.json");
                    std::fs::write(
                        format!("{}/endpoint-2.json", dir),
                        serde_json::to_string_pretty(&res2).unwrap(),
                    )
                    .expect("to create endpoint-2.json");

                    println!("⚠️ Responses differ");
                } else {
                    println!("✅ Responses match");
                }
            }
            (Err(e1), Err(e2)) => {
                println!("⚠️ Both endpoints failed");
                println!("  baseline: {}", e1);
                println!("  candidate: {}", e2);
            }
            (Err(e1), Ok(_)) => {
                println!("❌ Baseline endpoint failed: {}", e1);
                differences += 1;
            }
            (Ok(_), Err(e2)) => {
                println!("❌ Candidate endpoint failed: {}", e2);
                differences += 1;
            }
        }
        println!("--------------------------------------------------");
    }

    if differences == 0 {
        println!("🎉 All queries returned matching results!");
    } else {
        println!("⚠️ Found {} queries with different results.", differences);
        std::process::exit(1);
    }
}
