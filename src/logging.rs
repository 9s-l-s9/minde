// SPDX-License-Identifier: GPL-3.0-or-later

//! Human and dependency-free JSON tracing formats.

use std::fmt::{self, Write as _};

use tracing::{Event, Subscriber, field::Visit};
use tracing_subscriber::{
    EnvFilter,
    fmt::{FmtContext, FormatEvent, FormatFields, format::Writer},
    registry::LookupSpan,
};

pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if std::env::var("MINDE_LOG_FORMAT").as_deref() == Ok("json") {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .event_format(JsonEventFormat)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
}

#[derive(Debug, Clone, Copy)]
struct JsonEventFormat;

impl<S, N> FormatEvent<S, N> for JsonEventFormat
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        _context: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let metadata = event.metadata();
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis());
        let mut visitor = JsonVisitor::default();
        event.record(&mut visitor);

        write!(
            writer,
            "{{\"timestamp_ms\":{timestamp_ms},\"level\":\"{}\",\"target\":\"{}\",\"fields\":{{",
            metadata.level(),
            escape_json(metadata.target())
        )?;
        for (index, (name, value)) in visitor.fields.iter().enumerate() {
            if index > 0 {
                writer.write_str(",")?;
            }
            write!(writer, "\"{}\":{value}", escape_json(name))?;
        }
        writer.write_str("}}\n")
    }
}

#[derive(Default)]
struct JsonVisitor {
    fields: Vec<(String, String)>,
}

impl JsonVisitor {
    fn string(&mut self, field: &tracing::field::Field, value: &str) {
        self.fields.push((
            field.name().to_owned(),
            format!("\"{}\"", escape_json(value)),
        ));
    }

    fn number(&mut self, field: &tracing::field::Field, value: impl fmt::Display) {
        self.fields
            .push((field.name().to_owned(), value.to_string()));
    }
}

impl Visit for JsonVisitor {
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.number(field, value);
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.number(field, value);
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.number(field, value);
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        if value.is_finite() {
            self.number(field, value);
        } else {
            // JSON has no non-finite numbers. Keep their diagnostic meaning.
            self.string(field, &value.to_string());
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.string(field, value);
    }

    fn record_error(
        &mut self,
        field: &tracing::field::Field,
        value: &(dyn std::error::Error + 'static),
    ) {
        self.string(field, &value.to_string());
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        self.string(field, &format!("{value:?}"));
    }
}

fn escape_json(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character < ' ' => {
                let _ = write!(escaped, "\\u{:04x}", character as u32);
            }
            character => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_strings_escape_control_and_quote_characters() {
        assert_eq!(escape_json("a\n\"b\\c"), "a\\n\\\"b\\\\c");
    }

    #[test]
    fn json_events_preserve_non_finite_floats_as_strings() {
        use std::{
            io::{Read, Write},
            os::unix::net::UnixStream,
            process::{Command, Stdio},
            sync::Arc,
            time::Duration,
        };

        let (mut reader, writer) = UnixStream::pair().unwrap();
        reader
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(Arc::new(writer))
            .event_format(JsonEventFormat)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(
                target: "minde::logging::test",
                nan = f64::NAN,
                positive = f64::INFINITY,
                negative = f64::NEG_INFINITY,
                f32_nan = f32::NAN,
                finite = -1.5_f64,
                zero = -0.0_f64,
                largest = f64::MAX,
                smallest = f64::MIN,
                subnormal = f64::from_bits(1),
                signed = i64::MIN,
                unsigned = u64::MAX,
                boolean = true,
                text = "quote\" newline\n backslash\\",
            );
            tracing::info!(target: "minde::logging::test", finite = 2.5_f64);
        });
        let mut lines = String::new();
        reader.read_to_string(&mut lines).unwrap();
        assert_eq!(lines.lines().count(), 2, "{lines}");

        // Use the JSON parser already provided by the development and check
        // manifests, keeping the compositor's dependency graph unchanged.
        let mut parser = Command::new("guile")
            .args([
                "--no-auto-compile",
                "-c",
                r#"(use-modules (json) (ice-9 rdelim))
                (define first (json-string->scm (read-line)))
                (define fields (assoc-ref first "fields"))
                (unless (and (integer? (assoc-ref first "timestamp_ms"))
                             (string=? (assoc-ref first "level") "INFO")
                             (string=? (assoc-ref first "target") "minde::logging::test"))
                  (error "invalid event metadata" first))
                (for-each
                 (lambda (entry)
                   (let ((actual (assoc-ref fields (car entry))) (expected (cdr entry)))
                     (unless (if (number? expected)
                                 (and (number? actual)
                                      (= (if (inexact? expected) (exact->inexact actual) actual)
                                         expected))
                                 (equal? actual expected))
                       (error "field did not round trip" entry actual))))
                 '(("nan" . "NaN") ("positive" . "inf") ("negative" . "-inf")
                   ("f32_nan" . "NaN") ("finite" . -1.5) ("zero" . 0)
                   ("largest" . 1.7976931348623157e308)
                   ("smallest" . -1.7976931348623157e308)
                   ("subnormal" . 5e-324) ("signed" . -9223372036854775808)
                   ("unsigned" . 18446744073709551615) ("boolean" . #t)
                   ("text" . "quote\" newline\n backslash\\")))
                (unless (= (assoc-ref (assoc-ref (json-string->scm (read-line)) "fields")
                                      "finite") 2.5)
                  (error "following event did not parse"))
                (unless (eof-object? (read-line)) (error "unexpected trailing data"))"#,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        parser
            .stdin
            .take()
            .unwrap()
            .write_all(lines.as_bytes())
            .unwrap();
        let output = parser.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{lines}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
