use super::*;
use gpui::accesskit::{Action, Node, Role};

pub(super) fn message(
    id: &'static str,
    label: String,
    child: impl IntoElement,
) -> impl IntoElement {
    status(id, label, child, true)
}

pub(super) fn status(
    id: &'static str,
    label: String,
    child: impl IntoElement,
    announce: bool,
) -> impl IntoElement {
    accessible(id, child, move |_, _, _| {
        let mut node = Node::new(Role::Status);
        node.set_label(label.clone());
        if announce {
            node.set_live(gpui::accesskit::Live::Polite);
        }
        AccessibilityInfo::new(node)
    })
}

/// Static text that assistive technology should read, such as an empty-state line.
pub(super) fn label(id: &'static str, text: String, child: impl IntoElement) -> impl IntoElement {
    accessible(id, child, move |_, _, _| {
        let mut node = Node::new(Role::Label);
        node.set_label(text.clone());
        node.set_value(text.clone());
        node.set_read_only();
        AccessibilityInfo::new(node)
    })
}

/// A list of options without its own focus stop, such as path completions.
pub(super) fn listbox(
    id: &'static str,
    label: String,
    child: impl IntoElement,
) -> impl IntoElement {
    accessible(id, child, move |_, _, _| {
        let mut node = Node::new(Role::ListBox);
        node.set_label(label.clone());
        AccessibilityInfo::new(node)
    })
}

/// A labelled region; also gives assistive technology the clipping bounds of a scrolled area.
pub(super) fn group(id: &'static str, label: String, child: impl IntoElement) -> impl IntoElement {
    accessible(id, child, move |_, _, _| {
        let mut node = Node::new(Role::Group);
        node.set_label(label.clone());
        AccessibilityInfo::new(node)
    })
}

/// A non-modal popup above the panes, such as Commands or path completions.
pub(super) fn popup(id: &'static str, label: String, child: impl IntoElement) -> impl IntoElement {
    accessible(id, child, move |_, _, _| {
        let mut node = Node::new(Role::Dialog);
        node.set_label(label.clone());
        AccessibilityInfo::new(node)
    })
}

pub(super) fn collection(
    id: &'static str,
    label: String,
    child: impl IntoElement,
    focus: FocusHandle,
) -> impl IntoElement {
    let action_focus = focus.clone();
    accessible(id, child, move |_, window, _| {
        let mut node = Node::new(Role::ListBox);
        node.set_label(label.clone());
        node.add_action(Action::Focus);
        let mut info = AccessibilityInfo::new(node);
        info.focused = focus.is_focused(window);
        info
    })
    .on_action(move |request, _, window, _| {
        if request.action == Action::Focus {
            action_focus.focus(window)
        }
    })
}
impl Shell {
    pub(super) fn navigation_row(
        &self,
        entry: &Entry,
        position: (usize, usize),
        selected: bool,
        child: impl IntoElement,
        cx: &Context<Self>,
    ) -> AnyElement {
        let path = entry.path.clone();
        let directory = entry.directory;
        let symlink = entry.symlink;
        let (index, total) = position;
        let focus = self.nav_focus.clone();
        let view = cx.entity().downgrade();
        let label = format!(
            "{}{}",
            paths::name(&path),
            if directory { ", folder" } else { ", file" }
        );
        let detail = format!(
            "{}{}",
            paths::display(&path),
            if symlink { " · symbolic link" } else { "" }
        );
        accessible(
            SharedString::from(format!("navigation:{}", paths::entry(&path))),
            child,
            move |_, window, _| {
                let mut node = Node::new(Role::ListBoxOption);
                node.set_label(label.clone());
                node.set_description(detail.clone());
                node.set_selected(selected);
                node.set_position_in_set(index + 1);
                node.set_size_of_set(total);
                node.add_action(Action::Focus);
                node.add_action(Action::Click);
                let mut info = AccessibilityInfo::new(node);
                info.focused = selected && focus.is_focused(window);
                info
            },
        )
        .on_action(move |request, _, window, cx| {
            let _ = view.update(cx, |this, cx| {
                this.selection = Some(path.clone());
                this.search_ui.picker_selection = Some(path.clone());
                match request.action {
                    Action::Focus => {
                        this.nav_focus.focus(window);
                        if let Some(scroll) = this.scrolls.get(&this.current) {
                            scroll.scroll_to_item(index, ScrollStrategy::Top);
                        }
                    }
                    Action::Click => {
                        this.open(path.clone(), true, None);
                    }
                    _ => return,
                }
                cx.notify();
            });
        })
        .into_any_element()
    }
}
