use axum::{http::Uri, response::Html};

static LANDING_PAGE_HTML: &str = include_str!("../../static/landing-page.html");
static PRODUCT_LOGO_SVG: &str = include_str!("../../static/product_logo.svg");

pub async fn landing_page_handler(uri: Uri) -> Html<String> {
    let rendered_html = LANDING_PAGE_HTML
        .replace("__GRAPHIQL_LINK__", "/graphql")
        .replace("__REQUEST_PATH__", uri.path())
        .replace("__PRODUCT_NAME__", "Hive Router")
        .replace(
            "__PRODUCT_DESCRIPTION__",
            "A GraphQL Gateway written in Rust",
        )
        .replace(
            "__PRODUCT_LINK__",
            "https://the-guild.dev/graphql/hive/docs/router",
        )
        .replace("__PRODUCT_LOGO__", PRODUCT_LOGO_SVG);

    Html(rendered_html)
}
