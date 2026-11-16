use super::*;

impl Shell {
    pub(super) fn apply_external(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.documents.iter_mut().find(|doc| doc.id == id) else {
            return;
        };
        if doc.deferred_external.is_none() || doc.saving {
            return;
        }
        let input = doc.input.read(cx);
        if input.interaction_active()
            || (window.modifiers().shift && input.focus_handle(cx).is_focused(window))
        {
            let message = "External change waiting for selection or composition to finish";
            if self.status != message {
                self.status = message.into();
                glimpse::metrics::event("external-wait", id, doc.generation);
                cx.notify();
            }
            return;
        }
        if doc.dirty(cx) {
            doc.external = doc.deferred_external.take();
            self.status = "Changed on disk. Local edits retained. Reload or Save Copy.".into();
            cx.notify();
            return;
        }
        let (disk, text) = doc.deferred_external.take().unwrap();
        let (old_range, new_range) = glimpse::editor::difference(&doc.saved, &text);
        let outcome = doc.input.update(cx, |input, cx| {
            input.replace_text_preserving_selection(old_range, &text[new_range], window, cx)
        });
        if let Some(relocated) = outcome {
            glimpse::metrics::event("external-applied", id, doc.generation);
            doc.passive_refresh = true;
            doc.saved = text.into();
            doc.disk = disk;
            doc.external = None;
            self.status = if relocated {
                "Reloaded external change; selection moved from changed text. Undo is available."
            } else {
                "Reloaded external change; editing position retained. Undo is available."
            }
            .into();
        } else {
            doc.deferred_external = Some((disk, text));
        }
        cx.notify();
    }
}
