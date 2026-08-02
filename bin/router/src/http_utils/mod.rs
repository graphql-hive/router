// The `graphiql` feature skips Laboratory asset generation, so there is no page to seed.
#[cfg(not(feature = "graphiql"))]
pub mod laboratory;
pub mod landing_page;
pub mod probes;
