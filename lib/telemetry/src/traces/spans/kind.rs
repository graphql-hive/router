#[derive(Debug, strum::Display, strum::AsRefStr, strum::IntoStaticStr)]
#[strum(serialize_all = "kebab-case")]
#[non_exhaustive]
pub(crate) enum HiveSpanKind {
    HttpRequest,
    GraphqlParse,
    GraphqlValidate,
    GraphqlAuthorize,
    GraphqlNormalize,
    GraphqlPlan,
    GraphqlOperation,
    SubgraphGraphqlOperation,
}
