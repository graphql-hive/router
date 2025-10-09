use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for how the Router should override subgraph URLs.
/// This can be used to point to different subgraph endpoints based on environment,
/// or to use dynamic expressions to determine the URL at runtime.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone)]
#[schemars(example = override_subgraph_urls_example_1())]
pub struct OverrideSubgraphUrlsConfig {
    /// Keys are subgraph names as defined in the supergraph schema.
    #[serde(default)]
    pub subgraphs: HashMap<String, OverrideSubgraphUrlConfig>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(untagged)]
pub enum OverrideSubgraphUrlConfig {
    /// A static URL to override the subgraph's URL.
    ///
    /// This URL must be absolute and include the scheme (http or https).
    /// ### Example
    /// ```yaml
    /// subgraphs:
    ///  accounts:
    ///     url: "https://accounts.example.com/graphql"
    Url { url: String },
    /// A dynamic value computed by a VRL expression.
    ///
    /// This allows you to generate header values based on the incoming request,
    /// subgraph name, and (for response rules) subgraph response headers.
    /// The expression has access to a context object with a `.request` field
    /// that contains the incoming request data.
    ///
    /// For more information on the available functions and syntax, see the
    /// [VRL documentation](https://vrl.dev/).
    ///
    /// ### Example
    /// ```yaml
    /// subgraphs:
    ///  products:
    ///   expression: |
    ///     if .request.headers."x-region" == "us-east" {
    ///         "https://products-us-east.example.com/graphql"
    ///     } else if .request.headers."x-region" == "eu-west" {
    ///         "https://products-eu-west.example.com/graphql"
    ///     } else {
    ///         "https://products.example.com/graphql"
    ///     }
    /// ```
    ///
    /// `null` means the default URL from the supergraph will be used.
    Expression { expression: String },
}

fn override_subgraph_urls_example_1() -> OverrideSubgraphUrlsConfig {
    let expression = r#"
        if .request.headers."x-region" == "us-east" {
            "https://products-us-east.example.com/graphql"
        } else if .request.headers."x-region" == "eu-west" {
            "https://products-eu-west.example.com/graphql"
        } else {
            "https://products.example.com/graphql"
        }
    "#;
    let mut subgraphs = HashMap::new();
    subgraphs.insert(
        "accounts".to_string(),
        OverrideSubgraphUrlConfig::Url {
            url: "https://accounts.example.com/graphql".to_string(),
        },
    );
    subgraphs.insert(
        "products".to_string(),
        OverrideSubgraphUrlConfig::Expression {
            expression: expression.to_string(),
        },
    );

    OverrideSubgraphUrlsConfig { subgraphs }
}
