use std::borrow::Cow;

use crate::pipeline::persisted_documents::extract::HttpRequestContext;

use super::super::super::types::PersistedDocumentId;
use super::super::core::{DocumentIdSourceExtractor, ExtractionContext};

/// Extracts a value from the URL query string.
pub(crate) struct UrlQueryParamExtractor {
    pub(crate) name: String,
}

impl DocumentIdSourceExtractor for UrlQueryParamExtractor {
    fn extract(&self, ctx: &ExtractionContext<'_>) -> Option<PersistedDocumentId> {
        ctx.query_param(&self.name)
            .and_then(|value| PersistedDocumentId::try_from(value.as_ref()).ok())
    }
}

impl<'a> HttpRequestContext<'a> {
    pub fn query_param(&self, name: &str) -> Option<Cow<'a, str>> {
        self.query.as_ref()?.get(name)
    }
}

pub(crate) struct QueryParams<'a> {
    raw: &'a str,
}

/// I tried to use different for url decoding, and query params parsing,
/// but they all either allocated entire HashMaps or were slow.
/// The difference sometimes was 10ns vs 400ns,
/// Especially on large set of query params, where the key was not found.
/// I decided to implement a custom query params parser that does not allocate,
/// and use url decoding crate, to perform safer decoding.
impl<'a> QueryParams<'a> {
    pub(crate) fn new(raw: &'a str) -> Self {
        Self { raw }
    }

    pub(crate) fn get(&self, name: &str) -> Option<Cow<'a, str>> {
        let value = Self::find_first_value(self.raw, name)?;
        Self::decode_if_needed(value)
    }

    #[inline]
    fn find_first_value<'b>(query: &'b str, name: &str) -> Option<&'b str> {
        let bytes = query.as_bytes();
        let name_bytes = name.as_bytes();

        if name_bytes.is_empty() {
            return None;
        }

        // First-match semantics:
        // - first `name=value` returns `Some(value)`
        // - first `name` or `name=` returns `None`
        // - once the first `name` is seen, later duplicates are ignored
        for idx in memchr::memchr_iter(name_bytes[0], bytes) {
            // If it is not preceded by a '&' or it's not the first character, skip it.
            // Example:
            //  - /graphql?bar=1&foo=2
            //  - /graphql?foo=3
            //  - /graphql?foo=3&bar=1
            //  - /graphql?xfoo=3      [continue]
            //  - /graphql?bar=1foo    [continue]
            if !Self::is_pair_boundary(bytes, idx) {
                continue;
            }

            // Confirm full key match at this boundary.
            let Some(key_end) = Self::match_key_at(bytes, idx, name_bytes) else {
                continue;
            };

            // Bare key at end (`...&name`) is treated as empty,
            // so we return None to indicate the key is present but has no value.
            if key_end == bytes.len() {
                return None;
            }

            // Separator after key:
            // - `name&`
            // - '=' => key-value pair, continue parsing
            // - other => prefix match like `names=...`, keep scanning
            let separator = bytes[key_end];
            // `name&` means the key is present but has no value, so we return None.
            if separator == b'&' {
                return None;
            }

            // `names` is a prefix match, so we keep scanning.
            if separator != b'=' {
                continue;
            }

            // `name=` and `name=&...` are treated as no value, so we return None.
            let value_start = key_end + 1;
            if value_start >= bytes.len() || bytes[value_start] == b'&' {
                return None;
            }

            let suffix = &bytes[value_start..];
            // Find the next `&` or end of string, if any.
            let value_end = if let Some(offset) = memchr::memchr(b'&', suffix) {
                value_start + offset
            } else {
                query.len()
            };

            // Value is present, return it.
            if value_start < value_end {
                return Some(&query[value_start..value_end]);
            }
        }

        None
    }

    #[inline]
    /// Returns `true` if the byte at `idx` is the start of a key-value pair boundary.
    /// It is either the start of the query string or the previous character is `&`.
    fn is_pair_boundary(bytes: &[u8], idx: usize) -> bool {
        idx == 0 || bytes[idx - 1] == b'&'
    }

    #[inline]
    fn match_key_at(bytes: &[u8], idx: usize, name_bytes: &[u8]) -> Option<usize> {
        let key_end = idx + name_bytes.len();
        // Key end is beyond the end of the query string, skip it.
        if key_end > bytes.len() {
            return None;
        }

        // Key does not match, skip it.
        if &bytes[idx..key_end] != name_bytes {
            return None;
        }

        Some(key_end)
    }

    /// Decode url encoded value if necessary.
    fn decode_if_needed<'b>(value: &'b str) -> Option<Cow<'b, str>> {
        let value_bytes = value.as_bytes();
        let percent_at = memchr::memchr(b'%', value_bytes);
        let plus_at = memchr::memchr(b'+', value_bytes);

        if percent_at.is_none() && plus_at.is_none() {
            // No need to decode, return as is.
            return Some(Cow::Borrowed(value));
        }

        let Some(plus_at) = plus_at else {
            return percent_encoding::percent_decode(value_bytes)
                .decode_utf8()
                .ok();
        };

        // Special case we need to handle.
        // `+` is a space character in url encoding, so we replace it with a space.
        // I tried to use form_urlencoded crate but it was 4x slower than this.
        // The percent_encoding does not handle `+` as a space character, so we replace it first.
        // That's why we use Cow::Owned here, and Cow in general to avoid allocations.
        let replaced = Self::replace_plus(value_bytes, plus_at);

        let decoded = percent_encoding::percent_decode(&replaced)
            .decode_utf8()
            .ok()?;
        Some(Cow::Owned(decoded.into_owned()))
    }

    fn replace_plus(input: &[u8], first_position: usize) -> Cow<'_, [u8]> {
        let mut replaced = input.to_owned();
        replaced[first_position] = b' ';
        for byte in &mut replaced[first_position + 1..] {
            if *byte == b'+' {
                *byte = b' ';
            }
        }
        Cow::Owned(replaced)
    }
}

#[cfg(test)]
mod tests {
    use super::QueryParams;

    fn query_param(raw_query: &str, name: &str) -> Option<String> {
        QueryParams::new(raw_query)
            .get(name)
            .map(|value| value.into_owned())
    }

    #[test]
    fn query_params_lookup_rules() {
        let cases = [
            ("key=first&key=second", "key", Some("first")),
            ("key=&key=second", "key", None),
            ("key&key=second", "key", None),
            ("keys=1&key=value", "key", Some("value")),
            ("xkey=1&key=value", "key", Some("value")),
            ("foo=bar", "key", None),
            ("", "key", None),
            ("key=value", "", None),
        ];

        for (query, name, expected) in cases {
            let actual = query_param(query, name);
            assert_eq!(
                actual.as_deref(),
                expected,
                "query='{query}', name='{name}'"
            );
        }
    }

    #[test]
    fn query_params_decoding_rules() {
        let cases = [
            ("key=a+b", Some("a b")),
            ("key=a%2Bb", Some("a+b")),
            ("key=sha256%3Aabc", Some("sha256:abc")),
            ("key=abc%ZZ", Some("abc%ZZ")),
        ];

        for (query, expected) in cases {
            let actual = query_param(query, "key");
            assert_eq!(actual.as_deref(), expected, "query='{query}'");
        }
    }
}
