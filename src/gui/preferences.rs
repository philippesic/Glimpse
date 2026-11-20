use super::accessibility::message;
use glimpse::storage::Settings;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    scroll::{Scrollbar, ScrollbarShow},
};

pub enum Event {
    Apply(Box<Settings>),
    Cancel,
    Reload,
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Appearance,
    Text,
    Editing,
    Search,
    Shortcuts,
}

impl Page {
    const ALL: [Self; 5] = [
        Self::Appearance,
        Self::Text,
        Self::Editing,
        Self::Search,
        Self::Shortcuts,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Text => "Text",
            Self::Editing => "Editing",
            Self::Search => "Search",
            Self::Shortcuts => "Shortcuts",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Field {
    Font,
    Size,
    Height,
    Scale,
    Markdown,
    Indent,
    FileLimit,
    Exclusions,
    Keymaps,
}

impl Field {
    const ALL: [Self; 9] = [
        Self::Font,
        Self::Size,
        Self::Height,
        Self::Scale,
        Self::Markdown,
        Self::Indent,
        Self::FileLimit,
        Self::Exclusions,
        Self::Keymaps,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Font => "Source font family",
            Self::Size => "Source font size (10–32 px)",
            Self::Height => "Line height (1.2–2.0)",
            Self::Scale => "Interface scale (0.8–2.0)",
            Self::Markdown => "Reading font size (12–28 px)",
            Self::Indent => "Indent width (1–8 columns)",
            Self::FileLimit => "Saved text limit (1–10 MiB)",
            Self::Exclusions => "Exclude patterns",
            Self::Keymaps => "Shortcut overrides (JSON)",
        }
    }
    fn page(self) -> Page {
        match self {
            Self::Scale => Page::Appearance,
            Self::Font | Self::Size | Self::Height | Self::Markdown => Page::Text,
            Self::Indent => Page::Editing,
            Self::FileLimit | Self::Exclusions => Page::Search,
            Self::Keymaps => Page::Shortcuts,
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Font => "Leave blank to use system monospace.",
            Self::Size => "10–32 px",
            Self::Height => "1.2–2.0 times the font size",
            Self::Scale => "0.8–2.0 times the default interface size",
            Self::Markdown => "12–28 px",
            Self::Indent => "1–8 columns",
            Self::FileLimit => "1–10 MiB per saved file",
            Self::Exclusions => "One glob per line.",
            Self::Keymaps => "Use command names from the list below.",
        }
    }

    fn value(self, settings: &Settings) -> String {
        match self {
            Self::Font => settings.font_family.clone(),
            Self::Size => settings.font_size.to_string(),
            Self::Height => settings.line_height.to_string(),
            Self::Scale => settings.ui_scale.to_string(),
            Self::Markdown => settings.markdown_font_size.to_string(),
            Self::Indent => settings.indent.to_string(),
            Self::FileLimit => settings.search_file_mib.to_string(),
            Self::Exclusions => settings.search_exclude.clone(),
            Self::Keymaps => serde_json::to_string_pretty(
                &settings
                    .keymaps
                    .iter()
                    .map(|(command, key)| (command, super::platform::shortcut_label(key)))
                    .collect::<std::collections::BTreeMap<_, _>>(),
            )
            .unwrap(),
        }
    }
}

pub struct Preferences {
    draft: Settings,
    fields: Vec<Entity<InputState>>,
    page: Page,
    focus: FocusHandle,
    shortcuts_focus: FocusHandle,
    row_focus: Vec<FocusHandle>,
    theme_focus: [FocusHandle; 3],
    page_focus: [FocusHandle; 5],
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
    error: String,
    field_error: Option<(Field, String)>,
    pending: bool,
}

impl EventEmitter<Event> for Preferences {}

impl Preferences {
    pub fn new(settings: Settings, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let fields: Vec<_> = Field::ALL
            .into_iter()
            .map(|field| {
                cx.new(|cx| {
                    let mut input = InputState::new(window, cx)
                        .multi_line(matches!(field, Field::Exclusions | Field::Keymaps))
                        .default_value(field.value(&settings))
                        .placeholder(match field {
                            Field::Font => "System monospace",
                            Field::Exclusions => "One glob per line",
                            Field::Keymaps => "{\"save\": \"Cmd+S\"}",
                            _ => "",
                        });
                    input.set_accessibility_label(field.label(), cx);
                    input.set_cursor_blink(false, cx);
                    input
                })
            })
            .collect();
        let focus = cx.focus_handle();
        let shortcuts_focus = cx.focus_handle().tab_stop(true);
        window.focus(&focus);
        let scroll = ScrollHandle::new();
        let row_focus: Vec<_> = (0..10).map(|_| cx.focus_handle()).collect();
        let mut subscriptions: Vec<_> = row_focus
            .iter()
            .enumerate()
            .map(|(index, focus)| {
                cx.on_focus_in(focus, window, move |this, window, cx| {
                    this.scroll.scroll_to_item(index);
                    cx.defer_in(window, |_, _, cx| cx.notify());
                })
            })
            .collect();
        subscriptions.extend(Field::ALL.into_iter().zip(&fields).map(|(field, input)| {
            cx.subscribe(input, move |this, _, event, cx| {
                if matches!(event, InputEvent::Change)
                    && this
                        .field_error
                        .as_ref()
                        .is_some_and(|(failed, _)| *failed == field)
                {
                    this.field_error = None;
                    cx.notify();
                }
            })
        }));
        subscriptions.push(
            cx.on_focus_in(&shortcuts_focus, window, |this, window, cx| {
                this.scroll.scroll_to_top_of_item(2);
                cx.defer_in(window, |_, _, cx| cx.notify());
            }),
        );
        let view = cx.weak_entity();
        let owner = window.window_handle();
        subscriptions.push(cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle() != owner {
                return;
            }
            let _ = view.update(cx, |this: &mut Self, cx| {
                let key = &event.keystroke;
                let modifiers = &key.modifiers;
                if modifiers.alt
                    && !modifiers.control
                    && !modifiers.platform
                    && !modifiers.shift
                    && let Ok(number) = key.key.parse::<usize>()
                    && let Some(page) = number.checked_sub(1).and_then(|i| Page::ALL.get(i))
                    && !this.pending
                {
                    this.select_page(*page, cx);
                    this.focus.focus(window);
                    window.prevent_default();
                    cx.stop_propagation();
                    return;
                }
                if event
                    .context_stack
                    .iter()
                    .any(|context| context.contains("PopupMenu"))
                {
                    return;
                }
                // Categories are one Tab stop; arrow keys, Home and End choose among them.
                if let Some(index) = this
                    .page_focus
                    .iter()
                    .position(|focus| focus.is_focused(window))
                    && !modifiers.control
                    && !modifiers.platform
                    && !modifiers.alt
                    && !this.pending
                {
                    let last = Page::ALL.len() - 1;
                    let next = match key.key.as_str() {
                        "left" | "up" => Some(index.checked_sub(1).unwrap_or(last)),
                        "right" | "down" => Some(if index == last { 0 } else { index + 1 }),
                        "home" => Some(0),
                        "end" => Some(last),
                        _ => None,
                    };
                    if let Some(next) = next {
                        this.select_page(Page::ALL[next], cx);
                        this.page_focus[next].focus(window);
                        window.prevent_default();
                        cx.stop_propagation();
                        return;
                    }
                }
                if let Some(index) = this
                    .theme_focus
                    .iter()
                    .position(|focus| focus.is_focused(window))
                    && !modifiers.control
                    && !modifiers.platform
                    && !modifiers.alt
                    && !this.pending
                {
                    let next = match key.key.as_str() {
                        "left" | "up" => Some((index + 2) % 3),
                        "right" | "down" => Some((index + 1) % 3),
                        _ => None,
                    };
                    if let Some(next) = next {
                        this.draft.system_theme = next == 0;
                        if next > 0 {
                            this.draft.dark = next == 2;
                        }
                        this.theme_focus[next].focus(window);
                        window.prevent_default();
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                }
                if super::modal::tab(&event.keystroke, &this.focus, window, cx) {
                    return;
                }
                if event.keystroke.key == "escape" {
                    if !this.pending {
                        cx.emit(Event::Cancel);
                    }
                    cx.stop_propagation();
                }
            });
        }));
        Self {
            error: settings.warning.clone().unwrap_or_default(),
            field_error: None,
            draft: settings,
            fields,
            page: Page::Appearance,
            focus,
            shortcuts_focus,
            row_focus,
            theme_focus: [cx.focus_handle(), cx.focus_handle(), cx.focus_handle()],
            page_focus: std::array::from_fn(|_| cx.focus_handle()),
            scroll,
            _subscriptions: subscriptions,
            pending: false,
        }
    }

