use std::{marker::PhantomData, sync::MutexGuard};

use super::super::domains::{RequestContext, SharedRequestContext};
use super::super::RequestContextError;
use crate::hooks::PluginMarker;

impl SharedRequestContext {
    pub fn for_plugin<Plugin: PluginMarker>(&self) -> RequestContextPluginApi<Plugin> {
        RequestContextPluginApi::new(self.clone())
    }
}

#[derive(Clone)]
pub struct RequestContextPluginApi<Plugin> {
    context: SharedRequestContext,
    _plugin: PhantomData<Plugin>,
}

pub struct RequestContextPluginRead<Plugin> {
    pub(crate) snapshot: RequestContext,
    pub(crate) _plugin: PhantomData<Plugin>,
}

pub struct RequestContextPluginWrite<'a, Plugin> {
    pub(crate) context: MutexGuard<'a, RequestContext>,
    pub(crate) _plugin: PhantomData<Plugin>,
}

impl<Plugin: PluginMarker> RequestContextPluginApi<Plugin> {
    fn new(context: SharedRequestContext) -> RequestContextPluginApi<Plugin> {
        RequestContextPluginApi {
            context,
            _plugin: PhantomData,
        }
    }

    pub fn read(&self) -> Result<RequestContextPluginRead<Plugin>, RequestContextError> {
        Ok(RequestContextPluginRead {
            snapshot: self.context.snapshot()?,
            _plugin: PhantomData,
        })
    }

    pub fn write(&self) -> Result<RequestContextPluginWrite<'_, Plugin>, RequestContextError> {
        Ok(RequestContextPluginWrite {
            context: self.context.lock()?,
            _plugin: PhantomData,
        })
    }
}
