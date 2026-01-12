//! Graphql Parser
//! ==============
//!
//! This library contains full parser and formatter of the graphql
//! query language as well as AST types.
//!
//! Example: Parse and Format Query
//! -------------------------------
//!
//! ```rust
//! # extern crate graphql_tools;
//! use graphql_tools::parser::query::{parse_query, ParseError};
//!
//! # fn parse() -> Result<(), ParseError> {
//! let ast = parse_query::<&str>("query MyQuery { field1, field2 }")?;
//! // Format canonical representation
//! assert_eq!(format!("{}", ast), "\
//! query MyQuery {
//!   field1
//!   field2
//! }
//! ");
//! # Ok(())
//! # }
//! # fn main() {
//! #    parse().unwrap()
//! # }
//! ```
//!
//! Example: Parse and Format Schema
//! --------------------------------
//!
//! ```rust
//! # extern crate graphql_tools;
//! use graphql_tools::parser::schema::{parse_schema, ParseError};
//!
//! # fn parse() -> Result<(), ParseError> {
//! let ast = parse_schema::<String>(r#"
//!     schema {
//!         query: Query
//!     }
//!     type Query {
//!         users: [User!]!,
//!     }
//!     """
//!        Example user object
//!
//!        This is just a demo comment.
//!     """
//!     type User {
//!         name: String!,
//!     }
//! "#)?.to_owned();
//! // Format canonical representation
//! assert_eq!(format!("{}", ast), "\
//! schema {
//!   query: Query
//! }
//!
//! type Query {
//!   users: [User!]!
//! }
//!
//! \"\"\"
//!   Example user object
//!
//!   This is just a demo comment.
//! \"\"\"
//! type User {
//!   name: String!
//! }
//! ");
//! # Ok(())
//! # }
//! # fn main() {
//! #    parse().unwrap()
//! # }
//! ```
//!

mod common;
#[macro_use]
mod format;
mod helpers;
mod position;
mod tokenizer;

pub mod query;
pub mod schema;

pub use format::Style;
pub use position::Pos;
pub use query::parse_query;
pub use query::{minify_query, minify_query_document};
pub use schema::parse_schema;