    pub fn busy(&self) -> bool {
        self.pending
    }

    fn select_page(&mut self, page: Page, cx: &mut Context<Self>) {
        if self.page != page {
            self.page = page;
            self.scroll.set_offset(Point::default());
            cx.notify();
        }
    }

    pub fn failed(&mut self, error: String, cx: &mut Context<Self>) {
        self.pending = false;
        self.error = error;
        cx.notify();
    }

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.error.clear();
        self.field_error = None;
        match self.candidate(cx) {
            Ok(settings) => {
                self.pending = true;
                cx.emit(Event::Apply(Box::new(settings)));
            }
            Err((Some(field), error)) => {
                self.field_error = Some((field, error));
                self.select_page(field.page(), cx);
                self.fields[field as usize].focus_handle(cx).focus(window);
            }
            Err((None, error)) => self.error = error,
        }
        cx.notify();
    }

    fn candidate(&self, cx: &App) -> Result<Settings, (Option<Field>, String)> {
        let value = |field: Field| self.fields[field as usize].read(cx).value().to_string();
        let number = |field: Field| {
            value(field)
                .trim()
                .parse::<f32>()
                .map_err(|_| (Some(field), "Enter a number.".into()))
        };
        let integer = |field: Field| {
            value(field)
                .trim()
                .parse::<usize>()
                .map_err(|_| (Some(field), "Enter a whole number.".into()))
        };
        let mut settings = self.draft.clone();
        settings.font_family = value(Field::Font).trim().to_owned();
        if !settings.font_family.is_empty()
            && !cx
                .text_system()
                .all_font_names()
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&settings.font_family))
        {
            return Err((
                Some(Field::Font),
                "Choose an installed font or leave blank.".into(),
            ));
        }
        settings.font_size = number(Field::Size)?;
        settings.line_height = number(Field::Height)?;
        settings.ui_scale = number(Field::Scale)?;
        settings.markdown_font_size = number(Field::Markdown)?;
        settings.indent = integer(Field::Indent)?;
        settings.search_file_mib = integer(Field::FileLimit)?;
        settings.search_exclude = value(Field::Exclusions);
        settings.keymaps = serde_json::from_str(&value(Field::Keymaps)).map_err(|error| {
            (
                Some(Field::Keymaps),
                format!(
                    "Expected a JSON object. Check line {}.",
                    error.line().max(1)
                ),
            )
        })?;
        for key in settings.keymaps.values_mut() {
            *key = super::platform::portable_shortcut(key);
        }
        for (value, lower, upper, field) in [
            (settings.font_size, 10., 32., Field::Size),
            (settings.line_height, 1.2, 2., Field::Height),
            (settings.ui_scale, 0.8, 2., Field::Scale),
            (settings.markdown_font_size, 12., 28., Field::Markdown),
        ] {
            if !(lower..=upper).contains(&value) {
                return Err((
                    Some(field),
                    format!("Enter a value from {lower} to {upper}."),
                ));
            }
        }
        if !(1..=8).contains(&settings.indent) {
            return Err((Some(Field::Indent), "Enter a value from 1 to 8.".into()));
        }
        if !(1..=10).contains(&settings.search_file_mib) {
            return Err((Some(Field::FileLimit), "Enter a value from 1 to 10.".into()));
        }
        settings.warning = None;
        settings.validate().map_err(|error| {
            if let Err(binding_error) = glimpse::commands::bindings(&settings.keymaps) {
                let message = match binding_error {
                    glimpse::commands::BindingError::Conflict { shortcut, command } => {
                        let key = super::platform::shortcut_label(&shortcut);
                        format!("{key} is already used by {command}.")
                    }
                    glimpse::commands::BindingError::Invalid(message) => message,
                };
                (Some(Field::Keymaps), message)
            } else if let Err((line, pattern_error)) = settings.validate_exclusion_patterns() {
                let reason = match pattern_error {
                    ignore::Error::Glob { err, .. } => err,
                    error => error.to_string(),
                };
                (Some(Field::Exclusions), format!("Line {line} · {reason}"))
            } else {
                (None, error)
            }
        })?;
        Ok(settings)
    }

    fn field(&self, field: Field, cx: &App) -> AnyElement {
        let error = self
            .field_error
            .as_ref()
            .filter(|(failed, _)| *failed == field);
        let hint = error.map_or_else(|| field.hint().to_owned(), |(_, error)| error.clone());
        let tooltip = hint.clone();
        let label = field
            .label()
            .split_once(" (")
            .map_or(field.label(), |(label, _)| label);
        div()
            .id(field.label())
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(label)
            .child(
                Input::new(&self.fields[field as usize])
                    .disabled(self.pending)
                    .h(px(if matches!(field, Field::Exclusions | Field::Keymaps) {
                        96.
                    } else {
                        32. * self.draft.ui_scale
                    })),
            )
            .child(super::accessibility::status(
                "preferences-field-hint",
                hint.clone(),
                div()
                    .id("field-hint")
                    .text_sm()
                    .truncate()
                    .text_color(if error.is_some() {
                        cx.theme().danger
                    } else {
                        cx.theme().muted_foreground
                    })
                    .child(hint)
                    .tooltip(move |window, cx| {
                        gpui_component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
                    }),
                error.is_some(),
            ))
            .into_any_element()
    }

    fn scroll_shortcuts(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = &event.keystroke;
        if key.modifiers.control || key.modifiers.alt || key.modifiers.platform {
            return;
        }
        let scroll = &self.scroll;
        let position = scroll.offset();
        let step = px(24. * self.draft.ui_scale);
        let page = (scroll.bounds().size.height - step).max(step);
        match key.key.as_str() {
            "home" => scroll.scroll_to_top_of_item(2),
            "end" => scroll.scroll_to_bottom(),
            key => {
                let delta = match key {
                    "up" => step,
                    "down" => -step,
                    "pageup" => page,
                    "pagedown" => -page,
                    _ => return,
                };
                scroll.set_offset(point(
                    position.x,
                    (position.y + delta).clamp(-scroll.max_offset().height, px(0.)),
                ));
            }
        }
        window.prevent_default();
        cx.stop_propagation();
        cx.notify();
    }

    fn toggle(
        &self,
        label: &'static str,
        field: fn(&mut Settings) -> &mut bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let mut draft = self.draft.clone();
        let checked = *field(&mut draft);
        Checkbox::new(label)
            .small()
            .min_h(px(24. * self.draft.ui_scale))
            .items_center()
            .label(label)
            .checked(checked)
            .animate(!self.draft.reduce_motion)
            .disabled(self.pending)
            .on_click(cx.listener(move |this, _, _, cx| {
                let value = field(&mut this.draft);
                *value = !*value;
                cx.notify();
            }))
            .into_any_element()
    }
}

