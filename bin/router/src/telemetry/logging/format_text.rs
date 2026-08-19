use std::cell::RefCell;

use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::format::{Compact, Format, Writer};
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

use crate::telemetry::logging::request_id::REQUEST_IDENTIFIERS;
use crate::telemetry::logging::summary::REQUEST_SUMMARY;
use crate::telemetry::logging::targets;

pub struct RouterTextFormat<T>(pub Format<Compact, T>);

impl<S, N, T> FormatEvent<S, N> for RouterTextFormat<T>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
    T: FormatTime,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        thread_local! {
            static BUF: RefCell<String> = const { RefCell::new(String::new()) };
        }

        BUF.with(|cell| {
            let mut buf = cell.borrow_mut();
            buf.clear();

            self.0.format_event(ctx, Writer::new(&mut *buf), event)?;

            let line = buf.strip_suffix('\n').unwrap_or(&buf);
            writer.write_str(line)?;

            let _ = REQUEST_IDENTIFIERS.try_with(|ids| -> std::fmt::Result {
                write!(writer, " request_id={:?}", ids.req_id())?;
                if let Some(trace_id) = ids.trace_id() {
                    write!(writer, " trace_id={:?}", trace_id)?;
                }
                if let Ok(correlations) = ids.correlations.lock() {
                    for (key, value) in correlations.iter() {
                        write!(writer, " {key}={value:?}")?;
                    }
                }
                Ok(())
            });

            if event.metadata().target() == targets::SUMMARY {
                let _ = REQUEST_SUMMARY.try_with(|summary| -> std::fmt::Result {
                    if let Ok(custom) = summary.custom.lock() {
                        for (key, value) in custom.iter() {
                            write!(writer, " {key}={value}")?;
                        }
                    }
                    Ok(())
                });
            }

            writer.write_char('\n')
        })
    }
}
