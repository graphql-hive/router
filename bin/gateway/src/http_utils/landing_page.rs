use http::{header::CONTENT_TYPE, StatusCode};
use ntex::{http::ResponseBuilder, web::Responder};

static LANDING_PAGE_HTML: &str = include_str!("../../static/landing-page.html");
static PRODUCT_LOGO_SVG: &str = include_str!("../../static/product_logo.svg");

pub async fn landing_page_handler() -> impl Responder {
    let rendered_html = LANDING_PAGE_HTML
        .replace("__GRAPHIQL_LINK__", "/graphql")
        .replace("__PRODUCT_NAME__", "Hive Gateway RS")
        .replace(
            "__PRODUCT_DESCRIPTION__",
            "A GraphQL Gateway written in Rust",
        )
        .replace("__PRODUCT_PACKAGE_NAME__", "hive-gateway-rs")
        .replace(
            "__PRODUCT_LINK__",
            "https://the-guild.dev/graphql/hive/docs/gateway",
        )
        .replace("__PRODUCT_LOGO__", PRODUCT_LOGO_SVG);

    ResponseBuilder::new(StatusCode::OK)
        .header(CONTENT_TYPE, "text/html")
        .body(rendered_html)
}
