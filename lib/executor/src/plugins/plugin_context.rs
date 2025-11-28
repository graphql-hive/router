use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use dashmap::{
    mapref::one::{Ref, RefMut},
    DashMap,
};
use http::Uri;
use ntex::router::Path;
use ntex_http::HeaderMap;

use crate::plugin_trait::RouterPluginBoxed;

pub struct RouterHttpRequest<'exec> {
    pub uri: &'exec Uri,
    pub method: &'exec http::Method,
    pub version: http::Version,
    pub headers: &'exec HeaderMap,
    pub path: &'exec str,
    pub query_string: &'exec str,
    pub match_info: &'exec Path<Uri>,
}

#[derive(Default)]
pub struct PluginContext {
    inner: DashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

pub struct PluginContextRefEntry<'a, T> {
    pub entry: Option<Ref<'a, TypeId, Box<dyn Any + Send + Sync>>>,
    phantom: std::marker::PhantomData<T>,
}

impl<'a, T: Any + Send + Sync> PluginContextRefEntry<'a, T> {
    pub fn get_ref(&self) -> Option<&T> {
        match &self.entry {
            None => None,
            Some(entry) => {
                let boxed_any = entry.value();
                Some(boxed_any.downcast_ref::<T>()?)
            }
        }
    }
}
pub struct PluginContextMutEntry<'a, T> {
    pub entry: Option<RefMut<'a, TypeId, Box<dyn Any + Send + Sync>>>,
    phantom: std::marker::PhantomData<T>,
}

impl<'a, T: Any + Send + Sync> PluginContextMutEntry<'a, T> {
    pub fn get_ref_mut(&mut self) -> Option<&mut T> {
        match &mut self.entry {
            None => None,
            Some(entry) => {
                let boxed_any = entry.value_mut();
                Some(boxed_any.downcast_mut::<T>()?)
            }
        }
    }
}

impl PluginContext {
    pub fn contains<T: Any + Send + Sync>(&self) -> bool {
        let type_id = TypeId::of::<T>();
        self.inner.contains_key(&type_id)
    }
    pub fn insert<T: Any + Send + Sync>(&self, value: T) -> Option<Box<T>> {
        let type_id = TypeId::of::<T>();
        self.inner
            .insert(type_id, Box::new(value))
            .and_then(|boxed_any| boxed_any.downcast::<T>().ok())
    }
    pub fn get_ref_entry<T: Any + Send + Sync>(&self) -> PluginContextRefEntry<'_, T> {
        let type_id = TypeId::of::<T>();
        let entry = self.inner.get(&type_id);
        PluginContextRefEntry {
            entry,
            phantom: std::marker::PhantomData,
        }
    }
    pub fn get_mut_entry<'a, T: Any + Send + Sync>(&'a self) -> PluginContextMutEntry<'a, T> {
        let type_id = TypeId::of::<T>();
        let entry = self.inner.get_mut(&type_id);

        PluginContextMutEntry {
            entry,
            phantom: std::marker::PhantomData,
        }
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

        let entry = ctx.get_ref_entry();
        let ctx_ref: &TestCtx = entry.get_ref().unwrap();
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
            let mut entry = ctx.get_mut_entry();
            let ctx_mut: &mut TestCtx = entry.get_ref_mut().unwrap();
            ctx_mut.value = 100;
        }

        let entry = ctx.get_ref_entry();
        let ctx_ref: &TestCtx = entry.get_ref().unwrap();
        assert_eq!(ctx_ref.value, 100);
    }
}