impl Render for Preferences {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows = Vec::new();
        match self.page {
            Page::Appearance => {
                rows.push(
                    super::accessibility::group(
                        "theme-group",
                        "Theme".into(),
                        div().flex().flex_col().gap_1().child("Theme").child(
                            div().flex().flex_wrap().gap_2().children(
                                [
                                    ("System", None),
                                    ("Light", Some(false)),
                                    ("Dark", Some(true)),
                                ]
                                .into_iter()
                                .enumerate()
                                .map(|(index, (label, dark))| {
                                    let chosen = if let Some(dark) = dark {
                                        !self.draft.system_theme && self.draft.dark == dark
                                    } else {
                                        self.draft.system_theme
                                    };
                                    Button::new(label)
                                        .with_focus_handle(self.theme_focus[index].clone())
                                        .tab_stop(chosen)
                                        .small()
                                        .compact()
                                        .label(label)
                                        .accessibility_role(gpui::accesskit::Role::RadioButton)
                                        .accessibility_toggled(chosen)
                                        .map(|button| {
                                            if chosen {
                                                button.primary()
                                            } else {
                                                button.outline()
                                            }
                                        })
                                        .disabled(self.pending)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.draft.system_theme = dark.is_none();
                                            if let Some(dark) = dark {
                                                this.draft.dark = dark;
                                            }
                                            cx.notify();
                                        }))
                                }),
                            ),
                        ),
                    )
                    .into_any_element(),
                );
                rows.push(self.field(Field::Scale, cx));
                rows.push(self.toggle("Compact file rows", |s| &mut s.compact, cx));
                rows.push(self.toggle("Reduce motion", |s| &mut s.reduce_motion, cx));
                rows.push(self.toggle("Remember locations", |s| &mut s.remember_locations, cx));
            }
            Page::Text => {
                rows.push(self.field(Field::Font, cx));
                rows.push(self.field(Field::Size, cx));
                rows.push(self.field(Field::Height, cx));
                rows.push(self.field(Field::Markdown, cx));
                rows.push(self.toggle("Proportional plain text", |s| &mut s.proportional, cx));
            }
            Page::Editing => {
                rows.push(self.field(Field::Indent, cx));
                rows.push(self.toggle("Use tabs for indentation", |s| &mut s.tabs, cx));
                rows.push(self.toggle("Wrap long lines", |s| &mut s.wrap, cx));
                rows.push(self.toggle("Line numbers", |s| &mut s.line_numbers, cx));
                rows.push(self.toggle("Highlight current line", |s| &mut s.current_line, cx));
                rows.push(self.toggle("Automatic indentation", |s| &mut s.auto_indent, cx));
                rows.push(self.toggle("Syntax highlighting", |s| &mut s.syntax, cx));
            }
            Page::Search => {
                rows.push(self.toggle("Show hidden files", |s| &mut s.show_hidden, cx));
                rows.push(self.toggle("Search ignored files", |s| &mut s.search_ignored, cx));
                rows.push(self.toggle("Search metadata folders", |s| &mut s.search_metadata, cx));
                rows.push(self.toggle("Cross mounted volumes", |s| &mut s.search_mounts, cx));
                rows.push(self.toggle(
                    "Follow directory symlinks",
                    |s| &mut s.search_directory_links,
                    cx,
                ));
                rows.push(self.field(Field::FileLimit, cx));
                rows.push(self.field(Field::Exclusions, cx));
            }
            Page::Shortcuts => {
                let key = super::platform::shortcut_label("Cmd+Shift+O");
                let example = format!("{{\"folder\": \"{key}\"}}");
                let prompt = "Enter a JSON object that maps command names to keys.";
                let rules =
                    "Use an empty value to disable a shortcut. Text editing keys are reserved.";
                rows.push(
                    super::accessibility::label(
                        "keymap-help",
                        format!("{prompt} {example}. {rules}"),
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().child(prompt))
                            .child(div().whitespace_nowrap().child(example))
                            .child(div().child(rules)),
                    )
                    .into_any_element(),
                );
                rows.push(self.field(Field::Keymaps, cx));
                let muted = cx.theme().muted_foreground;
                let focus = self.shortcuts_focus.clone();
                let action_focus = focus.clone();
                rows.push(
                    accessible(
                        "current-shortcuts-region",
                        div()
                            .id("current-shortcuts")
                            .track_focus(&focus)
                            .border_1()
                            .rounded_md()
                            .border_color(gpui::transparent_black())
                            .focus(|style| style.border_color(cx.theme().ring))
                            .px_1()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .on_key_down(cx.listener(Self::scroll_shortcuts))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Current shortcuts"),
                            )
                            .children(glimpse::commands::COMMANDS.iter().filter_map(|command| {
                                let key = self
                                    .draft
                                    .keymaps
                                    .get(command.id)
                                    .map(String::as_str)
                                    .unwrap_or(command.shortcut);
                                (!key.is_empty()).then(|| {
                                    let shortcut = super::platform::shortcut_label(key);
                                    let label =
                                        format!("{} ({}) · {shortcut}", command.label, command.id);
                                    super::accessibility::label(
                                        command.id,
                                        label,
                                        div()
                                            .flex()
                                            .items_baseline()
                                            .gap(px(8.))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .child(command.label),
                                            )
                                            .child(
                                                div().text_color(muted).text_sm().child(command.id),
                                            )
                                            .child(
                                                div()
                                                    .w(px(150.))
                                                    .flex_shrink_0()
                                                    .text_right()
                                                    .child(shortcut),
                                            ),
                                    )
                                })
                            })),
                        move |_, window, _| {
                            let mut node = gpui::accesskit::Node::new(gpui::accesskit::Role::Group);
                            node.set_label("Current shortcuts");
                            node.add_action(gpui::accesskit::Action::Focus);
                            let mut info = AccessibilityInfo::new(node);
                            info.focused = focus.is_focused(window);
                            info
                        },
                    )
                    .on_action(move |request, _, window, _| {
                        if request.action == gpui::accesskit::Action::Focus {
                            action_focus.focus(window);
                        }
                    })
                    .into_any_element(),
                );
            }
        }
        let compact_categories =
            self.draft.ui_scale > 1.25 || window.viewport_size().width < px(600.);
        let short = window.viewport_size().height < px(480.);
        let compact_header = short && compact_categories;
        let categories = if compact_categories {
            let selected = self.page;
            let view = cx.weak_entity();
            div()
                .flex()
                .items_center()
                .child(
                    Button::new("preferences-category-menu")
                        .label(if compact_header {
                            selected.label().to_owned()
                        } else {
                            format!("Category · {}", selected.label())
                        })
                        .tooltip("Preferences category")
                        .small()
                        .outline()
                        .dropdown_caret(true)
                        .disabled(self.pending)
                        .dropdown_menu(move |mut menu, window, _| {
                            menu = menu
                                .accessible_name("Preferences category")
                                .scrollable(true)
                                .max_w(window.viewport_size().width - px(16.))
                                .max_h((window.viewport_size().height - px(40.)).max(px(100.)));
                            for (index, page) in Page::ALL.into_iter().enumerate() {
                                let view = view.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(page.label())
                                        .checked(page == selected)
                                        .shortcut_hint(super::platform::shortcut_label(&format!(
                                            "Alt+{}",
                                            index + 1
                                        )))
                                        .on_click(move |_, window, cx| {
                                            let view = view.clone();
                                            window.defer(cx, move |_, cx| {
                                                let _ = view.update(cx, |this, cx| {
                                                    this.select_page(page, cx)
                                                });
                                            });
                                        }),
                                );
                            }
                            menu
                        }),
                )
                .child(message(
                    "preferences-category",
                    selected.label().into(),
                    div().size_0(),
                ))
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .children(Page::ALL.into_iter().enumerate().map(|(index, page)| {
                    Button::new(page.label())
                        .label(page.label())
                        .small()
                        .with_focus_handle(self.page_focus[index].clone())
                        .tab_stop(page == self.page)
                        .accessibility_role(gpui::accesskit::Role::Tab)
                        .selected(page == self.page)
                        .map(|button| {
                            if page == self.page {
                                button.primary()
                            } else {
                                button.ghost()
                            }
                        })
                        .disabled(self.pending)
                        .on_click(cx.listener(move |this, _, _, cx| this.select_page(page, cx)))
                }))
                .map(|tabs| {
                    gpui::accessible("preferences-categories", tabs, |_, _, _| {
                        let mut node = gpui::accesskit::Node::new(gpui::accesskit::Role::TabList);
                        node.set_label("Preferences categories");
                        AccessibilityInfo::new(node)
                    })
                })
                .into_any_element()
        };
        let title = div()
            .flex_shrink_0()
            .text_size(px(20. * self.draft.ui_scale))
            .when(compact_header, |title| {
                title.text_size(px(16. * self.draft.ui_scale))
            })
            .child("Preferences");
        let header = div()
            .flex()
            .flex_shrink_0()
            .gap(px(if short { 8. } else { 12. }))
            .map(|header| {
                if compact_header {
                    header
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .gap_y(px(4.))
                } else {
                    header.flex_col()
                }
            })
            .child(title)
            .child(categories);
        let semantic_title = "Preferences".to_owned();
        let semantic_detail = "Appearance, text, editing, search and keyboard settings".to_owned();
        let rendered = div()
            .id("preferences-overlay")
            .absolute()
            .inset_0()
            .occlude()
            .bg(cx.theme().overlay)
            .when(short, |overlay| overlay.bg(cx.theme().background))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("preferences")
                    .track_focus(&self.focus)
                    .occlude()
                    .w(px(620.).min(window.viewport_size().width - px(32.)))
                    .h((px(560.) * self.draft.ui_scale.max(1.))
                        .min(window.viewport_size().height - px(32.)))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .p(px(16.))
                    .when(short, |panel| panel.gap(px(8.)).py(px(10.)))
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .text_color(cx.theme().foreground)
                    .child(header)
                    .child(
                        div()
                            .relative()
                            .min_h_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .when(compact_categories, |fields| {
                                fields.border_t_1().border_color(cx.theme().border).pt_1()
                            })
                            .child(super::accessibility::group(
                                "preference-fields-group",
                                "Settings".into(),
                                div()
                                    .id("preference-fields")
                                    .min_h_0()
                                    .flex_1()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.scroll)
                                    .pl(px(4.))
                                    .pt(px(4.))
                                    .pr(px(18.))
                                    .children(rows.into_iter().enumerate().map(|(i, row)| {
                                        div().track_focus(&self.row_focus[i]).pb(px(12.)).child(row)
                                    })),
                            ))
                            .child(
                                div().absolute().inset_0().child(
                                    Scrollbar::vertical(&self.scroll)
                                        .scrollbar_show(ScrollbarShow::Always),
                                ),
                            ),
                    )
                    .when(!self.error.is_empty(), |this| {
                        this.child(super::accessibility::message(
                            "preferences-error",
                            self.error.clone(),
                            div()
                                .id("preferences-validation")
                                .max_h(px(80.))
                                .overflow_y_scroll()
                                .text_color(cx.theme().danger)
                                .child(self.error.clone()),
                        ))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                Button::new("reload-preferences")
                                    .small()
                                    .compact()
                                    .outline()
                                    .label("Reload")
                                    .tooltip("Reload preferences from disk")
                                    .disabled(self.pending)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.pending = true;
                                        cx.emit(Event::Reload);
                                    })),
                            )
                            .child(div().flex_1())
                            .child(
                                Button::new("cancel-preferences")
                                    .small()
                                    .compact()
                                    .outline()
                                    .label("Cancel")
                                    .disabled(self.pending)
                                    .on_click(cx.listener(|_, _, _, cx| cx.emit(Event::Cancel))),
                            )
                            .child(
                                Button::new("apply-preferences")
                                    .small()
                                    .compact()
                                    .label(if self.pending { "Saving…" } else { "Apply" })
                                    .primary()
                                    .disabled(self.pending)
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.apply(window, cx)),
                                    ),
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
