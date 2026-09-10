use crate::executor::headers::{
    plan::HeaderAggregationStrategy, response::ResponseHeaderAggregator,
};
use crate::telemetry::logging::targets;
use http::HeaderValue;
use tracing::{debug, warn};

lazy_static::lazy_static! {
    static ref NO_STORE_HEADER_VALUE: HeaderValue =
        HeaderValue::from_static("no-store, no-cache, must-revalidate");
}

#[derive(Clone, Default)]
struct CacheControl {
    no_store: bool,
    no_cache: bool,
    must_revalidate: bool,
    proxy_revalidate: bool,
    must_understand: bool,
    no_transform: bool,
    immutable: bool,
    is_private: bool,
    is_public: bool,
    max_age: Option<u32>,
    s_maxage: Option<u32>,
    stale_while_revalidate: Option<u32>,
    stale_if_error: Option<u32>,
}

fn poison() -> CacheControl {
    CacheControl {
        no_store: true,
        no_cache: true,
        ..Default::default()
    }
}

// directives like max-age require a numeric value; anything else
// (missing or non-numeric) makes the whole header untrustworthy
fn parse_u32(token: &str, value: Option<&str>) -> Result<u32, ()> {
    match value {
        Some(v) => v.parse::<u32>().map_err(|_| {
            warn!(
                target: targets::CACHE_CONTROL,
                directive = token,
                value = v,
                "cache-control directive has non-numeric value"
            );
        }),
        None => {
            warn!(
                target: targets::CACHE_CONTROL,
                directive = token,
                "cache-control directive is missing a value"
            );
            Err(())
        }
    }
}

fn parse(header: &str) -> Option<CacheControl> {
    let trimmed = header.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut p = CacheControl::default();

    for part in trimmed.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (token, value) = match part.split_once('=') {
            Some((t, v)) => (t.trim(), Some(v.trim())),
            None => (part, None),
        };
        let token_lower = token.to_ascii_lowercase();
        match token_lower.as_str() {
            "no-store" => p.no_store = true,
            // note: no-cache may carry a field list (no-cache="set-cookie"); we treat the
            // qualified form as the unqualified, more restrictive one. same for private below.
            // a quoted list containing a comma splits into garbage parts which then hit the
            // unknown-directive arm and poison - still safe, just noisier than necessary.
            "no-cache" => p.no_cache = true,
            "private" => p.is_private = true,
            "public" => p.is_public = true,
            "must-revalidate" => p.must_revalidate = true,
            "proxy-revalidate" => p.proxy_revalidate = true,
            "must-understand" => p.must_understand = true,
            "no-transform" => p.no_transform = true,
            "immutable" => p.immutable = true,
            "max-age" | "s-maxage" | "stale-while-revalidate" | "stale-if-error" => {
                let Ok(n) = parse_u32(&token_lower, value) else {
                    return Some(poison());
                };
                match token_lower.as_str() {
                    "max-age" => p.max_age = Some(n),
                    "s-maxage" => p.s_maxage = Some(n),
                    "stale-while-revalidate" => p.stale_while_revalidate = Some(n),
                    _ => p.stale_if_error = Some(n),
                }
            }
            v => {
                // a directive we don't model may be restrictive, so dropping the
                // value would be unsecure - poison the merge instead
                warn!(target: targets::CACHE_CONTROL, directive = v, "cache-control has unrecognized directive");
                return Some(poison());
            }
        }
    }

    Some(p)
}

