//! Opt-in CPU timings; these exclude compositor and physical display delivery.
use std::{sync::OnceLock, time::Instant};

pub(crate) struct Span(Option<(&'static str, Instant)>);
impl Span {
    pub(crate) fn new(name: &'static str) -> Self {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        Self(
            ENABLED
                .get_or_init(|| std::env::var_os("GLIMPSE_PERF_TRACE").is_some())
                .then(|| (name, Instant::now())),
        )
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        if let Some((name, started)) = self.0 {
            let micros = started.elapsed().as_secs_f64() * 1_000_000.;
            eprintln!("PERF_CPU_US {name} {micros:.3}");
        }
    }
}
