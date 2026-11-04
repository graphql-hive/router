use http::{header::CONTENT_TYPE, StatusCode};
use ntex::{http::ResponseBuilder, web::Responder};

use crate::RouterAppState;

static LANDING_PAGE_HTML: &str = include_str!("../../static/landing-page.html");
static PRODUCT_LOGO_SVG: &str = include_str!("../../static/product_logo.svg");

// responders are typed by the app state of the app serving them
pub async fn landing_page_handler(
    graphql_endpoint: String,
) -> impl Responder<RouterAppState> {
    let rendered_html = LANDING_PAGE_HTML
        .replace("__LABORATORY_LINK__", &graphql_endpoint)
        .replace("__PRODUCT_NAME__", "Hive Router")
        .replace(
            "__PRODUCT_DESCRIPTION__",
            "A GraphQL Router written in Rust",
        )
        .replace("__PRODUCT_PACKAGE_NAME__", "router")
        .replace("__PRODUCT_LINK__", "https://github.com/graphql-hive/router")
        .replace("__PRODUCT_LOGO__", PRODUCT_LOGO_SVG);

    ResponseBuilder::new(StatusCode::OK)
        .header(CONTENT_TYPE, "text/html")
        .body(rendered_html)
}
