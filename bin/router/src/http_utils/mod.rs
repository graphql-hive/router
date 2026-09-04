// The `graphiql` feature skips Laboratory asset generation, so there is no page to seed.
pub mod body;
pub(crate) mod headers;
#[cfg(not(feature = "graphiql"))]
pub mod laboratory;
pub mod landing_page;
pub mod probes;
