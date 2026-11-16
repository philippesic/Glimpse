use super::Event;
use accesskit::{ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, TreeUpdate};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

struct Activate {
    active: Arc<AtomicBool>,
    events: async_channel::Sender<Event>,
}
impl ActivationHandler for Activate {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        self.active.store(true, Ordering::Release);
        let _ = self.events.try_send(Event::Refresh);
        None
    }
}
struct Actions(async_channel::Sender<Event>);
impl ActionHandler for Actions {
    fn do_action(&mut self, request: ActionRequest) {
        let _ = self.0.try_send(Event::Action(request));
    }
}
struct Deactivate {
    active: Arc<AtomicBool>,
    events: async_channel::Sender<Event>,
}
impl DeactivationHandler for Deactivate {
    fn deactivate_accessibility(&mut self) {
        self.active.store(false, Ordering::Release);
        let _ = self.events.try_send(Event::Refresh);
    }
}
pub(super) fn adapter(
    active: Arc<AtomicBool>,
    events: async_channel::Sender<Event>,
) -> accesskit_unix::Adapter {
    accesskit_unix::Adapter::new(
        Activate {
            active: active.clone(),
            events: events.clone(),
        },
        Actions(events.clone()),
        Deactivate { active, events },
    )
}
