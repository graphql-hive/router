use std::{marker::PhantomData, sync::MutexGuard};

use super::super::domains::{RequestContext, SharedRequestContext};
use super::super::RequestContextError;
use crate::hooks::HookMarker;

impl SharedRequestContext {
    /// Returns a scoped API handle for plugins.
    /// The `Hook` generic determines the available capabilities (read/write permissions)
    /// based on the current plugin's hook.
    pub fn for_plugin<Hook: HookMarker>(&self) -> RequestContextPluginApi<Hook> {
        RequestContextPluginApi::new(self.clone())
    }
}

/// A type-safe API for router plugins to interact with the request context.
/// The `Hook` generic is a marker type (`OnQueryPlan`) used to enforce
/// hook-specific permissions at compile time.
#[derive(Clone)]
pub struct RequestContextPluginApi<Hook> {
    context: SharedRequestContext,
    _hook: PhantomData<Hook>,
}

/// A read-only snapshot of the request context.
/// Holding this struct allows plugins to perform async work (across `.await`)
/// without blocking other plugins or the main execution flow.
pub struct RequestContextPluginRead<Hook> {
    pub(crate) snapshot: RequestContext,
    pub(crate) _hook: PhantomData<Hook>,
}

/// A synchronous write guard for the request context.
/// This struct holds a [MutexGuard] in the context.
/// To avoid deadlocks or long-lived pipeline blocks,
/// it should never be held across `.await`.
pub struct RequestContextPluginWrite<'a, Hook> {
    pub(crate) context: MutexGuard<'a, RequestContext>,
    pub(crate) _hook: PhantomData<Hook>,
}

impl<Hook: HookMarker> RequestContextPluginApi<Hook> {
    fn new(context: SharedRequestContext) -> RequestContextPluginApi<Hook> {
        RequestContextPluginApi {
            context,
            _hook: PhantomData,
        }
    }

    /// Creates a read-only snapshot of the current context.
    pub fn read(&self) -> Result<RequestContextPluginRead<Hook>, RequestContextError> {
        Ok(RequestContextPluginRead {
            snapshot: self.context.snapshot()?,
            _hook: PhantomData,
        })
    }

    /// Acquires a write lock on the context.
    pub fn write(&self) -> Result<RequestContextPluginWrite<'_, Hook>, RequestContextError> {
        Ok(RequestContextPluginWrite {
            context: self.context.read_lock()?,
            _hook: PhantomData,
        })
    }
}
