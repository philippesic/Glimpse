use super::Event;
pub(super) mod keyboard;
use crate::{App, Task, work_trace::Span};
use accesskit::{
    ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, Node, NodeId,
    TextActionResult, TreeId, TreeInfo, TreeUpdate,
};
use collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    mpsc,
};
use std::time::Duration;

type TextGroups = HashMap<NodeId, Arc<[(NodeId, Node)]>>;

pub(super) struct Frame {
    pub(super) nodes: Vec<(NodeId, Node)>,
    pub(super) text: TextGroups,
    pub(super) tree: TreeInfo,
    pub(super) focus: NodeId,
    pub(super) focused: bool,
}

impl Frame {
    fn update(mut self, previous: &mut TextGroups) -> TreeUpdate {
        let _profile = Span::new("a11y_diff");
        for (owner, nodes) in &self.text {
            if let Some(old) = previous.get(owner) {
                if Arc::ptr_eq(old, nodes) {
                    continue;
                }
                let older: HashMap<_, _> = old.iter().map(|(id, node)| (*id, node)).collect();
                self.nodes.extend(
                    nodes
                        .iter()
                        .filter(|(id, node)| {
                            older
                                .get(id)
                                .is_none_or(|old| !old.shares_data_with(node) && *old != node)
                        })
                        .cloned(),
                );
            } else {
                self.nodes.extend(nodes.iter().cloned());
            }
        }
        *previous = self.text;
        TreeUpdate {
            nodes: self.nodes,
            tree: Some(self.tree),
            tree_id: TreeId::ROOT,
            focus: self.focus,
        }
    }
}

pub(super) struct Publisher {
    pending: Arc<Mutex<Option<Frame>>>,
    wake: async_channel::Sender<()>,
    _task: Task<()>,
}

impl Publisher {
    pub(super) fn new(
        active: Arc<AtomicBool>,
        events: async_channel::Sender<Event>,
        cx: &App,
    ) -> Self {
        let (wake, notifications) = async_channel::bounded(1);
        let pending = Arc::new(Mutex::new(None::<Frame>));
        let activation = Arc::new(AtomicU64::new(0));
        let mut adapter = accesskit_unix::Adapter::new(
            Activate {
                active: active.clone(),
                events: events.clone(),
                activation: activation.clone(),
            },
            Actions(events.clone()),
            Deactivate {
                active: active.clone(),
                events,
                wake: wake.clone(),
            },
        );
        let task = cx.background_executor().spawn({
            let pending = pending.clone();
            async move {
                let mut previous = TextGroups::default();
                let mut last_activation = 0;
                while notifications.recv().await.is_ok() {
                    let frame = pending.lock().unwrap().take();
                    if !active.load(Ordering::Acquire) {
                        previous.clear();
                        continue;
                    }
                    let Some(frame) = frame else {
                        continue;
                    };
                    let _profile = Span::new("a11y_adapter_worker");
                    adapter.update_window_focus_state(frame.focused);
                    adapter.update_if_active(|| {
                        let generation = activation.load(Ordering::Acquire);
                        if generation != last_activation {
                            previous.clear();
                            last_activation = generation;
                        }
                        frame.update(&mut previous)
                    });
                }
            }
        });
        Self {
            pending,
            wake,
            _task: task,
        }
    }

    pub(super) fn submit(&self, frame: Frame) {
        *self.pending.lock().unwrap() = Some(frame);
        let _ = self.wake.try_send(());
    }
}

struct Activate {
    active: Arc<AtomicBool>,
    events: async_channel::Sender<Event>,
    activation: Arc<AtomicU64>,
}
impl ActivationHandler for Activate {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        self.activation.fetch_add(1, Ordering::Release);
        self.active.store(true, Ordering::Release);
        keyboard::activate(self.active.clone());
        let _ = self.events.try_send(Event::Refresh);
        None
    }
}
struct Actions(async_channel::Sender<Event>);

const QUEUED: u8 = 0;
const APPLYING: u8 = 1;
const CANCELLED: u8 = 2;

pub(crate) struct PendingTextAction {
    state: Arc<AtomicU8>,
    reply: mpsc::Sender<bool>,
}

impl PendingTextAction {
    pub(super) fn apply(self, run: impl FnOnce() -> bool) {
        if self
            .state
            .compare_exchange(QUEUED, APPLYING, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let _ = self.reply.send(run());
        }
    }
}

impl ActionHandler for Actions {
    fn do_action(&mut self, request: ActionRequest) {
        let _ = self.0.try_send(Event::Action(request, None, None));
    }

    fn do_text_action(
        &mut self,
        request: ActionRequest,
        revision: Option<usize>,
    ) -> TextActionResult {
        let (reply, result) = mpsc::channel();
        let state = Arc::new(AtomicU8::new(QUEUED));
        let action = PendingTextAction {
            state: state.clone(),
            reply,
        };
        if self
            .0
            .try_send(Event::Action(request, revision, Some(action)))
            .is_err()
        {
            return TextActionResult::Immediate(false);
        }
        TextActionResult::Deferred(Box::new(move || {
            match result.recv_timeout(Duration::from_millis(250)) {
                Ok(applied) => applied,
                Err(mpsc::RecvTimeoutError::Disconnected) => false,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let cancelled = state
                        .compare_exchange(QUEUED, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok();
                    !cancelled && result.recv().unwrap_or(false)
                }
            }
        }))
    }
}
struct Deactivate {
    active: Arc<AtomicBool>,
    events: async_channel::Sender<Event>,
    wake: async_channel::Sender<()>,
}
impl DeactivationHandler for Deactivate {
    fn deactivate_accessibility(&mut self) {
        self.active.store(false, Ordering::Release);
        let _ = self.events.try_send(Event::Refresh);
        let _ = self.wake.try_send(());
    }
}
