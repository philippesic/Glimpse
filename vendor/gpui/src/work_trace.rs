use std::{sync::OnceLock, time::Instant};

pub(crate) struct Span {
    started: Option<(Instant, &'static str)>,
}

fn origin() -> Option<Instant> {
    static ORIGIN: OnceLock<Option<Instant>> = OnceLock::new();
    *ORIGIN.get_or_init(|| std::env::var_os("GPUI_WORK_TRACE").map(|_| Instant::now()))
}

impl Span {
    pub(crate) fn new(name: &'static str) -> Self {
        Self {
            started: origin().map(|_| (Instant::now(), name)),
        }
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        if let Some((started, name)) = self.started {
            let ended = Instant::now();
            eprintln!(
                "GPUI_WORK {name} {} {}",
                started.duration_since(origin().unwrap()).as_nanos(),
                ended.duration_since(started).as_nanos()
            );
        }
    }
}
