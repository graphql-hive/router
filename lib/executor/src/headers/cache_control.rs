use crate::headers::{plan::HeaderAggregationStrategy, response::ResponseHeaderAggregator};
use http::HeaderValue;
use tracing::warn;

#[derive(Clone, Default)]
struct ParsedCacheControl {
    no_store: bool,
    no_cache: bool,
    must_revalidate: bool,
    is_private: bool,
    is_public: bool,
    max_age: Option<u32>,
}

fn parse(header: &str) -> Option<ParsedCacheControl> {
    let trimmed = header.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut p = ParsedCacheControl::default();

    for part in trimmed.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (token, value) = match part.split_once('=') {
            Some((t, v)) => (t.trim(), Some(v.trim())),
            None => (part, None),
        };
        match token.to_ascii_lowercase().as_str() {
            "no-store" => p.no_store = true,
            "no-cache" => p.no_cache = true,
            "must-revalidate" => p.must_revalidate = true,
            "private" => p.is_private = true,
            "public" => p.is_public = true,
            "max-age" => {
                if let Some(v) = value {
                    match v.parse::<u32>() {
                        Ok(n) => p.max_age = Some(n),
                        Err(_) => {
                            tracing::warn!("cache-control max-age has non-numeric value: {v}")
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Some(p)
}

fn merge_into(acc: &mut Option<ParsedCacheControl>, incoming: ParsedCacheControl) {
    let Some(existing) = acc else {
        *acc = Some(incoming);
        return;
    };

    if existing.no_store
        || existing.no_cache
        || existing.is_private
        || incoming.no_store
        || incoming.no_cache
        || incoming.is_private
    {
        *existing = ParsedCacheControl {
            no_store: true,
            no_cache: true,
            ..Default::default()
        };
        return;
    }

    existing.is_public = existing.is_public && incoming.is_public;
    existing.must_revalidate = existing.must_revalidate || incoming.must_revalidate;
    existing.max_age = match (existing.max_age, incoming.max_age) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };
}

fn to_header_value(p: &ParsedCacheControl) -> String {
    if p.no_store || p.no_cache || p.is_private {
        return "no-store, no-cache".to_string();
    }

    let mut parts: Vec<String> = Vec::new();

    if p.is_public {
        parts.push("public".to_string());
    } else if p.is_private {
        parts.push("private".to_string());
    }

    if let Some(age) = p.max_age {
        parts.push(format!("max-age={age}"));
    }

    if p.must_revalidate {
        parts.push("must-revalidate".to_string());
    }

    parts.join(", ")
}

/// Collapse all accumulated `Cache-Control` header values in the aggregator into
/// a single, restrictively merged value and write it back, replacing whatever was
/// there before.
///
/// After all subgraph responses have been written into the `ResponseHeaderAggregator`
/// via the normal header propagation rules, `finalize` is called once before the
/// aggregator is flushed to the client response. At that point `aggregator.entries`
/// may contain zero, one, or many `Cache-Control` values depending on how many
/// subgraphs sent the header and whether any response-header rules also propagated it.
///
/// If the aggregator contains no `Cache-Control` entry at all (no subgraph sent it an
/// no propagation rule added it), the function returns immediately without inserting
/// anything. The header is left absent from the client response.
///
/// When `force_no_store` is `true` the caller has determined that caching must be
/// unconditionally forbidden - for example because the operation is a mutation, a
/// subgraph returned a GraphQL `errors` array, or a network-level error occurred.
/// The function overwrites whatever is in the aggregator with
/// `no-store, no-cache, must-revalidate` and returns. This path also exits early if
/// no `Cache-Control` entry exists, matching the behaviour of the normal path (we only
/// emit a header when a subgraph sent one first).
///
/// When `force_no_store` is `false` the raw string values stored in the aggregator are
/// parsed and folded left-to-right with the following policy:
///
/// 1. Poison check - if any value contains `no-store`, `no-cache`, or `private`, the
///    accumulated result is immediately locked to `no-store, no-cache` and all remaining
///    directives (`public`, `max-age`, `must-revalidate`) are discarded. Further
///    incoming values cannot "un-poison" this state.
/// 2. max-age - the minimum of all present `max-age` values is kept. A subgraph that
///    omits `max-age` entirely does not pull the min down; it is simply ignored for
///    this field, letting a shorter age set by another subgraph win.
/// 3. public - preserved only when every subgraph that sent `Cache-Control` also sent
///    `public`. A single subgraph that omits it is enough to strip `public` from the
///    result.
/// 4. must-revalidate - set if any subgraph sets it (logical OR). Cleared on poison.
///
/// The merged result is serialised back to a `HeaderValue` and re-inserted into the
/// aggregator under `Last` strategy so that any subsequent header-flush loop sees
/// exactly one value.
///
/// If every collected value was unparseable (e.g. non-UTF-8 bytes) the fold produces no
/// `acc` and the entry is left unchanged, meaning the last raw value written by the
/// propagation rules survives as-is.
pub fn finalize(aggregator: &mut ResponseHeaderAggregator, force_no_store: bool) {
    let Some((_, values)) = aggregator.entries.get(&http::header::CACHE_CONTROL) else {
        // there's no cache-control headers anywhere, so nothing to merge or poison - just leave it absent
        return;
    };

    if force_no_store {
        let value = HeaderValue::from_static("no-store, no-cache, must-revalidate");
        aggregator.entries.insert(
            http::header::CACHE_CONTROL,
            (HeaderAggregationStrategy::Last, vec![value]),
        );
        return;
    }

    let mut acc: Option<ParsedCacheControl> = None;
    for v in values {
        if let Ok(s) = v.to_str() {
            if let Some(parsed) = parse(s) {
                merge_into(&mut acc, parsed);
            }
        }
    }

    if let Some(merged) = acc {
        let serialized = to_header_value(&merged);
        // safety: to_header_value only produces ASCII
        let value = HeaderValue::from_str(&serialized).expect("to_header_value produced non-ASCII");
        aggregator.entries.insert(
            http::header::CACHE_CONTROL,
            (HeaderAggregationStrategy::Last, vec![value]),
        );
    } else {
        // no valid values found, but there were cache-control headers
        // do the safe thing and graceful thing - completely omit the header
        warn!("no valid cache-control values found, removing header");
        aggregator.entries.remove(&http::header::CACHE_CONTROL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merge(a: Option<ParsedCacheControl>, b: ParsedCacheControl) -> ParsedCacheControl {
        let mut acc = a;
        merge_into(&mut acc, b);
        acc.unwrap()
    }

    // acc is None: first value is adopted as-is
    #[test]
    fn first_value_adopted() {
        let result = merge(
            None,
            ParsedCacheControl {
                is_public: true,
                max_age: Some(300),
                ..Default::default()
            },
        );
        assert!(result.is_public);
        assert_eq!(result.max_age, Some(300));
        assert!(!result.no_store);
        assert!(!result.no_cache);
    }

    // incoming no_store poisons the result
    #[test]
    fn incoming_no_store_poisons() {
        let result = merge(
            Some(ParsedCacheControl {
                is_public: true,
                max_age: Some(300),
                ..Default::default()
            }),
            ParsedCacheControl {
                no_store: true,
                ..Default::default()
            },
        );
        assert!(result.no_store);
        assert!(result.no_cache);
        assert!(!result.is_public);
        assert_eq!(result.max_age, None);
    }

    // incoming no_cache poisons the result
    #[test]
    fn incoming_no_cache_poisons() {
        let result = merge(
            Some(ParsedCacheControl {
                is_public: true,
                max_age: Some(60),
                ..Default::default()
            }),
            ParsedCacheControl {
                no_cache: true,
                ..Default::default()
            },
        );
        assert!(result.no_store);
        assert!(result.no_cache);
        assert!(!result.is_public);
    }

    // incoming private poisons the result
    #[test]
    fn incoming_private_poisons() {
        let result = merge(
            Some(ParsedCacheControl {
                is_public: true,
                max_age: Some(120),
                ..Default::default()
            }),
            ParsedCacheControl {
                is_private: true,
                ..Default::default()
            },
        );
        assert!(result.no_store);
        assert!(result.no_cache);
        assert!(!result.is_public);
        assert!(!result.is_private); // private is cleared, only no-store/no-cache remain
    }

    // existing no_store poisons even with a clean incoming
    #[test]
    fn existing_no_store_poisons() {
        let result = merge(
            Some(ParsedCacheControl {
                no_store: true,
                ..Default::default()
            }),
            ParsedCacheControl {
                is_public: true,
                max_age: Some(300),
                ..Default::default()
            },
        );
        assert!(result.no_store);
        assert!(result.no_cache);
        assert!(!result.is_public);
    }

    // existing private poisons even with a clean incoming
    #[test]
    fn existing_private_poisons() {
        let result = merge(
            Some(ParsedCacheControl {
                is_private: true,
                ..Default::default()
            }),
            ParsedCacheControl {
                is_public: true,
                max_age: Some(300),
                ..Default::default()
            },
        );
        assert!(result.no_store);
        assert!(result.no_cache);
    }

    // both no_store: result is no_store, no_cache
    #[test]
    fn both_no_store() {
        let result = merge(
            Some(ParsedCacheControl {
                no_store: true,
                ..Default::default()
            }),
            ParsedCacheControl {
                no_store: true,
                ..Default::default()
            },
        );
        assert!(result.no_store);
        assert!(result.no_cache);
    }

    // max_age: both present, take the min
    #[test]
    fn max_age_takes_min() {
        let result = merge(
            Some(ParsedCacheControl {
                max_age: Some(500),
                ..Default::default()
            }),
            ParsedCacheControl {
                max_age: Some(300),
                ..Default::default()
            },
        );
        assert_eq!(result.max_age, Some(300));
    }

    // max_age: both present, other direction
    #[test]
    fn max_age_takes_min_other_direction() {
        let result = merge(
            Some(ParsedCacheControl {
                max_age: Some(100),
                ..Default::default()
            }),
            ParsedCacheControl {
                max_age: Some(999),
                ..Default::default()
            },
        );
        assert_eq!(result.max_age, Some(100));
    }

    // max_age: existing has it, incoming does not - keep existing
    #[test]
    fn max_age_existing_only() {
        let result = merge(
            Some(ParsedCacheControl {
                max_age: Some(200),
                ..Default::default()
            }),
            ParsedCacheControl {
                max_age: None,
                ..Default::default()
            },
        );
        assert_eq!(result.max_age, Some(200));
    }

    // max_age: incoming has it, existing does not - adopt incoming
    #[test]
    fn max_age_incoming_only() {
        let result = merge(
            Some(ParsedCacheControl {
                max_age: None,
                ..Default::default()
            }),
            ParsedCacheControl {
                max_age: Some(60),
                ..Default::default()
            },
        );
        assert_eq!(result.max_age, Some(60));
    }

    // max_age: neither has it
    #[test]
    fn max_age_neither() {
        let result = merge(
            Some(ParsedCacheControl::default()),
            ParsedCacheControl::default(),
        );
        assert_eq!(result.max_age, None);
    }

    // public: both public -> stays public
    #[test]
    fn public_both_public() {
        let result = merge(
            Some(ParsedCacheControl {
                is_public: true,
                ..Default::default()
            }),
            ParsedCacheControl {
                is_public: true,
                ..Default::default()
            },
        );
        assert!(result.is_public);
    }

    // public: existing public, incoming not -> stripped
    #[test]
    fn public_stripped_when_incoming_not_public() {
        let result = merge(
            Some(ParsedCacheControl {
                is_public: true,
                ..Default::default()
            }),
            ParsedCacheControl {
                is_public: false,
                ..Default::default()
            },
        );
        assert!(!result.is_public);
    }

    // public: neither public -> stays false
    #[test]
    fn public_neither() {
        let result = merge(
            Some(ParsedCacheControl::default()),
            ParsedCacheControl::default(),
        );
        assert!(!result.is_public);
    }

    // must_revalidate: either side sets it -> propagated
    #[test]
    fn must_revalidate_from_incoming() {
        let result = merge(
            Some(ParsedCacheControl {
                must_revalidate: false,
                ..Default::default()
            }),
            ParsedCacheControl {
                must_revalidate: true,
                ..Default::default()
            },
        );
        assert!(result.must_revalidate);
    }

    #[test]
    fn must_revalidate_from_existing() {
        let result = merge(
            Some(ParsedCacheControl {
                must_revalidate: true,
                ..Default::default()
            }),
            ParsedCacheControl {
                must_revalidate: false,
                ..Default::default()
            },
        );
        assert!(result.must_revalidate);
    }

    // must_revalidate: neither sets it
    #[test]
    fn must_revalidate_neither() {
        let result = merge(
            Some(ParsedCacheControl::default()),
            ParsedCacheControl::default(),
        );
        assert!(!result.must_revalidate);
    }

    // must_revalidate is cleared when poison is triggered
    #[test]
    fn must_revalidate_cleared_on_poison() {
        let result = merge(
            Some(ParsedCacheControl {
                must_revalidate: true,
                ..Default::default()
            }),
            ParsedCacheControl {
                no_store: true,
                ..Default::default()
            },
        );
        assert!(result.no_store);
        assert!(result.no_cache);
        assert!(!result.must_revalidate);
    }

    // three-way merge: public survives only if all three agree
    #[test]
    fn three_way_all_public() {
        let mut acc = None;
        merge_into(
            &mut acc,
            ParsedCacheControl {
                is_public: true,
                max_age: Some(300),
                ..Default::default()
            },
        );
        merge_into(
            &mut acc,
            ParsedCacheControl {
                is_public: true,
                max_age: Some(200),
                ..Default::default()
            },
        );
        merge_into(
            &mut acc,
            ParsedCacheControl {
                is_public: true,
                max_age: Some(500),
                ..Default::default()
            },
        );
        let result = acc.unwrap();
        assert!(result.is_public);
        assert_eq!(result.max_age, Some(200));
    }

    // three-way merge: one non-public kills public
    #[test]
    fn three_way_one_not_public() {
        let mut acc = None;
        merge_into(
            &mut acc,
            ParsedCacheControl {
                is_public: true,
                max_age: Some(300),
                ..Default::default()
            },
        );
        merge_into(
            &mut acc,
            ParsedCacheControl {
                is_public: false,
                max_age: Some(100),
                ..Default::default()
            },
        );
        merge_into(
            &mut acc,
            ParsedCacheControl {
                is_public: true,
                max_age: Some(200),
                ..Default::default()
            },
        );
        let result = acc.unwrap();
        assert!(!result.is_public);
        assert_eq!(result.max_age, Some(100));
    }

    // three-way merge: third is poisonous, earlier max-age/public are discarded
    #[test]
    fn three_way_third_poisons() {
        let mut acc = None;
        merge_into(
            &mut acc,
            ParsedCacheControl {
                is_public: true,
                max_age: Some(300),
                ..Default::default()
            },
        );
        merge_into(
            &mut acc,
            ParsedCacheControl {
                is_public: true,
                max_age: Some(200),
                ..Default::default()
            },
        );
        merge_into(
            &mut acc,
            ParsedCacheControl {
                no_store: true,
                ..Default::default()
            },
        );
        let result = acc.unwrap();
        assert!(result.no_store);
        assert!(result.no_cache);
        assert!(!result.is_public);
        assert_eq!(result.max_age, None);
    }

    // three-way merge: first is poisonous, subsequent clean values don't un-poison
    #[test]
    fn three_way_first_poisons_no_recovery() {
        let mut acc = None;
        merge_into(
            &mut acc,
            ParsedCacheControl {
                no_cache: true,
                ..Default::default()
            },
        );
        merge_into(
            &mut acc,
            ParsedCacheControl {
                is_public: true,
                max_age: Some(300),
                ..Default::default()
            },
        );
        merge_into(
            &mut acc,
            ParsedCacheControl {
                is_public: true,
                max_age: Some(200),
                ..Default::default()
            },
        );
        let result = acc.unwrap();
        assert!(result.no_store);
        assert!(result.no_cache);
        assert!(!result.is_public);
    }

    fn make_aggregator(values: &[&str]) -> ResponseHeaderAggregator {
        let mut agg = ResponseHeaderAggregator::default();
        for v in values {
            agg.write(
                &http::header::CACHE_CONTROL,
                &http::HeaderValue::from_str(v).unwrap(),
                HeaderAggregationStrategy::Append,
            );
        }
        agg
    }

    fn cc_value(agg: &ResponseHeaderAggregator) -> Option<String> {
        agg.entries
            .get(&http::header::CACHE_CONTROL)
            .and_then(|(_, vs)| vs.first())
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    }

    #[test]
    fn finalize_force_no_store_forces_no_store() {
        let mut agg = make_aggregator(&["public, max-age=300"]);
        finalize(&mut agg, true);
        assert_eq!(
            cc_value(&agg).as_deref(),
            Some("no-store, no-cache, must-revalidate")
        );
    }

    #[test]
    fn finalize_merges_two_appended_values() {
        let mut agg = make_aggregator(&["public, max-age=300", "public, max-age=60"]);
        finalize(&mut agg, false);
        assert_eq!(cc_value(&agg).as_deref(), Some("public, max-age=60"));
    }

    #[test]
    fn finalize_private_collapses_to_no_store() {
        let mut agg = make_aggregator(&["private"]);
        finalize(&mut agg, false);
        assert_eq!(cc_value(&agg).as_deref(), Some("no-store, no-cache"));
    }

    #[test]
    fn finalize_absent_entry_no_error_leaves_absent() {
        let mut agg = ResponseHeaderAggregator::default();
        finalize(&mut agg, false);
        assert!(agg.entries.get(&http::header::CACHE_CONTROL).is_none());
    }

    // empty string: parse() returns None, acc stays None, entry left unchanged
    #[test]
    fn finalize_empty_string_removes_header() {
        let mut agg = make_aggregator(&[""]);
        finalize(&mut agg, false);
        assert_eq!(cc_value(&agg).as_deref(), None);
    }

    // non-UTF-8: to_str() fails, acc stays None, entry left unchanged
    #[test]
    fn finalize_invalid_utf8_removes_header() {
        let mut agg = ResponseHeaderAggregator::default();
        let invalid = http::HeaderValue::from_bytes(&[0xFF, 0xFE]).unwrap();
        agg.write(
            &http::header::CACHE_CONTROL,
            &invalid,
            HeaderAggregationStrategy::Append,
        );
        finalize(&mut agg, false);
        assert_eq!(cc_value(&agg).as_deref(), None);
    }

    // unrecognized directive: parse() returns Some(default), serializes to no-store
    #[test]
    fn finalize_unrecognized_directive_removes_header() {
        let mut agg = make_aggregator(&["bogus-directive"]);
        finalize(&mut agg, false);
        assert_eq!(cc_value(&agg).as_deref(), None);
    }

    #[test]
    fn finalize_absent_entry_with_force_no_store_absent() {
        let mut agg = ResponseHeaderAggregator::default();
        finalize(&mut agg, true);
        assert!(agg.entries.get(&http::header::CACHE_CONTROL).is_none());
    }
}
