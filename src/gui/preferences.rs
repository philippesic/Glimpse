use glimpse::storage::Settings;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
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

#[derive(Clone, Copy)]
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
            Self::Keymaps => serde_json::to_string_pretty(&settings.keymaps).unwrap(),
        }
    }
}

pub struct Preferences {
    draft: Settings,
    fields: Vec<Entity<InputState>>,
    page: Page,
    focus: FocusHandle,
    row_focus: Vec<FocusHandle>,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
    error: String,
    pending: bool,
}

impl EventEmitter<Event> for Preferences {}

impl Preferences {
    pub fn new(settings: Settings, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let fields = Field::ALL
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
                    input.set_cursor_blink(false, cx);
                    input
                })
            })
            .collect();
        let focus = cx.focus_handle();
        window.focus(&focus);
        let scroll = ScrollHandle::new();
        let row_focus: Vec<_> = (0..10).map(|_| cx.focus_handle()).collect();
        let mut subscriptions: Vec<_> = row_focus
            .iter()
            .enumerate()
            .map(|(index, focus)| {
                cx.on_focus_in(focus, window, move |this, _, cx| {
                    this.scroll.scroll_to_item(index);
                    cx.notify();
                })
            })
            .collect();
        let view = cx.weak_entity();
        let owner = window.window_handle();
        subscriptions.push(cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle() != owner {
                return;
            }
            let _ = view.update(cx, |this: &mut Self, cx| {
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
            draft: settings,
            fields,
            page: Page::Appearance,
            focus,
            row_focus,
            scroll,
            _subscriptions: subscriptions,
            pending: false,
        }
    }

    pub fn busy(&self) -> bool {
        self.pending
    }

    pub fn failed(&mut self, error: String, cx: &mut Context<Self>) {
        self.pending = false;
        self.error = error;
        cx.notify();
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        let result = self.candidate(cx);
        match result {
            Ok(settings) => {
                self.pending = true;
                self.error.clear();
                cx.emit(Event::Apply(Box::new(settings)));
            }
            Err(error) => self.error = error,
        }
        cx.notify();
    }

    fn candidate(&self, cx: &App) -> Result<Settings, String> {
        let value = |field: Field| self.fields[field as usize].read(cx).value().to_string();
        let number = |field, label| {
            value(field)
                .trim()
                .parse::<f32>()
                .map_err(|_| format!("{label} must be a number."))
        };
        let integer = |field, label| {
            value(field)
                .trim()
                .parse::<usize>()
                .map_err(|_| format!("{label} must be a whole number."))
        };
        let mut settings = self.draft.clone();
        settings.font_family = value(Field::Font).trim().to_owned();
        settings.font_size = number(Field::Size, "Source font size")?;
        settings.line_height = number(Field::Height, "Line height")?;
        settings.ui_scale = number(Field::Scale, "Interface scale")?;
        settings.markdown_font_size = number(Field::Markdown, "Reading font size")?;
        settings.indent = integer(Field::Indent, "Indent width")?;
        settings.search_file_mib = integer(Field::FileLimit, "Search file limit")?;
        settings.search_exclude = value(Field::Exclusions);
        settings.keymaps = serde_json::from_str(&value(Field::Keymaps))
            .map_err(|error| format!("Shortcut overrides: {error}"))?;
        for (value, lower, upper, label) in [
            (settings.font_size, 10., 32., "Source font size"),
            (settings.line_height, 1.2, 2., "Line height"),
            (settings.ui_scale, 0.8, 2., "Interface scale"),
            (settings.markdown_font_size, 12., 28., "Reading font size"),
        ] {
            if !(lower..=upper).contains(&value) {
                return Err(format!("{label} must be between {lower} and {upper}."));
            }
        }
        if !(1..=8).contains(&settings.indent) {
            return Err("Indent width must be between 1 and 8 columns.".into());
        }
        if !(1..=10).contains(&settings.search_file_mib) {
            return Err("Search file limit must be between 1 and 10 MiB.".into());
        }
        settings.warning = None;
        settings.validate()?;
        Ok(settings)
    }

    fn field(&self, label: &'static str, field: Field) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
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
            .into_any_element()
    }

    fn toggle(
        &self,
        label: &'static str,
        field: fn(&mut Settings) -> &mut bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let mut draft = self.draft.clone();
        let checked = *field(&mut draft);
        Button::new(label)
            .small()
            .compact()
            .label(format!("{}  {label}", if checked { "✓" } else { "○" }))
            .selected(checked)
            .outline()
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
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .children(
                            [
                                ("System", None),
                                ("Light", Some(false)),
                                ("Dark", Some(true)),
                            ]
                            .into_iter()
                            .map(|(label, dark)| {
                                Button::new(label)
                                    .small()
                                    .compact()
                                    .label(label)
                                    .primary()
                                    .when(
                                        if let Some(dark) = dark {
                                            self.draft.system_theme || self.draft.dark != dark
                                        } else {
                                            !self.draft.system_theme
                                        },
                                        |button| button.outline(),
                                    )
                                    .selected(if let Some(dark) = dark {
                                        !self.draft.system_theme && self.draft.dark == dark
                                    } else {
                                        self.draft.system_theme
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
                        )
                        .into_any_element(),
                );
                rows.push(self.field("Interface scale (0.8–2.0)", Field::Scale));
                rows.push(self.toggle("Compact file rows", |s| &mut s.compact, cx));
                rows.push(self.toggle("Reduce motion", |s| &mut s.reduce_motion, cx));
                rows.push(self.toggle("Remember locations", |s| &mut s.remember_locations, cx));
            }
            Page::Text => {
                rows.push(self.field("Source font family", Field::Font));
                rows.push(self.field("Source font size (10–32 px)", Field::Size));
                rows.push(self.field("Line height (1.2–2.0)", Field::Height));
                rows.push(self.field("Reading font size (12–28 px)", Field::Markdown));
                rows.push(self.toggle("Proportional plain text", |s| &mut s.proportional, cx));
            }
            Page::Editing => {
                rows.push(self.field("Indent width (1–8 columns)", Field::Indent));
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
                rows.push(self.field("Saved text limit (1–10 MiB)", Field::FileLimit));
                rows.push(self.field("Exclude patterns", Field::Exclusions));
            }
            Page::Shortcuts => {
                rows.push(div().child("Override commands with a JSON object. Cmd means Control on Linux and Command on macOS. An empty value disables a shortcut. Text editing keys remain reserved.").into_any_element());
                rows.push(self.field("Shortcut overrides (JSON)", Field::Keymaps));
                rows.push(
                    div()
                        .text_size(px(12.))
                        .children(
                            glimpse::commands::COMMANDS
                                .iter()
                                .filter(|command| !command.shortcut.is_empty())
                                .map(|command| {
                                    div().child(format!(
                                        "{}: {} ({})",
                                        command.id, command.shortcut, command.label
                                    ))
                                }),
                        )
                        .into_any_element(),
                );
            }
        }
        let categories = if self.draft.ui_scale > 1.25 || window.viewport_size().width < px(600.) {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .child(
                    Button::new("previous-category")
                        .label("‹")
                        .small()
                        .tooltip("Previous category")
                        .disabled(self.pending)
                        .on_click(cx.listener(|this, _, _, cx| {
                            let index = Page::ALL
                                .iter()
                                .position(|page| *page == this.page)
                                .unwrap();
                            this.page = Page::ALL[(index + 4) % 5];
                            this.scroll.set_offset(Point::default());
                            cx.notify();
                        })),
                )
                .child(format!(
                    "{} ({}/5)",
                    self.page.label(),
                    Page::ALL
                        .iter()
                        .position(|page| *page == self.page)
                        .unwrap()
                        + 1
                ))
                .child(
                    Button::new("next-category")
                        .label("›")
                        .small()
                        .tooltip("Next category")
                        .disabled(self.pending)
                        .on_click(cx.listener(|this, _, _, cx| {
                            let index = Page::ALL
                                .iter()
                                .position(|page| *page == this.page)
                                .unwrap();
                            this.page = Page::ALL[(index + 1) % 5];
                            this.scroll.set_offset(Point::default());
                            cx.notify();
                        })),
                )
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .children(Page::ALL.into_iter().map(|page| {
                    Button::new(page.label())
                        .label(page.label())
                        .small()
                        .primary()
                        .when(page != self.page, |button| button.outline())
                        .selected(page == self.page)
                        .disabled(self.pending)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.page = page;
                            this.scroll.set_offset(Point::default());
                            cx.notify();
                        }))
                }))
                .into_any_element()
        };
        div()
            .id("preferences-overlay")
            .absolute()
            .inset_0()
            .occlude()
            .bg(cx.theme().overlay)
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("preferences")
                    .track_focus(&self.focus)
                    .occlude()
                    .w(px(620.).min(window.viewport_size().width - px(32.)))
                    .max_h(window.viewport_size().height - px(32.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .p(px(16.))
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .text_color(cx.theme().foreground)
                    .child(
                        div()
                            .text_size(px(20. * self.draft.ui_scale))
                            .child("Preferences"),
                    )
                    .child(categories)
                    .child(
                        div()
                            .id("preference-fields")
                            .min_h_0()
                            .flex_1()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .children(rows.into_iter().enumerate().map(|(i, row)| {
                                div().track_focus(&self.row_focus[i]).pb(px(12.)).child(row)
                            })),
                    )
                    .when(!self.error.is_empty(), |this| {
                        this.child(
                            div()
                                .id("preferences-error")
                                .max_h(px(80.))
                                .overflow_y_scroll()
                                .text_color(cx.theme().danger)
                                .child(self.error.clone()),
                        )
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
                                    .on_click(cx.listener(|this, _, _, cx| this.apply(cx))),
                            ),
                    ),
            )
    }
}
