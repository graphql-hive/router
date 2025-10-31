use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::primitives::expression::Expression;

/// Configuration for how the Router should override subgraph URLs.
/// This can be used to point to different subgraph endpoints based on environment,
/// or to use dynamic expressions to determine the URL at runtime.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone)]
#[schemars(transparent, example = override_subgraph_urls_example_1())]
#[serde(transparent)]
pub struct OverrideSubgraphUrlsConfig(HashMap<String, PerSubgraphConfig>);

impl OverrideSubgraphUrlsConfig {
    pub fn get_subgraph_url(&self, subgraph_name: &str) -> Option<&UrlOrExpression> {
        self.0.get(subgraph_name).map(|entry| &entry.url)
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub struct PerSubgraphConfig {
    /// Overrides for the URL of the subgraph.
    ///
    /// For convenience, a plain string in your configuration will be treated as a static URL.
    ///
    /// ### Static URL Example
    /// ```yaml
    /// url: "https://api.example.com/graphql"
    /// ```
    ///
    /// ### Dynamic Expression Example
    ///
    /// The expression has access to the following variables:
    /// - `request`: The incoming HTTP request, including headers and other metadata.
    /// - `original_url`: The original URL of the subgraph (from supergraph sdl).
    ///
    /// ```yaml
    /// url:
    ///   expression: |
    ///     if .request.headers."x-region" == "us-east" {
    ///       "https://products-us-east.example.com/graphql"
    ///     } else if .request.headers."x-region" == "eu-west" {
    ///       "https://products-eu-west.example.com/graphql"
    ///     } else {
    ///       .original_url
    ///     }
    pub url: UrlOrExpression,
}
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(untagged)]
pub enum UrlOrExpression {
    /// A static URL string.
    Url(String),
    /// A dynamic value computed by a VRL expression.
    Expression(Expression),
}

fn override_subgraph_urls_example_1() -> OverrideSubgraphUrlsConfig {
    let expression = r#"
        if .request.headers."x-region" == "us-east" {
            "https://products-us-east.example.com/graphql"
        } else if .request.headers."x-region" == "eu-west" {
            "https://products-eu-west.example.com/graphql"
        } else {
          .original_url
        }
    "#;
    let mut subgraphs = HashMap::new();
    subgraphs.insert(
        "accounts".to_string(),
        PerSubgraphConfig {
            url: UrlOrExpression::Url("https://accounts.example.com/graphql".to_string()),
        },
    );
    subgraphs.insert(
        "products".to_string(),
        PerSubgraphConfig {
            url: UrlOrExpression::Expression(expression.to_string().try_into().unwrap()),
        },
    );

    OverrideSubgraphUrlsConfig(subgraphs)
}
