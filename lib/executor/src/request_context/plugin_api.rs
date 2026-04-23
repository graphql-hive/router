use std::{marker::PhantomData, sync::MutexGuard};

use crate::{
    hooks::PluginMarker,
    request_context::{RequestContext, RequestContextError, SharedRequestContext},
};

impl SharedRequestContext {
    pub fn for_plugin<Caps: PluginMarker>(&self) -> RequestContextPluginApi<Caps> {
        RequestContextPluginApi::new(self.clone())
    }
}

#[derive(Clone)]
pub struct RequestContextPluginApi<Caps> {
    context: SharedRequestContext,
    _caps: PhantomData<Caps>,
}

pub struct RequestContextPluginRead<Caps> {
    pub(crate) snapshot: RequestContext,
    pub(crate) _caps: PhantomData<Caps>,
}

pub struct RequestContextPluginWrite<'a, Caps> {
    pub(crate) context: MutexGuard<'a, RequestContext>,
    pub(crate) _caps: PhantomData<Caps>,
}

impl<Caps: PluginMarker> RequestContextPluginApi<Caps> {
    fn new(context: SharedRequestContext) -> RequestContextPluginApi<Caps> {
        RequestContextPluginApi {
            context,
            _caps: PhantomData,
        }
    }

    pub fn read(&self) -> Result<RequestContextPluginRead<Caps>, RequestContextError> {
        Ok(RequestContextPluginRead {
            snapshot: self.context.snapshot()?,
            _caps: PhantomData,
        })
    }

    pub fn write(&self) -> Result<RequestContextPluginWrite<'_, Caps>, RequestContextError> {
        Ok(RequestContextPluginWrite {
            context: self.context.lock()?,
            _caps: PhantomData,
        })
    }
}
