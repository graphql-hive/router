use http::{HeaderMap, HeaderName, HeaderValue};

use crate::telemetry::Injector;

pub struct HeaderMapInjector<'a>(&'a mut HeaderMap);

impl<'a> From<&'a mut HeaderMap> for HeaderMapInjector<'a> {
    fn from(value: &'a mut HeaderMap) -> Self {
        Self(value)
    }
}

impl Injector for HeaderMapInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        let Ok(name) = HeaderName::from_bytes(key.as_bytes()) else {
            return;
        };

        let Ok(val) = HeaderValue::from_str(&value) else {
            return;
        };

        self.0.insert(name, val);
    }
}
