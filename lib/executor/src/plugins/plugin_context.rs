use std::{
    any::{Any, TypeId},
    ops::{Deref, DerefMut},
    sync::Arc,
};

use dashmap::{
    mapref::one::{Ref, RefMut},
    DashMap,
};
use http::Uri;
use ntex::router::Path;
use ntex::{http::HeaderMap, web::HttpRequest};

use crate::plugin_trait::RouterPluginBoxed;

pub struct RouterHttpRequest<'req> {
    pub uri: &'req Uri,
    pub method: &'req http::Method,
    pub version: http::Version,
    pub headers: &'req HeaderMap,
    pub path: &'req str,
    pub query_string: &'req str,
    pub match_info: &'req Path<Uri>,
}

impl<'a> From<&'a HttpRequest> for RouterHttpRequest<'a> {
    fn from(req: &'a HttpRequest) -> Self {
        RouterHttpRequest {
            uri: req.uri(),
            method: req.method(),
            version: req.version(),
            headers: req.headers(),
            match_info: req.match_info(),
            query_string: req.query_string(),
            path: req.path(),
        }
    }
}

#[derive(Default)]
pub struct PluginContext {
    inner: DashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

pub struct PluginContextRefEntry<'a, T> {
    pub entry: Ref<'a, TypeId, Box<dyn Any + Send + Sync>>,
    phantom: std::marker::PhantomData<T>,
}

impl<'a, T: Any + Send + Sync> AsRef<T> for PluginContextRefEntry<'a, T> {
    fn as_ref(&self) -> &T {
        let boxed_any = self.entry.value();
        boxed_any
            .downcast_ref::<T>()
            .expect("type mismatch in PluginContextRefEntry")
    }
}

impl<'a, T: Any + Send + Sync> Deref for PluginContextRefEntry<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

pub struct PluginContextMutEntry<'a, T> {
    pub entry: RefMut<'a, TypeId, Box<dyn Any + Send + Sync>>,
    phantom: std::marker::PhantomData<T>,
}

impl<'a, T: Any + Send + Sync> AsRef<T> for PluginContextMutEntry<'a, T> {
    fn as_ref(&self) -> &T {
        let boxed_any = self.entry.value();
        boxed_any
            .downcast_ref::<T>()
            .expect("type mismatch in PluginContextMutEntry")
    }
}

impl<'a, T: Any + Send + Sync> Deref for PluginContextMutEntry<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<'a, T: Any + Send + Sync> AsMut<T> for PluginContextMutEntry<'a, T> {
    fn as_mut(&mut self) -> &mut T {
        let boxed_any = self.entry.value_mut();
        boxed_any
            .downcast_mut::<T>()
            .expect("type mismatch in PluginContextMutEntry")
    }
}

impl<'a, T: Any + Send + Sync> DerefMut for PluginContextMutEntry<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.as_mut()
    }
}

