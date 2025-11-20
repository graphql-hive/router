use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use dashmap::DashMap;
use http::Uri;
use ntex::router::Path;
use ntex_http::HeaderMap;

use crate::plugin_trait::RouterPlugin;

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
    inner: DashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl PluginContext {
    pub fn insert<T: Any + Send + Sync>(&self, value: T) {
        self.inner.insert(TypeId::of::<T>(), Arc::new(value));
    }
    pub fn get<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.inner
            .get(&TypeId::of::<T>())
            .map(|v| v.clone().downcast::<T>().ok().unwrap())
    }
}

pub struct PluginManager<'req> {
    pub plugins: Arc<Vec<Box<dyn RouterPlugin + Send + Sync>>>,
    pub router_http_request: RouterHttpRequest<'req>,
    pub context: Arc<PluginContext>,
}
