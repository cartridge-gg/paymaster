use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time;
use tracing_subscriber::{EnvFilter, Layer};

const DEFAULT_LOG_FILTER: &str = "info";
const DEFAULT_TIMESTAMP_FORMAT: &str = "%Y-%m-%d %H:%M:%S%.3f %Z";

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
    pub fn layer<S>() -> (impl Layer<S>, EnvFilter)
    where
        S: for<'span> tracing_subscriber::registry::LookupSpan<'span> + tracing::Subscriber,
    {
        let ansi = std::io::IsTerminal::is_terminal(&std::io::stdout());

        let default_filter = EnvFilter::try_new(DEFAULT_LOG_FILTER);
        let filter = EnvFilter::try_from_default_env().or(default_filter).expect("valid env filter");

        let layer = tracing_subscriber::fmt::layer().with_timer(LocalTime).with_ansi(ansi);

        (layer, filter)
    }
}
