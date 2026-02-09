#[derive(
    Debug,
    strum::Display,
    strum::AsRefStr,
    strum::IntoStaticStr,
    strum::EnumString,
    PartialEq,
    Clone,
)]
#[non_exhaustive]
pub enum HiveSpanKind {
    #[strum(serialize = "http.server")]
    HttpServerRequest,
    #[strum(serialize = "http.client")]
    HttpClientRequest,
    #[strum(serialize = "http.inflight")]
    HttpInflightRequest,
    #[strum(serialize = "graphql.parse")]
    GraphqlParse,
    #[strum(serialize = "graphql.validate")]
    GraphqlValidate,
    #[strum(serialize = "graphql.variable_coercion")]
    GraphqlVariableCoercion,
    #[strum(serialize = "graphql.authorize")]
    GraphqlAuthorize,
    #[strum(serialize = "graphql.normalize")]
    GraphqlNormalize,
    #[strum(serialize = "graphql.plan")]
    GraphqlPlan,
    #[strum(serialize = "graphql.execute")]
    GraphqlExecute,
    #[strum(serialize = "graphql.operation")]
    GraphqlOperation,
    #[strum(serialize = "graphql.subgraph.operation")]
    GraphQLSubgraphOperation,
}

impl HiveSpanKind {
    pub fn as_str(&self) -> &'static str {
        self.into()
    }
}

#[derive(
    Debug, strum::Display, strum::AsRefStr, strum::IntoStaticStr, strum::EnumString, PartialEq,
)]
#[non_exhaustive]
pub enum HiveEventKind {
    #[strum(serialize = "graphql.error")]
    GraphQLError,
}
