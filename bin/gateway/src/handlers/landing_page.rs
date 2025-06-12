use axum::{extract::State, response::Html};
use std::sync::Arc;

use crate::AppState;

static LANDING_PAGE_HTML: &str = include_str!("../../static/landing-page.html");
static PRODUCT_LOGO_SVG: &str = include_str!("../../static/product_logo.svg");

pub async fn landing_page_handler(State(app_state): State<Arc<AppState>>) -> Html<String> {
    let mut subgraph_html = String::new();
    subgraph_html.push_str("<section class=\"supergraph-information\">");
    subgraph_html.push_str("<h3>Supergraph Status: Loaded ✅</h3>");
    subgraph_html.push_str(&format!(
        "<p><strong>Source: </strong> <i>{}</i></p>",
        app_state.supergraph_source
    ));
    subgraph_html.push_str("<table>");
    subgraph_html.push_str("<tr><th>Subgraph</th><th>Transport</th><th>Location</th></tr>");
    for (subgraph_name, subgraph_endpoint) in app_state.subgraph_endpoint_map.iter() {
        subgraph_html.push_str("<tr>");
        subgraph_html.push_str(&format!("<td>{}</td>", subgraph_name));
        subgraph_html.push_str(&format!(
            "<td>{}</td>",
            if subgraph_endpoint.starts_with("http") {
                "http"
            } else {
                "Unknown"
            }
        ));
        subgraph_html.push_str(&format!(
            "<td><a href=\"{}\">{}</a></td>",
            subgraph_endpoint, subgraph_endpoint
        ));
        subgraph_html.push_str("</tr>");
    }
    subgraph_html.push_str("</table>");
    subgraph_html.push_str("</section>");

    let rendered_html = LANDING_PAGE_HTML
        .replace("__GRAPHIQL_LINK__", "/graphql")
        .replace("__REQUEST_PATH__", "/")
        .replace("__PRODUCT_NAME__", "Hive Gateway RS (Axum)")
        .replace(
            "__PRODUCT_DESCRIPTION__",
            "A GraphQL Gateway written in Rust, powered by Axum",
        )
        .replace("__PRODUCT_PACKAGE_NAME__", "hive-gateway-rs")
        .replace(
            "__PRODUCT_LINK__",
            "https://the-guild.dev/graphql/hive/docs/gateway",
        )
        .replace("__PRODUCT_LOGO__", PRODUCT_LOGO_SVG)
        .replace("__SUBGRAPH_HTML__", &subgraph_html);

    Html(rendered_html)
}
