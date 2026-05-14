use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for how the Router should override subgraph URLs.
/// This can be used to point to different subgraph endpoints based on environment,
/// or to use dynamic expressions to determine the URL at runtime.
///
/// Two top-level keys are supported:
/// - `subgraphs`: per-subgraph URL overrides, keyed by subgraph name. Each
///   value is either a static URL string or an object with an `expression`
///   field for dynamic VRL evaluation.
/// - `all`: a single override that is applied to every subgraph, useful when the
///   override logic is the same (or depends on the subgraph name) for all of them.
///
/// When both are present, the `subgraphs.<name>` override takes precedence over
/// `all` for the matching subgraph; subgraphs without a per-subgraph override
/// fall back to the `all` override.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[schemars(example = override_subgraph_urls_example_1())]
pub struct OverrideSubgraphUrlsConfig {
    /// Per-subgraph URL overrides, keyed by subgraph name.
    ///
    /// Each entry is either a static URL string or an object with an
    /// `expression` field for dynamic VRL evaluation.
    ///
    /// The expression has access to the following variables:
    /// - `.request`: The incoming HTTP request, including headers, query
    ///   parameters, the parsed GraphQL operation, and `url_matches`
    ///   (path parameters captured from `http.graphql_endpoint`).
    /// - `.default`: The original URL of the subgraph (from supergraph SDL).
    /// - `.subgraph.name`: The name of the subgraph the URL is being
    ///   resolved for.
    ///
    /// ### Example
    /// ```yaml
    /// override_subgraph_urls:
    ///   subgraphs:
    ///     accounts: "https://accounts.example.com/graphql"
    ///     products:
    ///       expression: |
    ///         if .request.headers."x-region" == "us-east" {
    ///           "https://products-us-east.example.com/graphql"
    ///         } else {
    ///           .default
    ///         }
    /// ```
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub subgraphs: HashMap<String, UrlOrExpression>,

    /// Override applied to every subgraph that does not have its own
    /// per-subgraph override under `subgraphs.<name>`. Useful when the
    /// override logic is the same (or only depends on the subgraph name)
    /// for every subgraph in the supergraph.
    ///
    /// The expression has access to the following variables:
    /// - `.request`: The incoming HTTP request, including headers, query
    ///   parameters, the parsed GraphQL operation, and `url_matches`
    ///   (path parameters captured from `http.graphql_endpoint`, e.g.
    ///   `/{tenant}/graphql`).
    /// - `.default`: The original URL of the subgraph (from supergraph SDL).
    /// - `.subgraph.name`: The name of the subgraph the URL is being
    ///   resolved for.
    ///
    /// ### Example
    /// ```yaml
    /// override_subgraph_urls:
    ///   all:
    ///     expression: |
    ///       if .subgraph.name == "products" && .request.headers."x-region" == "us-east" {
    ///         "https://products-us-east.example.com/graphql"
    ///       } else {
    ///         .default
    ///       }
    /// ```
    ///
    /// ### Path parameter example
    /// When `http.graphql_endpoint` is set to `/{tenant}/graphql`, every
    /// path parameter captured from the request path becomes available
    /// under `.request.url_matches`:
    /// ```yaml
    /// override_subgraph_urls:
    ///   all:
    ///     expression: |
    ///       tenant = string!(.request.url_matches.tenant)
    ///       replace(string!(.default), "/api/", "/api/" + tenant + "/")
    /// ```
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all: Option<UrlOrExpression>,
}

impl OverrideSubgraphUrlsConfig {
    /// Returns the per-subgraph override configured for `subgraph_name`, if any.
    pub fn get_subgraph_url(&self, subgraph_name: &str) -> Option<&UrlOrExpression> {
        self.subgraphs.get(subgraph_name)
    }

    /// Returns the global (`all`) override, if any.
    pub fn get_all_url(&self) -> Option<&UrlOrExpression> {
        self.all.as_ref()
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(untagged)]
pub enum UrlOrExpression {
    /// A static URL string.
    Url(String),
    /// A dynamic value computed by a VRL expression.
    Expression { expression: String },
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
        UrlOrExpression::Url("https://accounts.example.com/graphql".to_string()),
    );
    subgraphs.insert(
        "products".to_string(),
        UrlOrExpression::Expression {
            expression: expression.to_string(),
        },
    );

    OverrideSubgraphUrlsConfig {
        subgraphs,
        all: None,
    }
}
