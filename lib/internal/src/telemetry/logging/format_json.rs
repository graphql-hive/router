use std::{cell::RefCell, fmt::Write};

use tracing::{
    field::{Field, Visit},
    Event, Subscriber,
};
use tracing_subscriber::{
    fmt::{format::Writer, FmtContext, FormatEvent, FormatFields},
    registry::LookupSpan,
};

use crate::telemetry::logging::request_id::REQUEST_IDENTIFIERS;

#[inline]
fn write_json_body<W: std::fmt::Write>(w: &mut W, s: &str) -> std::fmt::Result {
    let bytes = s.as_bytes();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if b >= 0x20 && b != b'"' && b != b'\\' {
            continue;
        }
        if start < i {
            w.write_str(&s[start..i])?;
        }
        match b {
            b'"' => w.write_str("\\\"")?,
            b'\\' => w.write_str("\\\\")?,
            b'\n' => w.write_str("\\n")?,
            b'\r' => w.write_str("\\r")?,
            b'\t' => w.write_str("\\t")?,
            _ => write!(w, "\\u{b:04x}")?,
        }
        start = i + 1;
    }
    if start < bytes.len() {
        w.write_str(&s[start..])?;
    }

    Ok(())
}

#[inline]
fn write_json_str<W: std::fmt::Write>(w: &mut W, s: &str) -> std::fmt::Result {
    w.write_char('"')?;
    write_json_body(w, s)?;
    w.write_char('"')
}

struct JsonEscape<'a, W: std::fmt::Write>(&'a mut W);

impl<W: std::fmt::Write> std::fmt::Write for JsonEscape<'_, W> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        write_json_body(self.0, s)
    }
}

struct JsonVisitor<'a> {
    buf: &'a mut String,
}

impl JsonVisitor<'_> {
    fn key(&mut self, name: &str) {
        self.buf.push(',');
        let _ = write_json_str(&mut *self.buf, name);
        self.buf.push(':');
    }
}

impl Visit for JsonVisitor<'_> {
    fn record_str(&mut self, f: &Field, v: &str) {
        self.key(f.name());
        let _ = write_json_str(&mut *self.buf, v);
    }
    fn record_i64(&mut self, f: &Field, v: i64) {
        self.key(f.name());
        let _ = write!(self.buf, "{v}");
    }
    fn record_u64(&mut self, f: &Field, v: u64) {
        self.key(f.name());
        let _ = write!(self.buf, "{v}");
    }
    fn record_f64(&mut self, f: &Field, v: f64) {
        self.key(f.name());
        if v.is_finite() {
            let _ = write!(self.buf, "{v}");
        } else {
            let _ = write!(self.buf, "\"{v}\"");
        }
    }
    fn record_bool(&mut self, f: &Field, v: bool) {
        self.key(f.name());
        self.buf.push_str(if v { "true" } else { "false" });
    }
    fn record_error(&mut self, f: &Field, v: &(dyn std::error::Error + 'static)) {
        self.key(f.name());
        self.buf.push('"');
        let _ = write!(JsonEscape(&mut *self.buf), "{v}");
        self.buf.push('"');
    }
    fn record_debug(&mut self, f: &Field, v: &dyn std::fmt::Debug) {
        self.key(f.name());
        self.buf.push('"');
        let _ = write!(JsonEscape(&mut *self.buf), "{v:?}");
        self.buf.push('"');
    }
}

pub struct RouterJsonFormat;

impl<S, N> FormatEvent<S, N> for RouterJsonFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        thread_local! {
            static BUF: RefCell<String> = const { RefCell::new(String::new()) };
        }

        let meta = event.metadata();

        BUF.with(|cell| {
            let mut buf = cell.borrow_mut();
            buf.clear();

            buf.push_str("{\"timestamp\":\"");
            let _ = write!(
                buf,
                "{}",
                chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ")
            );
            buf.push_str("\",\"level\":\"");
            buf.push_str(meta.level().as_str());
            buf.push_str("\",\"target\":");
            write_json_str(&mut *buf, meta.target())?;

            let _ = REQUEST_IDENTIFIERS.try_with(|ids| -> std::fmt::Result {
                buf.push_str(",\"request_id\":");
                write_json_str(&mut *buf, ids.req_id())?;
                if let Some(trace_id) = ids.trace_id() {
                    buf.push_str(",\"trace_id\":");
                    write_json_str(&mut *buf, trace_id)?;
                }
                Ok(())
            });

            event.record(&mut JsonVisitor { buf: &mut buf });

            buf.push_str("}\n");
            writer.write_str(&buf)
        })
    }
}
