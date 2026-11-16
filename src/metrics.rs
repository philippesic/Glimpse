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
