use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time;
use tracing_subscriber::{EnvFilter, Layer};

const DEFAULT_LOG_FILTER: &str = "info";
const DEFAULT_LOG_FORMAT: LogFormat = LogFormat::Text;
const DEFAULT_TIMESTAMP_FORMAT: &str = "%Y-%m-%d %H:%M:%S%.3f %Z";
const LOG_FORMAT_ENV: &str = "PAYMASTER_LOG_FORMAT";

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum LogFormat {
    Text,
    Json,
}

impl LogFormat {
    fn from_env() -> Self {
        match std::env::var(LOG_FORMAT_ENV) {
            Ok(value) => Self::parse(&value).unwrap_or_else(|| panic!("invalid {LOG_FORMAT_ENV}={value:?}; expected one of: text, json")),
            Err(std::env::VarError::NotPresent) => DEFAULT_LOG_FORMAT,
            Err(std::env::VarError::NotUnicode(value)) => {
                panic!("invalid {LOG_FORMAT_ENV}={value:?}; value must be valid UTF-8")
            },
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "text" | "plain" | "compact" => Some(Self::Text),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

/// Formats timestamps in local time.
///
/// Example output: `2025-08-24 20:49:32.487 -04:00`
#[derive(Debug, Clone, Default)]
struct LocalTime;

impl time::FormatTime for LocalTime {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        let time = chrono::Local::now();
        write!(w, "{}", time.format(DEFAULT_TIMESTAMP_FORMAT))
    }
}

pub struct Fmt;

impl Fmt {
    pub fn layer<S>() -> (Box<dyn Layer<S> + Send + Sync + 'static>, EnvFilter)
    where
        S: for<'span> tracing_subscriber::registry::LookupSpan<'span> + tracing::Subscriber,
    {
        let ansi = std::io::IsTerminal::is_terminal(&std::io::stdout());

        let default_filter = EnvFilter::try_new(DEFAULT_LOG_FILTER);
        let filter = EnvFilter::try_from_default_env().or(default_filter).expect("valid env filter");

        let layer = match LogFormat::from_env() {
            LogFormat::Text => tracing_subscriber::fmt::layer().with_timer(LocalTime).with_ansi(ansi).boxed(),
            LogFormat::Json => tracing_subscriber::fmt::layer()
                .json()
                .flatten_event(true)
                .with_current_span(true)
                .with_span_list(true)
                .with_timer(LocalTime)
                .boxed(),
        };

        (layer, filter)
    }
}

#[cfg(test)]
mod tests {
    use super::{LogFormat, DEFAULT_LOG_FORMAT};

    #[test]
    fn parses_log_format_values() {
        assert_eq!(LogFormat::parse(""), Some(DEFAULT_LOG_FORMAT));
        assert_eq!(LogFormat::parse("text"), Some(LogFormat::Text));
        assert_eq!(LogFormat::parse("plain"), Some(LogFormat::Text));
        assert_eq!(LogFormat::parse("compact"), Some(LogFormat::Text));
        assert_eq!(LogFormat::parse("json"), Some(LogFormat::Json));
        assert_eq!(LogFormat::parse(" JSON "), Some(LogFormat::Json));
        assert_eq!(LogFormat::parse("xml"), None);
    }
}
