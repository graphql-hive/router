use moka::sync::Cache;
use rand::prelude::*;
use vrl::compiler::Program as VrlProgram;

use crate::agent::config::{
    AtLeastOnceKey as AtLeastOnceKeyConfig, AtLeastOnceKeyConstant,
    SamplerConfig as SamplerConfigType,
};
use crate::agent::usage_agent::{
    get_vrl_value_from_execution_report_and_request, ExecutionReport, RequestDetails,
};
use crate::expressions::lib::FromVrlValue;
use crate::expressions::values::string::StringConversionError;
use crate::expressions::CompileExpression;
use crate::expressions::ExecutableProgram;
use crate::expressions::{ExpressionCompileError, ExpressionExecutionError};

/// Errors that can be produced while evaluating a [`Sampler`] decision.
///
/// These mirror the existing `ExcludeExpression*` error family on the agent.
#[derive(Debug, thiserror::Error)]
pub enum SamplerError {
    #[error("failed to compile sampler key expression: {0}")]
    KeyExpressionCompile(#[from] ExpressionCompileError),
    #[error("failed to execute sampler key expression: {0}")]
    KeyExpressionExecute(#[from] ExpressionExecutionError),
    #[error("failed to convert sampler key expression result to string: {0}")]
    KeyExpressionConversion(#[from] StringConversionError),
}

/// How `at_least_once` sampling derives the key that identifies "the same
/// operation" for the purpose of guaranteeing one report.
pub enum AtLeastOnceKey {
    /// Use the GraphQL operation name (or `""` for anonymous operations).
    OperationName,
    /// Evaluate a VRL expression that returns the key as a string. The
    /// compiled program is boxed because [`VrlProgram`] is several hundred
    /// bytes and would otherwise dominate the size of every [`Sampler`].
    Expression { program: Box<VrlProgram> },
}

/// Runtime form of `telemetry.hive.usage_reporting.sampler`.
///
/// The agent calls [`Sampler::should_sample`] for every report after the
/// optional [`crate::agent::exclude::Exclude`] filter has run, so reports
/// that are excluded by configuration never enter the at-least-once
/// seen-set.
pub enum Sampler {
    /// Probabilistic sampling at a fixed rate in `[0.0, 1.0]`.
    Fixed { rate: f64 },
    /// Always samples the first occurrence per resolved key, then samples
    /// subsequent occurrences with `rate`. The `seen` cache is bounded by
    /// `max_seen_keys` (LRU eviction); evicted keys are treated as "first
    /// occurrence" again.
    AtLeastOnce {
        key: AtLeastOnceKey,
        rate: f64,
        seen: Cache<String, ()>,
    },
}

impl Default for Sampler {
    fn default() -> Self {
        Self::Fixed { rate: 1.0 }
    }
}

impl Sampler {
    /// Build a runtime [`Sampler`] from a deserialized [`SamplerConfigType`].
    ///
    /// Any VRL expression on `at_least_once.key` is compiled here so that
    /// configuration errors surface synchronously when the agent is built,
    /// not on the first sampled report.
    pub fn from_config(config: &SamplerConfigType) -> Result<Self, SamplerError> {
        match config {
            SamplerConfigType::Fixed { rate } => Ok(Self::Fixed {
                rate: rate.as_f64(),
            }),
            SamplerConfigType::AtLeastOnce {
                key,
                rate,
                max_seen_keys,
            } => {
                let runtime_key = match key {
                    AtLeastOnceKeyConfig::Constant(AtLeastOnceKeyConstant::OperationName) => {
                        AtLeastOnceKey::OperationName
                    }
                    AtLeastOnceKeyConfig::Expression { expression } => {
                        let program = expression.compile_expression(None)?;
                        AtLeastOnceKey::Expression {
                            program: Box::new(program),
                        }
                    }
                };
                Ok(Self::AtLeastOnce {
                    key: runtime_key,
                    rate: rate.as_f64(),
                    seen: Cache::new(*max_seen_keys),
                })
            }
        }
    }

    /// Decide whether a report should be added to the buffer.
    ///
    /// Returns `Ok(true)` to keep, `Ok(false)` to drop. `Err` is only returned
    /// when a `key.expression` (in `at_least_once`) fails to execute or convert
    /// to a string at runtime. Compilation of the expression happens earlier
    /// (when the agent is built).
    pub fn should_sample(
        &self,
        report: &ExecutionReport,
        request: Option<&RequestDetails>,
    ) -> Result<bool, SamplerError> {
        match self {
            Sampler::Fixed { rate } => Ok(sample_with_rate(*rate)),
            Sampler::AtLeastOnce { key, rate, seen } => {
                // The first occurrence per key is always sampled. `Cache::entry`
                // is atomic: if multiple threads race for the same missing
                // key, exactly one of them sees `is_fresh()`.
                //
                // For the common `OperationName` path we look the key up by
                // `&str` first so subsequent occurrences (the hot path) do
                // not allocate a new `String` per request. Only the very
                // first occurrence per operation name pays for the
                // `to_owned()`.
                let is_first_occurrence = match key {
                    AtLeastOnceKey::OperationName => {
                        let op_name = report.operation_name.as_deref().unwrap_or_default();
                        if seen.contains_key(op_name) {
                            false
                        } else {
                            seen.entry(op_name.to_owned()).or_insert(()).is_fresh()
                        }
                    }
                    AtLeastOnceKey::Expression { program } => {
                        let value = get_vrl_value_from_execution_report_and_request(
                            report,
                            request.cloned(),
                        );
                        let result = program.execute(value)?;
                        let key_str = String::from_vrl_value(result)?;
                        seen.entry(key_str).or_insert(()).is_fresh()
                    }
                };

                if is_first_occurrence {
                    return Ok(true);
                }

                Ok(sample_with_rate(*rate))
            }
        }
    }
}

#[inline]
fn sample_with_rate(rate: f64) -> bool {
    if rate >= 1.0 {
        return true;
    }
    if rate <= 0.0 {
        return false;
    }
    rand::rng().random_bool(rate)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use graphql_tools::parser::parse_schema;
    use moka::sync::Cache;

    use super::{AtLeastOnceKey, Sampler};
    use crate::agent::usage_agent::{ExecutionReport, OperationType};
    use crate::expressions::CompileExpression;

    fn make_report(operation_name: Option<&str>) -> ExecutionReport {
        let schema: graphql_tools::static_graphql::schema::Document =
            parse_schema("type Query { hello: String }").unwrap();
        ExecutionReport {
            schema: Arc::new(schema),
            operation_body: "query { hello }".to_string(),
            operation_name: operation_name.map(|s| s.to_string()),
            operation_type: Some(OperationType::Query),
            client_name: None,
            client_version: None,
            timestamp: 0,
            duration: Duration::from_millis(1),
            ok: true,
            errors: 0,
            persisted_document_hash: None,
        }
    }

    fn at_least_once_op_name(rate: f64) -> Sampler {
        at_least_once_op_name_with_capacity(rate, 1024)
    }

    fn at_least_once_op_name_with_capacity(rate: f64, max_seen_keys: u64) -> Sampler {
        Sampler::AtLeastOnce {
            key: AtLeastOnceKey::OperationName,
            rate,
            seen: Cache::new(max_seen_keys),
        }
    }

    #[test]
    fn fixed_full_rate_always_samples() {
        let sampler = Sampler::Fixed { rate: 1.0 };
        let report = make_report(Some("Q"));
        for _ in 0..10 {
            assert!(sampler.should_sample(&report, None).unwrap());
        }
    }

    #[test]
    fn fixed_zero_rate_never_samples() {
        let sampler = Sampler::Fixed { rate: 0.0 };
        let report = make_report(Some("Q"));
        for _ in 0..10 {
            assert!(!sampler.should_sample(&report, None).unwrap());
        }
    }

    #[test]
    fn at_least_once_keeps_first_occurrence_per_operation_name() {
        let sampler = at_least_once_op_name(0.0);

        let first_a = sampler
            .should_sample(&make_report(Some("OpA")), None)
            .unwrap();
        let first_b = sampler
            .should_sample(&make_report(Some("OpB")), None)
            .unwrap();
        let second_a = sampler
            .should_sample(&make_report(Some("OpA")), None)
            .unwrap();
        let second_b = sampler
            .should_sample(&make_report(Some("OpB")), None)
            .unwrap();

        assert!(first_a, "first OpA should always be sampled");
        assert!(first_b, "first OpB should always be sampled");
        assert!(!second_a, "second OpA should be dropped (rate=0%)");
        assert!(!second_b, "second OpB should be dropped (rate=0%)");
    }

    #[test]
    fn at_least_once_with_full_rate_always_samples() {
        let sampler = at_least_once_op_name(1.0);
        let report = make_report(Some("Op"));
        for _ in 0..5 {
            assert!(sampler.should_sample(&report, None).unwrap());
        }
    }

    #[test]
    fn at_least_once_treats_anonymous_operations_as_one_key() {
        let sampler = at_least_once_op_name(0.0);
        let first = sampler.should_sample(&make_report(None), None).unwrap();
        let second = sampler.should_sample(&make_report(None), None).unwrap();
        assert!(first);
        assert!(
            !second,
            "anonymous operations share the empty-string key, so the second one is dropped"
        );
    }

    #[test]
    fn at_least_once_with_key_expression() {
        let program = ".request.operation.name"
            .compile_expression(None)
            .expect("expression should compile");
        let sampler = Sampler::AtLeastOnce {
            key: AtLeastOnceKey::Expression {
                program: Box::new(program),
            },
            rate: 0.0,
            seen: Cache::new(1024),
        };

        let first_a = sampler
            .should_sample(&make_report(Some("OpA")), None)
            .unwrap();
        let second_a = sampler
            .should_sample(&make_report(Some("OpA")), None)
            .unwrap();
        let first_b = sampler
            .should_sample(&make_report(Some("OpB")), None)
            .unwrap();

        assert!(first_a);
        assert!(!second_a);
        assert!(first_b, "different key string should be considered new");
    }

    #[test]
    fn at_least_once_seen_set_is_bounded_by_max_seen_keys() {
        // Bound the cache to 32 entries and try to insert ~50x that many
        // distinct keys: even with moka's TinyLFU admission window, the
        // resident set has to stay close to the configured cap.
        const CAP: u64 = 32;
        let sampler = at_least_once_op_name_with_capacity(0.0, CAP);
        let seen = match &sampler {
            Sampler::AtLeastOnce { seen, .. } => seen.clone(),
            _ => unreachable!(),
        };

        for i in 0..1500 {
            let report = make_report(Some(&format!("Op{}", i)));
            sampler.should_sample(&report, None).unwrap();
        }
        seen.run_pending_tasks();

        assert!(
            seen.entry_count() <= CAP,
            "seen-set should never exceed max_seen_keys ({}), got {}",
            CAP,
            seen.entry_count(),
        );
    }
}
