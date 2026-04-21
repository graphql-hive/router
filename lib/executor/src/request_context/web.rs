use ntex::web;

use crate::request_context::{RequestContextError, SharedRequestContext};

pub trait RequestContextExt {
    fn read_request_context(&self) -> Result<SharedRequestContext, RequestContextError>;
    fn has_request_context(&self) -> bool;
    fn write_request_context(&mut self, context: SharedRequestContext);
}

impl RequestContextExt for web::HttpRequest {
    #[inline]
    fn read_request_context(&self) -> Result<SharedRequestContext, RequestContextError> {
        self.extensions()
            .get::<SharedRequestContext>()
            .cloned()
            .ok_or(RequestContextError::Missing)
    }

    #[inline]
    fn has_request_context(&self) -> bool {
        self.extensions().get::<SharedRequestContext>().is_some()
    }

    #[inline]
    fn write_request_context(&mut self, context: SharedRequestContext) {
        self.extensions_mut().insert(context);
    }
}

impl RequestContextExt for web::WebRequest<web::DefaultError> {
    #[inline]
    fn read_request_context(&self) -> Result<SharedRequestContext, RequestContextError> {
        self.extensions()
            .get::<SharedRequestContext>()
            .cloned()
            .ok_or(RequestContextError::Missing)
    }

    #[inline]
    fn has_request_context(&self) -> bool {
        self.extensions().get::<SharedRequestContext>().is_some()
    }

    #[inline]
    fn write_request_context(&mut self, context: SharedRequestContext) {
        self.extensions_mut().insert(context);
    }
}
