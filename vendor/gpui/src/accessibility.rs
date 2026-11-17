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
use accesskit::{ActionRequest, Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};
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
    Action(ActionRequest),
}

#[derive(Default)]
pub(crate) struct State {
    active: Arc<AtomicBool>,
    initialized: bool,
    next_id: u64,
    ids: HashMap<Vec<ElementId>, NodeId>,
    child_ids: HashMap<(NodeId, ElementId), NodeId>,
    visible: HashSet<NodeId>,
    text_groups: HashMap<NodeId, Arc<[(NodeId, Node)]>>,
    pub(crate) parents: Vec<NodeId>,
    pub(crate) title: String,
    _task: Option<Task<()>>,
    #[cfg(target_os = "linux")]
    adapter: Option<accesskit_unix::Adapter>,
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
            self.adapter = Some(linux::adapter(self.active.clone(), tx));
            self._task = Some(cx.spawn(async move |cx| {
                while let Ok(event) = rx.recv().await {
                    if handle
                        .update(cx, |_, window, cx| match event {
                            Event::Refresh => {
                                window.accessibility.text_groups.clear();
                                window.refresh();
                            }
                            Event::Action(request) => {
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
        for record in records {
            if let Some(parent) = nodes.get_mut(&record.parent) {
                parent.push_child(record.id);
            }
        }
        if let Some(modal) = modal {
            nodes.get_mut(&NodeId(1)).unwrap().set_children(vec![modal]);
        }
        let mut pending = vec![NodeId(1)];
        let mut visible = HashSet::default();
        while let Some(id) = pending.pop() {
            if !visible.insert(id) {
                continue;
            }
            if let Some(node) = nodes.get(&id) {
                pending.extend_from_slice(node.children());
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
        let current_ids: HashSet<_> = nodes
            .keys()
            .copied()
            .chain(
                records
                    .iter()
                    .flat_map(|record| record.info.text.iter().map(|(id, _)| *id)),
            )
            .collect();
        let mut text_groups = HashMap::default();
        let mut changed_text = Vec::new();
        for record in records.iter().filter(|record| visible.contains(&record.id)) {
            if !self
                .text_groups
                .get(&record.id)
                .is_some_and(|previous| Arc::ptr_eq(previous, &record.info.text))
            {
                changed_text.extend(record.info.text.iter().cloned());
            }
            if !record.info.text.is_empty() {
                text_groups.insert(record.id, record.info.text.clone());
            }
        }
        self.text_groups = text_groups;
        let update = TreeUpdate {
            nodes: nodes
                .into_iter()
                .filter(|(id, _)| visible.contains(id))
                .chain(changed_text)
                .collect(),
            tree: Some(tree),
            tree_id: TreeId::ROOT,
            focus,
        };
        self.ids.retain(|_, id| current_ids.contains(id));
        self.child_ids.retain(|_, id| current_ids.contains(id));
        self.visible = visible;
        #[cfg(target_os = "linux")]
        if let Some(adapter) = &mut self.adapter {
            adapter.update_window_focus_state(focused);
            adapter.update_if_active(|| update);
        }
        #[cfg(not(target_os = "linux"))]
        let _ = (focused, update);
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
        self.accessibility.next_id = self.accessibility.next_id.max(1) + 1;
        let id = NodeId(self.accessibility.next_id);
        self.accessibility.ids.insert(global_id.0.to_vec(), id);
        id
    }
    /// Allocate a stable text-run identifier below a semantic element.
    pub fn accessibility_child_id(
        &mut self,
        parent: &GlobalElementId,
        id: impl Into<ElementId>,
    ) -> NodeId {
        let parent = self.accessibility_id(parent);
        *self
            .accessibility
            .child_ids
            .entry((parent, id.into()))
            .or_insert_with(|| {
                self.accessibility.next_id = self.accessibility.next_id.max(1) + 1;
                NodeId(self.accessibility.next_id)
            })
    }
}
