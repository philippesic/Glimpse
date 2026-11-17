//! Semantic elements and assistive-technology actions for native windows.

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
#[derive(Clone)]
pub(crate) struct KeyboardEvent {
    pub pressed: bool,
    pub keycode: u32,
    pub keysym: u32,
    pub modifiers: u32,
    pub timestamp: u32,
    pub text: String,
    pub is_text: bool,
}

#[cfg(target_os = "linux")]
pub(crate) fn notify_wayland_key(event: KeyboardEvent) -> bool {
    linux::keyboard::notify(event)
}

#[cfg(target_os = "linux")]
pub(crate) fn wayland_key_notifications_active() -> bool {
    linux::keyboard::active()
}

use crate::{
    AnyElement, AnyWindowHandle, App, Bounds, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Task, Window,
};
use accesskit::{ActionRequest, Node, NodeId, Role, TreeInfo};
use collections::{HashMap, HashSet};
use std::{
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

type Handler = Rc<dyn Fn(ActionRequest, Bounds<Pixels>, &mut Window, &mut App)>;

#[cfg(target_os = "linux")]
pub(crate) enum Event {
    Refresh,
    Action(ActionRequest, Option<usize>),
}

#[derive(Default)]
pub(crate) struct State {
    active: Arc<AtomicBool>,
    initialized: bool,
    next_id: u32,
    ids: HashMap<Vec<ElementId>, NodeId>,
    visible: HashSet<NodeId>,
    pub(crate) parents: Vec<NodeId>,
    pub(crate) title: String,
    _task: Option<Task<()>>,
    #[cfg(target_os = "linux")]
    publisher: Option<linux::Publisher>,
}

#[derive(Clone)]
pub(crate) struct Record {
    id: NodeId,
    parent: NodeId,
    bounds: Bounds<Pixels>,
    info: AccessibilityInfo,
    handler: Option<Handler>,
}

/// The semantics associated with a painted element and its text runs.
#[derive(Clone)]
pub struct AccessibilityInfo {
    /// The element's role, label, state and supported actions.
    pub node: Node,
    /// Additional text-run nodes owned by this element.
    pub text: Arc<[(NodeId, Node)]>,
    /// Whether this element has keyboard focus.
    pub focused: bool,
    /// Whether assistive access is contained within this element.
    pub modal: bool,
}

/// The text-run ID namespace of one rendered accessible element.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TextRunIds(u32);

impl TextRunIds {
    /// A stable ID for a run key supplied by the text owner.
    pub fn get(self, key: usize) -> NodeId {
        let key = u32::try_from(key).expect("accessibility text run key exhausted");
        NodeId((u64::from(self.0) << 32) | u64::from(key))
    }
}
impl AccessibilityInfo {
    /// Describe an element without additional text runs.
    pub fn new(node: Node) -> Self {
        Self {
            node,
            text: Arc::from([]),
            focused: false,
            modal: false,
        }
    }
}

/// An element whose semantics and actions follow its rendered lifetime.
pub struct Accessible {
    id: ElementId,
    child: AnyElement,
    build: Box<dyn Fn(&GlobalElementId, &mut Window, &mut App) -> AccessibilityInfo>,
    handler: Option<Handler>,
}

/// Wrap a native element with lazily generated assistive semantics.
pub fn accessible(
    id: impl Into<ElementId>,
    child: impl IntoElement,
    build: impl Fn(&GlobalElementId, &mut Window, &mut App) -> AccessibilityInfo + 'static,
) -> Accessible {
    Accessible {
        id: id.into(),
        child: child.into_any_element(),
        build: Box::new(build),
        handler: None,
    }
}
impl Accessible {
    /// Handle supported actions on the UI thread.
    pub fn on_action(
        mut self,
        handler: impl Fn(ActionRequest, Bounds<Pixels>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.handler = Some(Rc::new(handler));
        self
    }
}
impl IntoElement for Accessible {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for Accessible {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
    }
    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if !window.accessibility.active.load(Ordering::Acquire) {
            self.child.paint(window, cx);
            return;
        }
        let global_id = global_id.unwrap();
        let id = window.accessibility_id(global_id);
        let mut info = (self.build)(global_id, window, cx);
        info.node.set_bounds(accesskit::Rect::new(
            bounds.left().into(),
            bounds.top().into(),
            bounds.right().into(),
            bounds.bottom().into(),
        ));
        let parent = window
            .accessibility
            .parents
            .last()
            .copied()
            .unwrap_or(NodeId(1));
        window.next_frame.accessibility.push(Record {
            id,
            parent,
            bounds,
            info,
            handler: self.handler.clone(),
        });
        window.accessibility.parents.push(id);
        self.child.paint(window, cx);
        window.accessibility.parents.pop();
    }
}
impl State {
    pub(crate) fn initialize(&mut self, handle: AnyWindowHandle, cx: &App) {
        if self.initialized {
            return;
        }
        self.initialized = true;
        #[cfg(target_os = "linux")]
        {
            let (tx, rx) = async_channel::unbounded();
            self.publisher = Some(linux::Publisher::new(self.active.clone(), tx, cx));
            self._task = Some(cx.spawn(async move |cx| {
                while let Ok(event) = rx.recv().await {
                    if handle
                        .update(cx, |_, window, cx| match event {
                            Event::Refresh => window.refresh(),
                            Event::Action(request, text_revision) => {
                                if !window.accessibility.active.load(Ordering::Acquire) {
                                    return;
                                }
                                let handler = window
                                    .rendered_frame
                                    .accessibility
                                    .iter()
                                    .find(|record| {
                                        record.id == request.target_node
                                            && window.accessibility.visible.contains(&record.id)
                                            && record.info.node.supports_action(request.action)
                                            && (!matches!(
                                                request.action,
                                                accesskit::Action::SetValue
                                                    | accesskit::Action::SetTextSelection
                                                    | accesskit::Action::ReplaceSelectedText
                                            ) || record.info.node.text_revision()
                                                == text_revision)
                                    })
                                    .and_then(|record| {
                                        record
                                            .handler
                                            .clone()
                                            .map(|handler| (handler, record.bounds))
                                    });
                                if let Some((handler, bounds)) = handler {
                                    if request.action == accesskit::Action::Focus {
                                        window.activate_window();
                                    }
                                    handler(request, bounds, window, cx);
                                }
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        }
        #[cfg(not(target_os = "linux"))]
        let _ = (handle, cx);
    }
    pub(crate) fn publish(&mut self, records: &[Record], focused: bool) {
        let _profile = crate::work_trace::Span::new("a11y");
        if !self.active.load(Ordering::Acquire) {
            return;
        }
        let mut nodes: HashMap<NodeId, Node> = HashMap::default();
        let mut root = Node::new(Role::Window);
        root.set_label(if self.title.is_empty() {
            "Application window"
        } else {
            &self.title
        });
        let modal = records
            .iter()
            .rev()
            .find(|record| record.info.modal)
            .map(|record| record.id);
        for record in records {
            nodes.insert(record.id, record.info.node.clone());
        }
        nodes.insert(NodeId(1), root);
        let mut rendered_children: HashMap<NodeId, Vec<NodeId>> = HashMap::default();
        for record in records {
            if let Some(parent) = nodes.get_mut(&record.parent) {
                parent.push_child(record.id);
                rendered_children
                    .entry(record.parent)
                    .or_default()
                    .push(record.id);
            }
        }
        if let Some(modal) = modal {
            nodes.get_mut(&NodeId(1)).unwrap().set_children(vec![modal]);
            rendered_children.insert(NodeId(1), vec![modal]);
        }
        let mut pending = vec![NodeId(1)];
        let mut visible = HashSet::default();
        while let Some(id) = pending.pop() {
            if !visible.insert(id) {
                continue;
            }
            if let Some(children) = rendered_children.get(&id) {
                pending.extend_from_slice(children);
            }
        }
        let focus = records
            .iter()
            .rev()
            .find(|record| record.info.focused && visible.contains(&record.id))
            .map(|record| record.id)
            .unwrap_or(modal.unwrap_or(NodeId(1)));
        let mut tree = TreeInfo::new(NodeId(1));
        tree.toolkit_name = Some("GPUI".into());
        tree.toolkit_version = Some(env!("CARGO_PKG_VERSION").into());
        self.ids.retain(|_, id| nodes.contains_key(id));
        #[cfg(target_os = "linux")]
        if let Some(publisher) = &self.publisher {
            publisher.submit(linux::Frame {
                nodes: nodes
                    .into_iter()
                    .filter(|(id, _)| visible.contains(id))
                    .collect(),
                text: records
                    .iter()
                    .filter(|record| visible.contains(&record.id) && !record.info.text.is_empty())
                    .map(|record| (record.id, record.info.text.clone()))
                    .collect(),
                tree,
                focus,
                focused,
            });
        }
        #[cfg(not(target_os = "linux"))]
        let _ = (focused, focus, tree);
        self.visible = visible;
    }
}
impl Window {
    /// Activate a rendered control through its normal pointer event handlers.
    pub fn dispatch_accessible_click(&mut self, bounds: Bounds<Pixels>, cx: &mut App) {
        let position = bounds.center();
        self.dispatch_event(
            crate::PlatformInput::MouseDown(crate::MouseDownEvent {
                position,
                button: crate::MouseButton::Left,
                click_count: 1,
                ..Default::default()
            }),
            cx,
        );
        self.dispatch_event(
            crate::PlatformInput::MouseUp(crate::MouseUpEvent {
                position,
                button: crate::MouseButton::Left,
                click_count: 1,
                ..Default::default()
            }),
            cx,
        );
    }
    fn accessibility_id(&mut self, global_id: &GlobalElementId) -> NodeId {
        if let Some(id) = self.accessibility.ids.get(global_id.0.as_slice()) {
            return *id;
        }
        self.accessibility.next_id = self
            .accessibility
            .next_id
            .max(1)
            .checked_add(1)
            .expect("accessibility element ID exhausted");
        let id = NodeId(u64::from(self.accessibility.next_id));
        self.accessibility.ids.insert(global_id.0.to_vec(), id);
        id
    }
    /// Reserve the text-run namespace for a semantic element.
    pub fn accessibility_text_run_ids(&mut self, parent: &GlobalElementId) -> TextRunIds {
        TextRunIds(self.accessibility_id(parent).0 as u32)
    }
}
