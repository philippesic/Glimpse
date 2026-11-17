use crate::accessibility::KeyboardEvent;
use std::{
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    time::{Duration, Instant},
};
use zbus::blocking::{Connection, connection::Builder};

struct Request {
    event: KeyboardEvent,
    response: SyncSender<bool>,
}
enum Message {
    Connect,
    Key(Request),
}
struct Keyboard {
    requests: SyncSender<Message>,
    active: Mutex<Vec<Weak<AtomicBool>>>,
    ready: Arc<AtomicBool>,
    connecting: Arc<AtomicBool>,
}
static KEYBOARD: OnceLock<Keyboard> = OnceLock::new();

fn connection() -> zbus::Result<Connection> {
    let address = match std::env::var("AT_SPI_BUS_ADDRESS")
        .ok()
        .filter(|s| !s.is_empty())
    {
        Some(address) => address,
        None => Builder::session()?
            .method_timeout(Duration::from_millis(500))
            .build()?
            .call_method(
                Some("org.a11y.Bus"),
                "/org/a11y/bus",
                Some("org.a11y.Bus"),
                "GetAddress",
                &(),
            )?
            .body()
            .deserialize::<String>()?,
    };
    Builder::address(address.as_str())?
        .method_timeout(Duration::from_millis(100))
        .build()
}

pub(super) fn activate(active: Arc<AtomicBool>) {
    let keyboard = KEYBOARD.get_or_init(|| {
        let (requests, receiver) = mpsc::sync_channel::<Message>(32);
        let ready = Arc::new(AtomicBool::new(false));
        let connecting = Arc::new(AtomicBool::new(true));
        let worker_ready = ready.clone();
        let worker_connecting = connecting.clone();
        std::thread::spawn(move || {
            let mut bus = connection().ok();
            worker_ready.store(bus.is_some(), Ordering::Release);
            worker_connecting.store(false, Ordering::Release);
            let mut warned = false;
            while let Ok(message) = receiver.recv() {
                let request = match message {
                    Message::Connect => {
                        bus = connection().ok();
                        worker_ready.store(bus.is_some(), Ordering::Release);
                        worker_connecting.store(false, Ordering::Release);
                        continue;
                    }
                    Message::Key(request) => request,
                };
                let result = (|| -> zbus::Result<bool> {
                    if bus.is_none() {
                        bus = Some(connection()?);
                    }
                    let event = request.event;
                    bus.as_ref()
                        .unwrap()
                        .call_method(
                            Some("org.a11y.atspi.Registry"),
                            "/org/a11y/atspi/registry/deviceeventcontroller",
                            Some("org.a11y.atspi.DeviceEventController"),
                            "NotifyListenersSync",
                            &((
                                if event.pressed { 0u32 } else { 1u32 },
                                event.keysym as i32,
                                event.keycode as i32,
                                event.modifiers as i32,
                                event.timestamp as i32,
                                event.text,
                                event.is_text,
                            ),),
                        )?
                        .body()
                        .deserialize()
                })();
                if std::env::var_os("GLIMPSE_TRACE_ATSPI").is_some() {
                    eprintln!("GPUI_ATSPI_REPLY {result:?}");
                }
                match result {
                    Ok(consumed) => {
                        warned = false;
                        let _ = request.response.try_send(consumed);
                    }
                    Err(error) => {
                        if !warned {
                            log::warn!("Assistive keyboard notification unavailable: {error}");
                            warned = true;
                        }
                        bus = None;
                        worker_ready.store(false, Ordering::Release);
                        let _ = request.response.try_send(false);
                    }
                }
            }
        });
        Keyboard {
            requests,
            active: Mutex::new(vec![]),
            ready,
            connecting,
        }
    });
    let mut listeners = keyboard.active.lock().unwrap();
    listeners.retain(|listener| listener.strong_count() > 0);
    if !listeners
        .iter()
        .any(|listener| listener.ptr_eq(&Arc::downgrade(&active)))
    {
        listeners.push(Arc::downgrade(&active));
    }
}

pub(crate) fn active() -> bool {
    KEYBOARD.get().is_some_and(|keyboard| {
        keyboard.active.lock().unwrap().iter().any(|listener| {
            listener
                .upgrade()
                .is_some_and(|active| active.load(Ordering::Acquire))
        })
    })
}

pub(crate) fn notify(event: KeyboardEvent) -> bool {
    if !active() {
        return false;
    }
    let keyboard = KEYBOARD.get().unwrap();
    if !keyboard.ready.load(Ordering::Acquire) {
        if !keyboard.connecting.swap(true, Ordering::AcqRel)
            && keyboard.requests.try_send(Message::Connect).is_err()
        {
            keyboard.connecting.store(false, Ordering::Release);
        }
        return false;
    }
    let started = Instant::now();
    let keycode = event.keycode;
    let (response, receiver) = mpsc::sync_channel(1);
    if keyboard
        .requests
        .try_send(Message::Key(Request { event, response }))
        .is_err()
    {
        return false;
    }
    let result = receiver.recv_timeout(Duration::from_millis(110));
    if std::env::var_os("GLIMPSE_TRACE_ATSPI").is_some() {
        eprintln!(
            "GPUI_ATSPI_KEY {keycode} {} {result:?}",
            started.elapsed().as_micros()
        );
    }
    result.unwrap_or(false)
}