impl PluginContext {
    /// Check if the context contains an entry of type T.
    ///
    /// This can be used by plugins to check for the presence of other plugins' context entries before trying to access them.
    ///
    /// Example:
    /// ```rust
    /// use hive_router::{
    ///    plugins::hooks::on_execute::{OnExecuteStartHookPayload, OnExecuteStartHookResult},
    /// };
    ///
    /// struct ContextData {
    ///     pub greetings: String
    /// }
    ///
    /// async fn on_execute<'exec>(&'exec self, payload: OnExecuteStartHookPayload<'exec>) -> OnExecuteStartHookResult<'exec> {
    ///     if payload.context.contains::<ContextData>() {
    ///         // safe to access ContextData entry
    ///     }
    ///     payload.proceed()
    /// }
    /// ```
    pub fn contains<T: Any + Send + Sync>(&self) -> bool {
        let type_id = TypeId::of::<T>();
        self.inner.contains_key(&type_id)
    }
    /// Insert a value of type T into the context.
    /// If an entry of that type already exists, it will be replaced and the old value will be returned.
    ///
    /// Example:
    /// ```rust
    /// use hive_router::{
    ///    plugins::hooks::on_execute::{OnExecuteStartHookPayload, OnExecuteStartHookResult},
    /// };
    ///
    /// struct ContextData {
    ///     pub greetings: String
    /// }
    ///
    /// async fn on_execute<'exec>(&'exec self, mut payload: OnExecuteStartHookPayload<'exec>) -> OnExecuteStartHookResult<'exec> {
    ///     let context_data = ContextData {
    ///         greetings: "Hello from context!".to_string()
    ///     };
    ///     payload.context.insert(context_data);
    ///     payload.proceed()
    /// }
    ///
    /// ```
    pub fn insert<T: Any + Send + Sync>(&self, value: T) -> Option<Box<T>> {
        let type_id = TypeId::of::<T>();
        self.inner
            .insert(type_id, Box::new(value))
            .and_then(|boxed_any| boxed_any.downcast::<T>().ok())
    }
    /// Get an immutable reference to the entry of type T in the context, if it exists.
    /// If no entry of that type exists, None is returned.
    ///
    /// Example:
    /// ```rust
    /// use hive_router::{
    ///    plugins::hooks::on_execute::{OnExecuteStartHookPayload, OnExecuteStartHookResult},
    /// };
    ///
    /// struct ContextData {
    ///     pub greetings: String
    /// }
    ///
    /// let context_data = ContextData {
    ///     greetings: "Hello from context!".to_string()
    /// };
    ///
    /// async fn on_execute<'exec>(&'exec self, mut payload: OnExecuteStartHookPayload<'exec>) -> OnExecuteStartHookResult<'exec> {
    ///     payload.context.insert(context_data);
    ///
    ///     let context_data_entry = payload.context.get_ref::<ContextData>();
    ///     if let Some(ref context_data) = context_data_entry {
    ///        println!("{}", context_data.greetings); // prints "Hello from context!"
    ///     }
    ///     payload.proceed()
    /// }
    /// ```
    pub fn get_ref<'a, T: Any + Send + Sync>(&'a self) -> Option<PluginContextRefEntry<'a, T>> {
        let type_id = TypeId::of::<T>();
        self.inner.get(&type_id).map(|entry| PluginContextRefEntry {
            entry,
            phantom: std::marker::PhantomData,
        })
    }
    /// Get a mutable reference to the entry of type T in the context, if it exists.
    /// If no entry of that type exists, None is returned.
    ///
    /// Example:
    /// ```rust
    /// use hive_router::{
    ///    plugins::hooks::on_execute::{OnExecuteStartHookPayload, OnExecuteStartHookResult},
    /// };
    ///
    /// struct ContextData {
    ///     pub greetings: String
    /// }
    ///
    /// let context_data = ContextData {
    ///     greetings: "Hello from context!".to_string()
    /// };
    ///
    /// async fn on_execute<'exec>(&'exec self, mut payload: OnExecuteStartHookPayload<'exec>) -> OnExecuteStartHookResult<'exec> {
    ///     payload.context.insert(context_data);
    ///     if let Some(mut context_data_entry) = payload.context.get_mut::<ContextData>() {
    ///        context_data_entry.greetings = "Hello from mutable reference!".to_string();
    ///     }
    ///     payload.proceed()
    /// }
    /// ```
    pub fn get_mut<'a, T: Any + Send + Sync>(&'a self) -> Option<PluginContextMutEntry<'a, T>> {
        let type_id = TypeId::of::<T>();
        self.inner
            .get_mut(&type_id)
            .map(|entry| PluginContextMutEntry {
                entry,
                phantom: std::marker::PhantomData,
            })
    }
}

pub struct PluginRequestState<'req> {
    pub plugins: Arc<Vec<RouterPluginBoxed>>,
    pub router_http_request: RouterHttpRequest<'req>,
    pub context: Arc<PluginContext>,
}

#[cfg(test)]
mod tests {
    #[test]
    fn inserts_and_gets_immut_ref() {
        use super::PluginContext;

        struct TestCtx {
            pub value: u32,
        }

        let ctx = PluginContext::default();
        ctx.insert(TestCtx { value: 42 });

        let ctx_ref: &TestCtx = &ctx.get_ref().unwrap();
        assert_eq!(ctx_ref.value, 42);
    }
    #[test]
    fn inserts_and_mutates_with_mut_ref() {
        use super::PluginContext;

        struct TestCtx {
            pub value: u32,
        }

        let ctx = PluginContext::default();
        ctx.insert(TestCtx { value: 42 });

        {
            let ctx_mut: &mut TestCtx = &mut ctx.get_mut().unwrap();
            ctx_mut.value = 100;
        }

        let ctx_ref: &TestCtx = &ctx.get_ref().unwrap();
        assert_eq!(ctx_ref.value, 100);
    }
}
