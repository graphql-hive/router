use http::header::{InvalidHeaderName, InvalidHeaderValue};
use regex_automata::meta::BuildError;

#[derive(thiserror::Error, Debug)]
pub enum HeaderRuleCompileError {
    #[error("Invalid header name '{0}'. Please check the configuration. Reason: {1}")]
    BadHeaderName(String, InvalidHeaderName),
    #[error("Invalid header value for header '{0}'. Please check the configuration. Reason: {1}")]
    BadHeaderValue(String, InvalidHeaderValue),
    #[error("The 'rename' option is only allowed when propagating a single header specified with 'named'. You cannot use 'rename' when propagating multiple headers or when using 'matching'.")]
    InvalidRename,
    #[error("The 'default' option is only allowed when propagating a single header specified with 'named'. You cannot use 'default' when propagating multiple headers or when using 'matching'.")]
    InvalidDefault,
    #[error("Failed to build regex for header matching. Please check your regex patterns for syntax errors. Reason: {0}")]
    RegexBuild(#[from] Box<BuildError>),
}
