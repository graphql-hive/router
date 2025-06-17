use async_graphql::ServerError;
use graphql_tools::validation::utils::ValidationError;

pub fn from_validation_error_to_server_error(
    val: &ValidationError,
) -> ServerError {
    ServerError::new(
        &val.message,
        Some(
            async_graphql::Pos {
                line: val.locations.first().map_or(1, |pos| pos.line),
                column: val.locations.first().map_or(1, |pos| pos.column),
            }
        )
    )
}