fn merge_into(acc: &mut Option<CacheControl>, incoming: CacheControl) {
    let Some(existing) = acc else {
        *acc = Some(incoming);
        return;
    };

    if existing.no_store || existing.no_cache || incoming.no_store || incoming.no_cache {
        *existing = CacheControl {
            no_store: true,
            no_cache: true,
            ..Default::default()
        };
        return;
    }

    // grants (public, immutable) hold only if every side grants them: AND.
    // restrictions (must-revalidate & friends, no-transform) hold if any side asks: OR.
    // durations (max-age & friends) take the shortest present value: min.
    existing.is_private = existing.is_private || incoming.is_private;
    existing.is_public = existing.is_public && incoming.is_public && !existing.is_private;
    existing.immutable = existing.immutable && incoming.immutable;
    existing.must_revalidate = existing.must_revalidate || incoming.must_revalidate;
    existing.proxy_revalidate = existing.proxy_revalidate || incoming.proxy_revalidate;
    existing.must_understand = existing.must_understand || incoming.must_understand;
    existing.no_transform = existing.no_transform || incoming.no_transform;

    let shared_max_age = min_opt(
        existing.s_maxage.or(existing.max_age),
        incoming.s_maxage.or(incoming.max_age),
    );
    let all_have_s_maxage = existing.s_maxage.is_some() && incoming.s_maxage.is_some();
    existing.max_age = min_opt(existing.max_age, incoming.max_age);
    existing.s_maxage = all_have_s_maxage.then_some(shared_max_age).flatten();
    if !all_have_s_maxage {
        existing.max_age = min_opt(existing.max_age, shared_max_age);
    }
    existing.stale_while_revalidate = min_opt(
        existing.stale_while_revalidate,
        incoming.stale_while_revalidate,
    );
    existing.stale_if_error = min_opt(existing.stale_if_error, incoming.stale_if_error);
}

// a side without the duration doesn't pull the min down, it just abstains
fn min_opt(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, None) => a,
        (None, b) => b,
    }
}

