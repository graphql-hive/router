use std::sync::MutexGuard;

use crate::request_context::{RequestContext, RequestContextError, SharedRequestContext};

impl SharedRequestContext {
    pub fn for_plugin(&self) -> RequestContextPluginApi {
        RequestContextPluginApi::new(self.clone())
    }
}

#[derive(Clone)]
pub struct RequestContextPluginApi {
    context: SharedRequestContext,
}

pub struct RequestContextPluginRead {
    pub(crate) snapshot: RequestContext,
}

pub struct RequestContextPluginWrite<'a> {
    pub(crate) context: MutexGuard<'a, RequestContext>,
}

impl RequestContextPluginApi {
    fn new(context: SharedRequestContext) -> RequestContextPluginApi {
        RequestContextPluginApi { context }
    }

    pub fn read(&self) -> Result<RequestContextPluginRead, RequestContextError> {
        Ok(RequestContextPluginRead {
            snapshot: self.context.snapshot()?,
        })
    }

    pub fn write(&self) -> Result<RequestContextPluginWrite<'_>, RequestContextError> {
        Ok(RequestContextPluginWrite {
            context: self.context.lock()?,
        })
    }
}
