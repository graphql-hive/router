use axum::response::Html;

static GRAPHIQL_HTML: &str = include_str!("../../static/graphiql.html");

pub async fn graphiql_handler() -> Html<&'static str> {
    Html(GRAPHIQL_HTML)
}
