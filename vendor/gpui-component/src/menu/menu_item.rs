use crate::{h_flex, ActiveTheme, Disableable, StyledExt};
use gpui::{
    prelude::FluentBuilder as _, AnyElement, App, ClickEvent, ElementId, InteractiveElement,
    IntoElement, MouseButton, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement as _, StyleRefinement, Styled, Window,
};
use smallvec::SmallVec;
use std::rc::Rc;

#[derive(IntoElement)]
pub(crate) struct MenuItemElement {
    id: ElementId,
    label: SharedString,
    focus: Option<gpui::FocusHandle>,
    on_focus: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    role: gpui::accesskit::Role,
    group_name: SharedString,
    style: StyleRefinement,
    disabled: bool,
    selected: bool,
    on_click: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
    on_hover: Option<Box<dyn Fn(&bool, &mut Window, &mut App) + 'static>>,
    children: SmallVec<[AnyElement; 2]>,
}

impl MenuItemElement {
    /// Create a new MenuItem with the given ID and group name.
    pub(crate) fn new(id: impl Into<ElementId>, group_name: impl Into<SharedString>) -> Self {
        let id: ElementId = id.into();
        Self {
            id: id.clone(),
            label: "Menu item".into(),
            focus: None,
            on_focus: None,
            role: gpui::accesskit::Role::MenuItem,
            group_name: group_name.into(),
            style: StyleRefinement::default(),
            disabled: false,
            selected: false,
            on_click: None,
            on_hover: None,
            children: SmallVec::new(),
        }
    }

    pub(crate) fn semantics(
        mut self,
        label: SharedString,
        role: gpui::accesskit::Role,
        focus: gpui::FocusHandle,
        on_focus: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.label = label;
        self.role = role;
        self.focus = Some(focus);
        self.on_focus = Some(Rc::new(on_focus));
        self
    }

    /// Set ListItem as the selected item style.
    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Set the disabled state of the MenuItem.
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set a handler for when the MenuItem is clicked.
    pub(crate) fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    /// Set a handler for when the mouse enters the MenuItem.
    #[allow(unused)]
    pub fn on_hover(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_hover = Some(Box::new(handler));
        self
    }
}

impl Disableable for MenuItemElement {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Styled for MenuItemElement {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for MenuItemElement {
    fn extend(&mut self, elements: impl IntoIterator<Item = gpui::AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for MenuItemElement {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let semantic_id = self.id.clone();
        let label = self.label;
        let role = self.role;
        let disabled = self.disabled;
        let selected = self.selected;
        let focus = self.focus;
        let on_focus = self.on_focus;
        let on_click = self.on_click.clone();
        let rendered = h_flex()
            .id(self.id)
            .group(&self.group_name)
            .gap_x_1()
            .py_1()
            .px_2()
            .text_base()
            .text_color(cx.theme().foreground)
            .relative()
            .items_center()
            .justify_between()
            .refine_style(&self.style)
            .when_some(self.on_hover, |this, on_hover| {
                this.on_hover(move |hovered, window, cx| (on_hover)(hovered, window, cx))
            })
            .when(!self.disabled, |this| {
                this.group_hover(self.group_name, |this| {
                    this.bg(cx.theme().primary)
                        .text_color(cx.theme().primary_foreground)
                })
                .when(self.selected, |this| {
                    this.bg(cx.theme().primary)
                        .text_color(cx.theme().primary_foreground)
                })
                .when_some(self.on_click, |this, on_click| {
                    this.on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_click(move |event, window, cx| on_click(event, window, cx))
                })
            })
            .when(self.disabled, |this| {
                this.text_color(cx.theme().muted_foreground)
            })
            .children(self.children);
        gpui::accessible(semantic_id, rendered, move |_, window, _| {
            use gpui::accesskit::{Action, Node};
            let mut node = Node::new(role);
            node.set_label(label.to_string());
            node.set_selected(selected);
            if disabled {
                node.set_disabled()
            } else {
                node.add_action(Action::Focus);
                node.add_action(Action::Click)
            }
            let mut info = gpui::AccessibilityInfo::new(node);
            info.focused = selected && focus.as_ref().is_some_and(|focus| focus.is_focused(window));
            info
        })
        .on_action(move |request, _, window, cx| {
            if disabled {
                return;
            }
            match request.action {
                gpui::accesskit::Action::Focus => {
                    if let Some(on_focus) = &on_focus {
                        on_focus(window, cx)
                    }
                }
                gpui::accesskit::Action::Click => {
                    if let Some(on_click) = &on_click {
                        on_click(&ClickEvent::default(), window, cx)
                    }
                }
                _ => {}
            }
        })
    }
}
