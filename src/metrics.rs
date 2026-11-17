use std::{sync::OnceLock, time::Instant};

pub fn event(name: &str, document: u64, generation: u64) {
    static START: OnceLock<Option<Instant>> = OnceLock::new();
    if let Some(start) =
        START.get_or_init(|| std::env::var_os("GLIMPSE_TRACE").map(|_| Instant::now()))
    {
        eprintln!(
            "GLIMPSE_TRACE {} {name} {document} {generation}",
            start.elapsed().as_micros()
        );
    }
}

#[cfg(feature = "test-support")]
pub fn delay(operation: &str) {
    if let Ok(value) = std::env::var(format!("GLIMPSE_TEST_{operation}_DELAY_MS"))
        && let Ok(milliseconds) = value.parse::<u64>()
    {
        std::thread::sleep(std::time::Duration::from_millis(milliseconds.min(10_000)));
    }
}
