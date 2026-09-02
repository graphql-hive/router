use ntex_http::{header, HeaderMap, HeaderValue};

pub(crate) fn append_vary(headers: &mut HeaderMap, token: &str) {
    if let Some(existing) = headers.get(header::VARY).and_then(|v| v.to_str().ok()) {
        if existing
            .split(',')
            .map(|s| s.trim())
            .any(|t| t.eq_ignore_ascii_case(token))
        {
            // already present
            return;
        }

        let new_header_value = if existing.is_empty() {
            HeaderValue::from_str(token)
        } else {
            HeaderValue::from_str(&format!("{}, {}", existing, token))
        };

        if let Ok(v) = new_header_value {
            headers.insert(header::VARY, v);
        }

        return;
    }

    if let Ok(v) = HeaderValue::from_str(token) {
        headers.insert(header::VARY, v);
    }
}
