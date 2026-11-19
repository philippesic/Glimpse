use glimpse::paths;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    scroll::{Scrollbar, ScrollbarShow},
};
use std::{
    collections::{HashMap, HashSet},
    path::{MAIN_SEPARATOR, Path},
};
use unicode_segmentation::UnicodeSegmentation;

pub(super) struct UnsavedSubject {
    name: String,
    folder: String,
}

impl UnsavedSubject {
    fn label(&self) -> String {
        format!("{} — {}", self.name, self.folder)
    }

    fn element(
        &self,
        index: usize,
        lines: Option<usize>,
        folder: SharedString,
        numbered: bool,
        cx: &App,
    ) -> impl IntoElement {
        let mut label = self.label();
        let folder = if numbered {
            label = format!("{}. {label}", index + 1);
            SharedString::from(format!("{}. {folder}", index + 1))
        } else {
            folder
        };
        gpui::accessible(
            ("decision-subject", index),
            div()
                .min_w_0()
                .pb_1()
                .child(
                    div()
                        .font_semibold()
                        .when(lines == Some(1), |name| name.truncate())
                        .when_some(lines.filter(|&count| count > 1), |name, lines| {
                            name.line_clamp(lines)
                        })
                        .child(self.name.clone()),
                )
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .truncate()
                        .child(folder),
                ),
            move |_, _, _| {
                let mut node = gpui::accesskit::Node::new(gpui::accesskit::Role::Label);
                node.set_value(label.clone());
                node.set_read_only();
                AccessibilityInfo::new(node)
            },
        )
    }
}

pub(super) fn abbreviate_folder(
    folder: &str,
    width: Pixels,
    text_width: &impl Fn(&SharedString) -> Pixels,
) -> SharedString {
    let full = SharedString::from(folder.to_owned());
    if text_width(&full) <= width {
        return full;
    }

    let mut offsets: Vec<_> = folder
        .grapheme_indices(true)
        .map(|(offset, _)| offset)
        .collect();
    offsets.push(folder.len());
    let count = offsets.len() - 1;
    let mut abbreviated = SharedString::from("…");
    let mut shortest = 1;
    let mut longest = count.saturating_sub(1);
    while shortest <= longest {
        let kept = shortest + (longest - shortest) / 2;
        let prefix = kept * 2 / 5;
        let suffix = kept - prefix;
        let candidate = SharedString::from(format!(
            "{}…{}",
            &folder[..offsets[prefix]],
            &folder[offsets[count - suffix]..],
        ));
        if text_width(&candidate) <= width {
            abbreviated = candidate;
            shortest = kept + 1;
        } else {
            longest = kept - 1;
        }
    }
    abbreviated
}

pub struct Decision {
    title: String,
    detail: String,
    subjects: Vec<UnsavedSubject>,
    summary: Option<String>,
    primary: Option<usize>,
    primary_focus: Option<FocusHandle>,
    destructive: Option<usize>,
    actions: Vec<&'static str>,
    focus: FocusHandle,
    comparison: Option<[Entity<InputState>; 2]>,
    detail_scroll: ScrollHandle,
    _subscription: Subscription,
    _comparison_subscriptions: Vec<Subscription>,
}
pub struct Choice(pub usize);
impl EventEmitter<Choice> for Decision {}

