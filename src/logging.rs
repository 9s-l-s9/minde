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
        self.number(field, value);
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
}
