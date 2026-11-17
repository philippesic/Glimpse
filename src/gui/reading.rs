use super::*;
use gpui_component::text::TextViewResources;
use std::sync::Arc;

pub(super) struct View {
    id: u64,
    generation: u64,
    path: PathBuf,
    source: Option<SharedString>,
    resources: Arc<TextViewResources>,
    notes: String,
    error: Option<String>,
}
impl Shell {
    pub(super) fn prepare_reading(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .reading_view
            .as_ref()
            .is_some_and(|view| !self.reading || self.active != Some(view.id))
        {
            self.reading_view = None;
            self.release_view_memory(window, cx);
        }
        if !self.reading {
            return;
        }
        let Some(doc) = self
            .documents
            .iter()
            .find(|doc| Some(doc.id) == self.active)
        else {
            return;
        };
        if self.reading_view.as_ref().is_some_and(|view| {
            view.id == doc.id && view.generation == doc.generation && view.path == doc.disk.path
        }) {
            return;
        }
        self.reading_view = Some(View {
            id: doc.id,
            generation: doc.generation,
            path: doc.disk.path.clone(),
            source: None,
            resources: Arc::default(),
            notes: String::new(),
            error: None,
        });
        self.io.send(Job::ReadMarkdown(
            doc.id,
            doc.generation,
            doc.input.read(cx).value().to_string(),
            doc.disk.path.clone(),
        ));
    }
    pub(super) fn reading_ready(
        &mut self,
        id: u64,
        generation: u64,
        path: PathBuf,
        result: Result<glimpse::reading::Prepared, String>,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        let Some(view) = self
            .reading_view
            .as_mut()
            .filter(|view| view.id == id && view.generation == generation && view.path == path)
        else {
            return;
        };
        match result {
            Ok(prepared) => {
                glimpse::metrics::event("reading-image-count", id, prepared.images.len() as u64);
                glimpse::metrics::event(
                    "reading-image-bytes",
                    id,
                    prepared
                        .images
                        .values()
                        .map(|image| image.bgra.len() as u64)
                        .sum(),
                );
                let base = view.path.clone();
                view.resources = Arc::new(TextViewResources {
                    restrict_images: true,
                    images: prepared
                        .images
                        .into_iter()
                        .map(|(url, pixels)| {
                            (
                                url.into(),
                                Arc::new(RenderImage::new(vec![image::Frame::new(
                                    image::RgbaImage::from_raw(
                                        pixels.width,
                                        pixels.height,
                                        pixels.bgra,
                                    )
                                    .unwrap(),
                                )])),
                            )
                        })
                        .collect(),
                    on_link: Some(Arc::new(move |url, _, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            match glimpse::reading::resolve_link(&base, url) {
                                glimpse::reading::Link::Local(path) => {
                                    this.open(path, true, None);
                                }
                                glimpse::reading::Link::External(url) => cx.open_url(&url),
                                glimpse::reading::Link::Unsupported => {
                                    this.status =
                                        "This link type is not supported. Source text is unchanged."
                                            .into()
                                }
                            }
                            cx.notify();
                        });
                    })),
                });
                view.source = Some(prepared.source.into());
                view.notes = prepared.notes.join("\n\n");
            }
            Err(error) => view.error = Some(error),
        }
        cx.notify();
    }
    pub(super) fn reading_notices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.reading_view.as_ref().filter(|view| {
            self.documents.iter().any(|doc| {
                Some(doc.id) == self.active
                    && doc.id == view.id
                    && doc.generation == view.generation
                    && doc.disk.path == view.path
            })
        }) else {
            return;
        };
        let notes = view.notes.clone();
        if !notes.is_empty() {
            self.ask_decision(
                DecisionAction::Notice,
                "Reading image notices".into(),
                notes,
                vec!["Close"],
                window,
                cx,
            );
        }
    }

    pub(super) fn reading_element(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut area = div()
            .id("reading")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .w_full()
            .max_w(px(960.))
            .mx_auto()
            .flex()
            .flex_col()
            .p_4()
            .text_size(px(self.settings.markdown_font_size * self.settings.ui_scale));
        let Some(view) = &self.reading_view else {
            return area.child("Preparing reading view…").into_any_element();
        };
        if let Some(error) = &view.error {
            return area
                .child(error.clone())
                .child(self.button("Retry reading view", "reading-retry", cx))
                .into_any_element();
        }
        if !view.notes.is_empty() {
            let label = view.notes.clone();
            area = area.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .min_w_0()
                    .flex_shrink_0()
                    .child(
                        div().flex_1().min_w_0().child(accessible(
                            "reading-notices",
                            div()
                                .w_full()
                                .truncate()
                                .text_sm()
                                .child("Images omitted or unavailable"),
                            move |_, _, _| {
                                let mut node =
                                    gpui::accesskit::Node::new(gpui::accesskit::Role::Note);
                                node.set_label(label.clone());
                                AccessibilityInfo::new(node)
                            },
                        )),
                    )
                    .child(self.button("Image details…", "reading-notices", cx))
                    .child(self.button("Reload images", "reading-retry", cx)),
            );
        }
        if let Some(source) = &view.source {
            area = area.child(
                TextView::markdown(
                    SharedString::from(format!("markdown-{}-{}", view.id, view.generation)),
                    source.clone(),
                    window,
                    cx,
                )
                .resources(view.resources.clone(), cx)
                .focus_handle(self.reading_focus.clone(), cx)
                .selectable(true)
                .scrollable(true),
            );
        } else {
            area = area.child("Preparing reading view…")
        }
        area.into_any_element()
    }
}
