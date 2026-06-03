use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Defines how the Router changes subgraph URLs.
///
/// Use this when you want the Router to send requests to a different URL than
/// the one defined in the supergraph.
///
/// You can configure it in two ways:
///
/// - `subgraphs`: overrides URLs for specific subgraphs.
///   Each subgraph can use either a fixed URL or a dynamic `expression`.
///
/// - `all`: one override used for every subgraph.
///   This is useful when the same rule should apply to all subgraphs.
///
/// If both `subgraphs` and `all` are set, the per-subgraph override wins.
///
/// Expressions can use:
/// - `.request`: the incoming HTTP request
/// - `.request.path_params`: path parameters from `http.graphql_endpoint`
/// - `.default`: the original subgraph URL from the supergraph
/// - `.subgraph.name`: the current subgraph name
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[schemars(example = override_subgraph_urls_example_1())]
pub struct OverrideSubgraphUrlsConfig {
    /// URL overrides for specific subgraphs.
    ///
    /// The key is the subgraph name.
    ///
    /// Each subgraph can use:
    /// - a fixed URL string
    /// - a dynamic expression
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub subgraphs: HashMap<String, OverrideUrlConfig>,
    /// Default URL override for all subgraphs.
    ///
    /// This override is used when a subgraph does not have its own override in
    /// `subgraphs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all: Option<OverrideAllUrlConfig>,
}

impl OverrideSubgraphUrlsConfig {
    /// Returns the per-subgraph override configured for `subgraph_name`, if any.
    pub fn get_subgraph_url(&self, subgraph_name: &str) -> Option<&UrlOrExpression> {
        self.subgraphs.get(subgraph_name).map(|config| &config.url)
    }

    /// Returns the global (`all`) override, if any.
    pub fn get_all_url(&self) -> Option<&str> {
        self.all
            .as_ref()
            .map(|config| config.url.expression.as_str())
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct OverrideUrlConfig {
    pub url: UrlOrExpression,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct OverrideAllUrlConfig {
    pub url: OverrideExpressionConfig,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct OverrideExpressionConfig {
    pub expression: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(untagged)]
pub enum UrlOrExpression {
    /// A static URL string.
    Url(String),
    /// A dynamic value computed by a VRL expression.
    Expression { expression: String },
}

#[cfg(test)]
mod tests {
    use crate::parse_yaml_config;

    #[test]
    fn rejects_static_all_url_override() {
        let config = r#"
        supergraph:
          source: file
          path: supergraph.graphql
        override_subgraph_urls:
          all:
            url: "https://example.com/graphql"
        "#;

        let result = parse_yaml_config(config.to_string());

        assert!(result.is_err(), "expected static all.url to be rejected");
    }
}

fn override_subgraph_urls_example_1() -> OverrideSubgraphUrlsConfig {
    let expression = r#"
        if .request.headers."x-region" == "us-east" {
            "https://products-us-east.example.com/graphql"
        } else if .request.headers."x-region" == "eu-west" {
            "https://products-eu-west.example.com/graphql"
        } else {
          .default
        }
    "#;
    let mut subgraphs = HashMap::new();
    subgraphs.insert(
        "accounts".to_string(),
        OverrideUrlConfig {
            url: UrlOrExpression::Url("https://accounts.example.com/graphql".to_string()),
        },
    );
    subgraphs.insert(
        "products".to_string(),
        OverrideUrlConfig {
            url: UrlOrExpression::Expression {
                expression: expression.to_string(),
            },
        },
    );

    OverrideSubgraphUrlsConfig {
        subgraphs,
        all: None,
    }
}
