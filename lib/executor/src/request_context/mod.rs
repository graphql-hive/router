mod api;
mod deser;
mod domains;
mod error;
mod web;

pub use api::coprocessor::RequestContextPatch;
pub use api::plugin::RequestContextPluginApi;
pub use domains::persisted_documents::{
    RequestContextPersistedDocumentsRead, RequestContextPersistedDocumentsWrite,
};
pub use domains::{RequestContext, SelectedRequestContext, SharedRequestContext};
pub use error::RequestContextError;
pub use web::RequestContextExt;