impl Decision {
    pub fn new(
        title: String,
        detail: String,
        actions: Vec<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        let view = cx.weak_entity();
        let owner = window.window_handle();
        let subscription = cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle() != owner {
                return;
            }
            let _ = view.update(cx, |this: &mut Self, cx| {
                if super::modal::tab(&event.keystroke, &this.focus, window, cx) {
                    return;
                }
                if !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.platform
                    && !event.keystroke.modifiers.alt
                {
                    let old = this.detail_scroll.offset();
                    let page = this.detail_scroll.bounds().size.height;
                    let maximum = this.detail_scroll.max_offset().height;
                    let offset = match event.keystroke.key.as_str() {
                        "pageup" => Some((old.y + page).min(px(0.))),
                        "pagedown" => Some((old.y - page).max(-maximum)),
                        "home" => Some(px(0.)),
                        "end" => Some(-maximum),
                        _ => None,
                    };
                    if let Some(offset) = offset {
                        this.detail_scroll.set_offset(point(old.x, offset));
                        window.prevent_default();
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                }
                if event.keystroke.key == "escape" {
                    cx.emit(Choice(0));
                    window.prevent_default();
                    cx.stop_propagation();
                }
            });
        });
        Self {
            title,
            detail,
            subjects: Vec::new(),
            summary: None,
            primary: None,
            primary_focus: None,
            destructive: None,
            actions,
            focus,
            comparison: None,
            detail_scroll: ScrollHandle::new(),
            _subscription: subscription,
            _comparison_subscriptions: Vec::new(),
        }
    }
    pub fn close_actions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.summary = Some("Discard removes recovery drafts.".into());
        self.primary = Some(1);
        self.destructive = Some(2);
        let focus = cx.focus_handle();
        focus.focus(window);
        self.primary_focus = Some(focus);
    }
    pub(super) fn unsaved_subjects(paths: &[&Path]) -> Vec<UnsavedSubject> {
        paths
            .iter()
            .map(|path| UnsavedSubject {
                name: paths::name(path),
                folder: paths::parent_label(path, paths.iter().copied()),
            })
            .collect()
    }
    pub(super) fn set_subjects(&mut self, subjects: Vec<UnsavedSubject>, cx: &mut Context<Self>) {
        self.subjects = subjects;
        cx.notify();
    }
    fn folder_captions(
        subjects: &[UnsavedSubject],
        width: Pixels,
        text_width: impl Fn(&SharedString) -> Pixels,
    ) -> (Vec<SharedString>, bool) {
        let mut captions = Vec::with_capacity(subjects.len());
        let mut groups: HashMap<_, Vec<usize>> = HashMap::new();
        let difference = |index: usize, group: &[usize]| {
            let folder = &subjects[index].folder;
            let common = group
                .iter()
                .filter(|&&other| other != index)
                .map(|&other| {
                    folder
                        .graphemes(true)
                        .zip(subjects[other].folder.graphemes(true))
                        .take_while(|(a, b)| a == b)
                        .count()
                })
                .max()
                .unwrap_or_default();
            folder
                .grapheme_indices(true)
                .nth(common)
                .map(|(offset, _)| offset)
        };
        for (index, subject) in subjects.iter().enumerate() {
            let caption = abbreviate_folder(&subject.folder, width, &text_width);
            groups
                .entry((subject.name.as_str(), caption.clone()))
                .or_default()
                .push(index);
            captions.push(caption);
        }
        for group in groups.values().filter(|group| group.len() > 1) {
            for &index in group {
                let folder = &subjects[index].folder;
                if let Some(difference) = difference(index, group) {
                    let start = folder
                        .split_word_bound_indices()
                        .take_while(|(offset, _)| *offset <= difference)
                        .last()
                        .map_or(difference, |(offset, _)| offset);
                    let start = if difference - start > 12 {
                        difference
                    } else {
                        start
                    };
                    let prefix = if start == 0 { "" } else { "…" };
                    let distinguishing = format!("{prefix}{}", &folder[start..]);
                    captions[index] = abbreviate_folder(&distinguishing, width, &text_width);
                }
            }
        }
        groups.clear();
        for (index, (subject, caption)) in subjects.iter().zip(&captions).enumerate() {
            groups
                .entry((subject.name.as_str(), caption.clone()))
                .or_default()
                .push(index);
        }
        let separator = SharedString::from(" · ");
        let separator_width = text_width(&separator);
        for group in groups.values().filter(|group| group.len() > 1) {
            for &index in group {
                let folder = &subjects[index].folder;
                let Some(offset) = difference(index, group) else {
                    continue;
                };
                let Some((_, word)) = folder
                    .split_word_bound_indices()
                    .find(|(start, word)| offset < start + word.len())
                else {
                    continue;
                };
                let cue = abbreviate_folder(word, width * 0.4, &text_width);
                let context = abbreviate_folder(
                    &captions[index],
                    width - text_width(&cue) - separator_width,
                    &text_width,
                );
                let caption = SharedString::from(format!("{cue}{separator}{context}"));
                if text_width(&caption) <= width {
                    captions[index] = caption;
                }
            }
        }
        let mut seen = HashSet::new();
        let numbered = subjects
            .iter()
            .zip(&captions)
            .any(|(subject, folder)| !seen.insert((&subject.name, folder)));
        (captions, numbered)
    }
    fn close_paths(&self, numbered: bool) -> Div {
        div()
            .pt_2()
            .flex()
            .flex_col()
            .gap_2()
            .children(self.detail.lines().enumerate().map(|(index, path)| {
                let full_path = path.to_owned();
                let filename_offset = path
                    .rmatch_indices(MAIN_SEPARATOR)
                    .next()
                    .map_or(0, |(offset, separator)| offset + separator.len());
                let (folder, filename) = path.split_at(filename_offset);
                div()
                    .id(("decision-full-path", index))
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .font_semibold()
                            .whitespace_normal()
                            .child(if numbered {
                                format!("{}. {filename}", index + 1)
                            } else {
                                filename.to_owned()
                            }),
                    )
                    .child(div().w_full().whitespace_normal().child(folder.to_owned()))
                    .tooltip(move |window, cx| {
                        gpui_component::tooltip::Tooltip::new(full_path.clone()).build(window, cx)
                    })
            }))
    }
    pub fn compare(
        &mut self,
        local: &str,
        observed: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let comparison =
            [("Local text", local), ("Observed disk text", observed)].map(|(label, text)| {
                cx.new(|cx| {
                    let mut input = InputState::new(window, cx)
                        .code_editor("text")
                        .default_value(text.to_owned());
                    input.set_cursor_blink(false, cx);
                    input.set_accessibility_label(label, cx);
                    input
                })
            });
        self._comparison_subscriptions = comparison
            .iter()
            .enumerate()
            .map(|(index, input)| {
                let focus = input.read(cx).focus_handle(cx);
                cx.on_focus_in(&focus, window, move |this, window, cx| {
                    this.detail_scroll
                        .scroll_to_item(this.subjects.len() + index);
                    cx.defer_in(window, |_, _, cx| cx.notify());
                })
            })
            .collect();
        self.comparison = Some(comparison);
        cx.notify();
    }
}
impl Render for Decision {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let compact = viewport.height < px(480.);
        let panel_width = px(640.).min(viewport.width - px(32.));
        let scroll_inset = px(18.);
        let rem = window.rem_size();
        let horizontal_padding = if compact { rem } else { rem * 2. };
        let caption_size = if compact && self.comparison.is_none() {
            rem * 0.875
        } else {
            rem
        };
        let text_style = window.text_style();
        let (folder_captions, numbered) = Self::folder_captions(
            &self.subjects,
            panel_width - px(2.) - horizontal_padding - scroll_inset,
            |text| {
                let run = text_style.to_run(text.len());
                window
                    .text_system()
                    .shape_line(text.clone(), caption_size, &[run], None)
                    .width
            },
        );
        let narrow = viewport.width < px(400.) * (cx.theme().font_size / px(16.));
        let comparison_height = px(160.).min(viewport.height * 0.22);
        let detail_fraction = if self.comparison.is_some() { 0.67 } else { 0.4 };
        let subject_lines = compact.then_some(if self.subjects.len() == 1 { 3 } else { 1 });
        let detail = if !self.subjects.is_empty() {
            self.close_paths(numbered).into_any_element()
        } else {
            self.detail.clone().into_any_element()
        };
        let semantic_title = self.title.clone();
        let mut semantic_detail = self.summary.as_ref().map_or_else(
            || self.detail.clone(),
            |summary| format!("{summary}\n\n{}", self.detail),
        );
        if !self.subjects.is_empty() {
            semantic_detail = format!(
                "Unsaved documents:\n{}\n\n{semantic_detail}",
                self.subjects
                    .iter()
                    .map(UnsavedSubject::label)
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
        let rendered = div()
            .id("decision-overlay")
            .absolute()
            .inset_0()
            .occlude()
            .bg(cx.theme().overlay)
            .when(compact, |overlay| overlay.bg(cx.theme().background))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("decision")
                    .track_focus(&self.focus)
                    .occlude()
                    .w(panel_width)
                    .max_h(viewport.height - px(32.))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    .when(compact, |panel| panel.gap_2().p_2())
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .text_color(cx.theme().foreground)
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xl()
                            .when(compact, |title| {
                                title.text_base().line_height(relative(1.2))
                            })
                            .child(self.title.clone()),
                    )
                    .child(
                        div()
                            .relative()
                            .max_h(viewport.height * detail_fraction)
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .flex_shrink()
                            .overflow_hidden()
                            .child(
                                div()
                                    .id("decision-detail")
                                    .w_full()
                                    .min_h_0()
                                    .flex_shrink()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.detail_scroll)
                                    .pr(scroll_inset)
                                    .when(compact && self.comparison.is_none(), |body| {
                                        body.text_sm().line_height(relative(1.25))
                                    })
                                    .children(
                                        self.subjects.iter().zip(folder_captions).enumerate().map(
                                            |(index, (subject, folder))| {
                                                subject.element(
                                                    index,
                                                    subject_lines,
                                                    folder,
                                                    numbered,
                                                    cx,
                                                )
                                            },
                                        ),
                                    )
                                    .when_some(self.comparison.as_ref(), |body, inputs| {
                                        body.children(inputs.iter().enumerate().map(
                                            |(i, input)| {
                                                div()
                                                    .pt_3()
                                                    .child(if i == 0 {
                                                        "Local text (read only)"
                                                    } else {
                                                        "Observed disk text (read only)"
                                                    })
                                                    .child(div().h(comparison_height).child(
                                                        Input::new(input).h_full().disabled(true),
                                                    ))
                                            },
                                        ))
                                    })
                                    .child(detail),
                            )
                            .child(
                                div().absolute().inset_0().child(
                                    Scrollbar::vertical(&self.detail_scroll)
                                        .scrollbar_show(ScrollbarShow::Always),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .flex_col()
                            .gap_2()
                            .when_some(self.summary.as_ref(), |footer, summary| {
                                footer.child(
                                    div()
                                        .text_sm()
                                        .line_height(relative(1.2))
                                        .text_color(cx.theme().muted_foreground)
                                        .child(summary.clone()),
                                )
                            })
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .justify_end()
                                    .gap_2()
                                    .when(narrow, |row| row.gap_1())
                                    .children(self.actions.iter().enumerate().map(|(i, label)| {
                                        let label = if narrow
                                            && self.primary == Some(i)
                                            && matches!(
                                                *label,
                                                "Save and Close" | "Save and Finish"
                                            ) {
                                            "Save"
                                        } else {
                                            *label
                                        };
                                        Button::new(i)
                                            .label(label)
                                            .small()
                                            .when(narrow, |button| button.compact())
                                            .when(self.primary != Some(i), |button| {
                                                button.outline()
                                            })
                                            .when(self.primary == Some(i), |button| {
                                                button.primary()
                                            })
                                            .when_some(
                                                self.primary_focus
                                                    .as_ref()
                                                    .filter(|_| self.primary == Some(i)),
                                                |button, focus| {
                                                    button.with_focus_handle(focus.clone())
                                                },
                                            )
                                            .when(self.destructive == Some(i), |button| {
                                                button.danger()
                                            })
                                            .on_click(
                                                cx.listener(move |_, _, _, cx| cx.emit(Choice(i))),
                                            )
                                    })),
                            ),
                    ),
            );
        gpui::accessible("modal-semantics", rendered, move |_, _, _| {
            let mut node = gpui::accesskit::Node::new(gpui::accesskit::Role::Dialog);
            node.set_label(semantic_title.clone());
            node.set_description(semantic_detail.clone());
            let mut info = gpui::AccessibilityInfo::new(node);
            info.modal = true;
            info
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Decision, UnsavedSubject};
    use gpui::{SharedString, px};
    use unicode_segmentation::UnicodeSegmentation;

    fn captions(folders: &[&str]) -> (Vec<SharedString>, bool) {
        let subjects = folders
            .iter()
            .map(|folder| UnsavedSubject {
                name: "notes.md".into(),
                folder: (*folder).into(),
            })
            .collect::<Vec<_>>();
        Decision::folder_captions(&subjects, px(38.), |text| {
            px(text.graphemes(true).count() as f32)
        })
    }

    #[test]
    fn adjusted_groups_keep_both_distinguishing_cues() {
        let paths = [("alpha", "A"), ("alpha", "B"), ("beta", "A"), ("beta", "B")].map(
            |(group, middle)| {
                format!("{group}-long-shared-folder-middle-{middle}-the-trailing-parent-directory")
            },
        );
        let folders = paths.each_ref().map(String::as_str);
        let (labels, numbered) = captions(&folders);
        assert!(!numbered);
        for (label, (group, middle)) in
            labels
                .iter()
                .zip([("alpha", "A"), ("alpha", "B"), ("beta", "A"), ("beta", "B")])
        {
            assert!(label.starts_with(group), "{label}");
            assert!(label.contains(middle), "{label}");
            assert!(label.ends_with("directory"), "{label}");
        }
    }

    #[test]
    fn already_distinct_prefixes_and_short_paths_need_no_numbers() {
        let (labels, numbered) = captions(&["notes", "notes/archive", "other"]);
        assert_eq!(
            labels,
            ["notes", "notes/archive", "other"].map(SharedString::from)
        );
        assert!(!numbered);
    }
}