fn to_header_value(p: &CacheControl) -> String {
    if p.no_store || p.no_cache {
        return "no-store, no-cache".to_string();
    }

    let mut parts: Vec<String> = Vec::new();

    if p.is_private {
        parts.push("private".to_string());
    } else if p.is_public {
        parts.push("public".to_string());
    }

    if let Some(age) = p.max_age {
        parts.push(format!("max-age={age}"));
    }

    if let Some(age) = p.s_maxage {
        parts.push(format!("s-maxage={age}"));
    }

    if let Some(age) = p.stale_while_revalidate {
        parts.push(format!("stale-while-revalidate={age}"));
    }

    if let Some(age) = p.stale_if_error {
        parts.push(format!("stale-if-error={age}"));
    }

    if p.must_revalidate {
        parts.push("must-revalidate".to_string());
    }

    if p.proxy_revalidate {
        parts.push("proxy-revalidate".to_string());
    }

    if p.must_understand {
        parts.push("must-understand".to_string());
    }

    if p.no_transform {
        parts.push("no-transform".to_string());
    }

    if p.immutable {
        parts.push("immutable".to_string());
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
/// 1. Poison check - if any value contains `no-store` or `no-cache`, the accumulated
///    result is immediately locked to `no-store, no-cache` and all remaining directives
///    are discarded. Further incoming values cannot "un-poison" this state.
/// 2. `private` - preserved if any subgraph sets it and overrides `public`. Other
///    directives, including freshness lifetimes, are retained.
/// 3. Durations (`max-age`, `s-maxage`, `stale-while-revalidate`, `stale-if-error`) -
///    the minimum of all present values is kept. Because shared caches prefer `s-maxage`
///    over `max-age`, mixed inputs are capped by their minimum effective shared lifetime.
/// 4. Grants (`public`, `immutable`) - preserved only when every subgraph that
///    returned a response also sent them. A subgraph that returned bytes but omitted
///    `Cache-Control` entirely is counted through `total_responses` (the number of
///    all subgraphs whose response counts) and is enough to strip the grant from the
///    result, because silence is not consent.
/// 5. Restrictions (`must-revalidate`, `proxy-revalidate`, `must-understand`,
///    `no-transform`) - set if any subgraph sets them (logical OR). Cleared on poison.
/// 6. A value with a malformed directive, or a directive the router does not model,
///    is treated like poison (rule 1), since it may have been restrictive.
///
/// The merged result is serialised back to a `HeaderValue` and re-inserted into the
/// aggregator under `Last` strategy so that any subsequent header-flush loop sees
/// exactly one value.
///
/// If every collected value was unparseable (e.g. non-UTF-8 bytes) or empty the fold
/// produces no `acc` and the `Cache-Control` header is removed from the aggregator.
/// This avoids forwarding potentially unsafe or malformed caching directives.
pub fn finalize(
    aggregator: &mut ResponseHeaderAggregator,
    force_no_store: bool,
    total_responses: usize,
) {
    let Some((_, values)) = aggregator.entries.get(&http::header::CACHE_CONTROL) else {
        // there's no cache-control headers anywhere, so nothing to merge or poison - just leave it absent
        return;
    };

    if force_no_store {
        let value = NO_STORE_HEADER_VALUE.clone();
        aggregator.entries.insert(
            http::header::CACHE_CONTROL,
            (HeaderAggregationStrategy::Last, vec![value]),
        );
        return;
    }

    let mut acc: Option<CacheControl> = None;
    for v in values {
        if let Ok(s) = v.to_str() {
            if let Some(parsed) = parse(s) {
                merge_into(&mut acc, parsed);
            }
        }
    }

    if let Some(mut merged) = acc {
        // a silent subgraph (no cache-control header at all) did not assert public
        // or immutable, so these grants cannot hold when not every contacted subgraph
        // sent them
        if total_responses > values.len() {
            merged.is_public = false;
            merged.immutable = false;
        }
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
        for v in values {
            debug!(target: targets::CACHE_CONTROL, value = ?v, "invalid cache-control value");
        }

        warn!(target: targets::CACHE_CONTROL, "no valid cache-control values found, removing header");

        aggregator.entries.remove(&http::header::CACHE_CONTROL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merge(a: Option<CacheControl>, b: CacheControl) -> CacheControl {
        let mut acc = a;
        merge_into(&mut acc, b);
        acc.unwrap()
    }

    // acc is None: first value is adopted as-is
    #[test]
    fn first_value_adopted() {
        let result = merge(
            None,
            CacheControl {
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
            Some(CacheControl {
                is_public: true,
                max_age: Some(300),
                ..Default::default()
            }),
            CacheControl {
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
            Some(CacheControl {
                is_public: true,
                max_age: Some(60),
                ..Default::default()
            }),
            CacheControl {
                no_cache: true,
                ..Default::default()
            },
        );
        assert!(result.no_store);
        assert!(result.no_cache);
        assert!(!result.is_public);
    }

    // incoming private overrides public without preventing private caches from storing
    #[test]
    fn incoming_private_is_preserved() {
        let result = merge(
            Some(CacheControl {
                is_public: true,
                max_age: Some(120),
                ..Default::default()
            }),
            CacheControl {
                is_private: true,
                max_age: Some(50),
                ..Default::default()
            },
        );
        assert!(!result.no_store);
        assert!(!result.no_cache);
        assert!(!result.is_public);
        assert!(result.is_private);
        assert_eq!(result.max_age, Some(50));
    }

    // existing no_store poisons even with a clean incoming
    #[test]
    fn existing_no_store_poisons() {
        let result = merge(
            Some(CacheControl {
                no_store: true,
                ..Default::default()
            }),
            CacheControl {
                is_public: true,
                max_age: Some(300),
                ..Default::default()
            },
        );
        assert!(result.no_store);
        assert!(result.no_cache);
        assert!(!result.is_public);
    }

    // existing private remains private when merged with a public response
    #[test]
    fn existing_private_is_preserved() {
        let result = merge(
            Some(CacheControl {
                is_private: true,
                ..Default::default()
            }),
            CacheControl {
                is_public: true,
                max_age: Some(300),
                ..Default::default()
            },
        );
        assert!(!result.no_store);
        assert!(!result.no_cache);
        assert!(result.is_private);
        assert!(!result.is_public);
    }

    // both no_store: result is no_store, no_cache
    #[test]
    fn both_no_store() {
        let result = merge(
            Some(CacheControl {
                no_store: true,
                ..Default::default()
            }),
            CacheControl {
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
            Some(CacheControl {
                max_age: Some(500),
                ..Default::default()
            }),
            CacheControl {
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
            Some(CacheControl {
                max_age: Some(100),
                ..Default::default()
            }),
            CacheControl {
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
            Some(CacheControl {
                max_age: Some(200),
                ..Default::default()
            }),
            CacheControl {
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
            Some(CacheControl {
                max_age: None,
                ..Default::default()
            }),
            CacheControl {
                max_age: Some(60),
                ..Default::default()
            },
        );
        assert_eq!(result.max_age, Some(60));
    }

    // max_age: neither has it
    #[test]
    fn max_age_neither() {
        let result = merge(Some(CacheControl::default()), CacheControl::default());
        assert_eq!(result.max_age, None);
    }

    // public: both public -> stays public
    #[test]
    fn public_both_public() {
        let result = merge(
            Some(CacheControl {
                is_public: true,
                ..Default::default()
            }),
            CacheControl {
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
            Some(CacheControl {
                is_public: true,
                ..Default::default()
            }),
            CacheControl {
                is_public: false,
                ..Default::default()
            },
        );
        assert!(!result.is_public);
    }

    // public: neither public -> stays false
    #[test]
    fn public_neither() {
        let result = merge(Some(CacheControl::default()), CacheControl::default());
        assert!(!result.is_public);
    }

    // must_revalidate: either side sets it -> propagated
    #[test]
    fn must_revalidate_from_incoming() {
        let result = merge(
            Some(CacheControl {
                must_revalidate: false,
                ..Default::default()
            }),
            CacheControl {
                must_revalidate: true,
                ..Default::default()
            },
        );
        assert!(result.must_revalidate);
    }

    #[test]
    fn must_revalidate_from_existing() {
        let result = merge(
            Some(CacheControl {
                must_revalidate: true,
                ..Default::default()
            }),
            CacheControl {
                must_revalidate: false,
                ..Default::default()
            },
        );
        assert!(result.must_revalidate);
    }

    // must_revalidate: neither sets it
    #[test]
    fn must_revalidate_neither() {
        let result = merge(Some(CacheControl::default()), CacheControl::default());
        assert!(!result.must_revalidate);
    }

    // must_revalidate is cleared when poison is triggered
    #[test]
    fn must_revalidate_cleared_on_poison() {
        let result = merge(
            Some(CacheControl {
                must_revalidate: true,
                ..Default::default()
            }),
            CacheControl {
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
            CacheControl {
                is_public: true,
                max_age: Some(300),
                ..Default::default()
            },
        );
        merge_into(
            &mut acc,
            CacheControl {
                is_public: true,
                max_age: Some(200),
                ..Default::default()
            },
        );
        merge_into(
            &mut acc,
            CacheControl {
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
            CacheControl {
                is_public: true,
                max_age: Some(300),
                ..Default::default()
            },
        );
        merge_into(
            &mut acc,
            CacheControl {
                is_public: false,
                max_age: Some(100),
                ..Default::default()
            },
        );
        merge_into(
            &mut acc,
            CacheControl {
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
            CacheControl {
                is_public: true,
                max_age: Some(300),
                ..Default::default()
            },
        );
        merge_into(
            &mut acc,
            CacheControl {
                is_public: true,
                max_age: Some(200),
                ..Default::default()
            },
        );
        merge_into(
            &mut acc,
            CacheControl {
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
            CacheControl {
                no_cache: true,
                ..Default::default()
            },
        );
        merge_into(
            &mut acc,
            CacheControl {
                is_public: true,
                max_age: Some(300),
                ..Default::default()
            },
        );
        merge_into(
            &mut acc,
            CacheControl {
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
        finalize(&mut agg, true, 1);
        assert_eq!(
            cc_value(&agg).as_deref(),
            Some("no-store, no-cache, must-revalidate")
        );
    }

    #[test]
    fn finalize_merges_two_appended_values() {
        let mut agg = make_aggregator(&["public, max-age=300", "public, max-age=60"]);
        finalize(&mut agg, false, 2);
        assert_eq!(cc_value(&agg).as_deref(), Some("public, max-age=60"));
    }

    #[test]
    fn finalize_private_is_preserved() {
        let mut agg = make_aggregator(&["private"]);
        finalize(&mut agg, false, 1);
        assert_eq!(cc_value(&agg).as_deref(), Some("private"));
    }

    #[test]
    fn finalize_user_requirement_private_overrides_public_and_keeps_min_age() {
        let mut agg = make_aggregator(&["public, max-age=100", "private, max-age=50"]);
        finalize(&mut agg, false, 2);
        assert_eq!(cc_value(&agg).as_deref(), Some("private, max-age=50"));
    }

    #[test]
    fn finalize_absent_entry_no_error_leaves_absent() {
        let mut agg = ResponseHeaderAggregator::default();
        finalize(&mut agg, false, 0);
        assert!(agg.entries.get(&http::header::CACHE_CONTROL).is_none());
    }

    // empty string: parse() returns None, acc stays None, entry left unchanged
    #[test]
    fn finalize_empty_string_removes_header() {
        let mut agg = make_aggregator(&[""]);
        finalize(&mut agg, false, 1);
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
        finalize(&mut agg, false, 1);
        assert_eq!(cc_value(&agg).as_deref(), None);
    }

    #[test]
    fn finalize_unrecognized_value_poisons() {
        let mut agg = make_aggregator(&["bogus-directive"]);
        finalize(&mut agg, false, 1);
        assert_eq!(cc_value(&agg).as_deref(), Some("no-store, no-cache"));
    }

    #[test]
    fn finalize_single_unrecognized_directive_poisons() {
        let mut agg = make_aggregator(&["public, max-age=300, huh"]);
        finalize(&mut agg, false, 1);
        assert_eq!(cc_value(&agg).as_deref(), Some("no-store, no-cache"));
    }

    #[test]
    fn finalize_malformed_max_age_poisons() {
        let mut agg = make_aggregator(&["public, max-age=woof"]);
        finalize(&mut agg, false, 1);
        assert_eq!(cc_value(&agg).as_deref(), Some("no-store, no-cache"));
    }

    // the exact #1172 scenario: no-store next to s-maxage, sibling says public -
    // result must not be cacheable
    #[test]
    fn finalize_no_store_with_s_maxage_not_dropped() {
        let mut agg = make_aggregator(&["no-store, s-maxage=0", "public, max-age=300"]);
        finalize(&mut agg, false, 2);
        assert_eq!(cc_value(&agg).as_deref(), Some("no-store, no-cache"));
    }

    // standard directives are modeled and no longer poison a clean merge
    #[test]
    fn finalize_standard_directives_pass_through() {
        let mut agg = make_aggregator(&[
            "public, max-age=300, s-maxage=600, stale-while-revalidate=30, stale-if-error=60, no-transform, immutable",
        ]);
        finalize(&mut agg, false, 1);
        assert_eq!(
            cc_value(&agg).as_deref(),
            Some("public, max-age=300, s-maxage=600, stale-while-revalidate=30, stale-if-error=60, no-transform, immutable")
        );
    }

    #[test]
    fn finalize_user_requirement_s_maxage_takes_min() {
        let mut agg = make_aggregator(&["public, s-maxage=100", "public, s-maxage=20"]);
        finalize(&mut agg, false, 2);
        assert_eq!(cc_value(&agg).as_deref(), Some("public, s-maxage=20"));
    }

    // durations take the min across subgraphs, independently per directive
    #[test]
    fn finalize_durations_take_min() {
        let mut agg = make_aggregator(&[
            "s-maxage=600, stale-while-revalidate=10, stale-if-error=120",
            "s-maxage=60, stale-while-revalidate=30, stale-if-error=15",
        ]);
        finalize(&mut agg, false, 2);
        assert_eq!(
            cc_value(&agg).as_deref(),
            Some("s-maxage=60, stale-while-revalidate=10, stale-if-error=15")
        );
    }

    // shared caches prefer s-maxage, so mixed inputs use the lower effective lifetime
    #[test]
    fn finalize_mixed_max_age_and_s_maxage_take_shared_min() {
        let mut agg = make_aggregator(&["s-maxage=600", "max-age=100"]);
        finalize(&mut agg, false, 2);
        assert_eq!(cc_value(&agg).as_deref(), Some("max-age=100"));
    }

    // restrictions are OR: one subgraph asking is enough
    #[test]
    fn finalize_restrictions_are_or() {
        let mut agg = make_aggregator(&[
            "max-age=100, no-transform",
            "max-age=200, proxy-revalidate, must-understand",
        ]);
        finalize(&mut agg, false, 2);
        assert_eq!(
            cc_value(&agg).as_deref(),
            Some("max-age=100, proxy-revalidate, must-understand, no-transform")
        );
    }

    // immutable is a grant: dropped unless every subgraph sends it
    #[test]
    fn finalize_immutable_requires_all() {
        let mut agg = make_aggregator(&["max-age=100, immutable", "max-age=100"]);
        finalize(&mut agg, false, 2);
        assert_eq!(cc_value(&agg).as_deref(), Some("max-age=100"));

        let mut agg = make_aggregator(&["max-age=100, immutable", "max-age=100, immutable"]);
        finalize(&mut agg, false, 2);
        assert_eq!(cc_value(&agg).as_deref(), Some("max-age=100, immutable"));
    }

    // a silent subgraph strips immutable, same as public
    #[test]
    fn finalize_immutable_stripped_when_silent_subgraph() {
        let mut agg = make_aggregator(&["public, max-age=100, immutable"]);
        finalize(&mut agg, false, 2);
        assert_eq!(cc_value(&agg).as_deref(), Some("max-age=100"));
    }

    // poison clears the new fields too
    #[test]
    fn finalize_poison_clears_new_fields() {
        let mut agg = make_aggregator(&[
            "s-maxage=600, stale-while-revalidate=30, no-transform, immutable",
            "no-store",
        ]);
        finalize(&mut agg, false, 2);
        assert_eq!(cc_value(&agg).as_deref(), Some("no-store, no-cache"));
    }

    // malformed duration values on the new directives still poison
    #[test]
    fn finalize_malformed_s_maxage_poisons() {
        let mut agg = make_aggregator(&["public, s-maxage=woof"]);
        finalize(&mut agg, false, 1);
        assert_eq!(cc_value(&agg).as_deref(), Some("no-store, no-cache"));
    }

    // qualified no-cache/private forms are treated as the unqualified restrictive form
    #[test]
    fn finalize_qualified_no_cache_poisons() {
        let mut agg = make_aggregator(&["no-cache=\"set-cookie\", max-age=300"]);
        finalize(&mut agg, false, 1);
        assert_eq!(cc_value(&agg).as_deref(), Some("no-store, no-cache"));
    }

    #[test]
    fn finalize_absent_entry_with_force_no_store_absent() {
        let mut agg = ResponseHeaderAggregator::default();
        finalize(&mut agg, true, 0);
        assert!(agg.entries.get(&http::header::CACHE_CONTROL).is_none());
    }

    #[test]
    fn finalize_public_stripped_when_silent_subgraph() {
        // one subgraph sent public, one sent nothing - public must not survive
        let mut agg = make_aggregator(&["public, max-age=200"]);
        finalize(&mut agg, false, 2);
        let cc = cc_value(&agg).unwrap_or_default();
        assert!(!cc.contains("public"), "expected no public, got: {cc}");
        assert!(
            cc.contains("max-age=200"),
            "expected max-age=200, got: {cc}"
        );
    }
}
