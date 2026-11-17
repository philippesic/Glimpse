use super::*;

#[derive(Clone, Copy)]
pub(super) enum Resolution {
    Reload,
    Overwrite,
}
pub(super) struct Snapshot {
    id: u64,
    generation: u64,
    local: String,
    disk: DiskFile,
    text: String,
    resolution: Resolution,
}
impl Shell {
    pub(super) fn conflict_action(
        &mut self,
        command: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_cleanup_for_active(window, cx);
        let Some(doc) = self
            .documents
            .iter()
            .find(|doc| Some(doc.id) == self.active && !doc.saving)
        else {
            return;
        };
        let Some((disk, text)) = &doc.external else {
            self.check_document(doc.id);
            self.status = "Checking the original file for changes".into();
            return;
        };
        let local = doc.input.read(cx).value().to_string();
        let detail = format!(
            "File: {}\nResolved destination: {}\nLocal text: {} bytes. Observed disk version: {} bytes. A later disk change stops this operation.",
            paths::display(&doc.disk.path),
            paths::display(&disk.target),
            local.len(),
            disk.bytes.len()
        );
        if command == "compare" {
            let observed = text.clone();
            self.ask_decision(
                DecisionAction::Compare,
                "Compare local and disk text".into(),
                detail,
                vec!["Keep Editing"],
                window,
                cx,
            );
            if let Some(view) = &self.decision {
                view.update(cx, |view, cx| view.compare(&local, &observed, window, cx));
            }
            return;
        }
        let snapshot = Snapshot {
            id: doc.id,
            generation: doc.generation,
            local,
            disk: disk.clone(),
            text: text.clone(),
            resolution: if command == "reload" {
                Resolution::Reload
            } else {
                Resolution::Overwrite
            },
        };
        let (title, detail, action) = match snapshot.resolution {
            Resolution::Reload => (
                "Reload Disk Version?",
                format!(
                    "{detail}\nThe local text will be replaced by this observed disk version. Its automatic draft will be retired after recovery storage acknowledges the operation. The local text remains in history and Undo."
                ),
                "Reload Disk Version",
            ),
            Resolution::Overwrite => (
                "Overwrite Disk Version?",
                format!(
                    "{detail}\nThis replaces only the observed disk version with the local text. Both versions must be preserved in recovery first."
                ),
                "Overwrite Disk Version",
            ),
        };
        self.ask_decision(
            DecisionAction::Conflict(Box::new(snapshot)),
            title.into(),
            detail,
            vec!["Cancel", action],
            window,
            cx,
        );
    }
    pub(super) fn resolve_conflict(
        &mut self,
        snapshot: Snapshot,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.documents.iter_mut().find(|doc| doc.id == snapshot.id) else {
            return;
        };
        if doc.saving
            || doc.generation != snapshot.generation
            || doc.input.read(cx).value().as_str() != snapshot.local
        {
            self.status = "The local text changed. Review the current versions before resolving the conflict.".into();
            return;
        }
        doc.deferred_external = None;
        doc.saving = true;
        self.checks.invalidate(doc.id);
        match snapshot.resolution {
            Resolution::Reload => self.io.send(Job::Reload(Box::new(glimpse::io::Reload {
                id: doc.id,
                generation: snapshot.generation,
                original: doc.disk.clone(),
                local: snapshot.local,
                disk: snapshot.disk,
                text: snapshot.text,
            }))),
            Resolution::Overwrite => self.io.send(Job::Save {
                id: doc.id,
                generation: snapshot.generation,
                disk: snapshot.disk,
                text: snapshot.local.into(),
                without_recovery: false,
            }),
        }
    }
    pub(super) fn reloaded(
        &mut self,
        reload: glimpse::io::Reload,
        result: Result<(), String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.documents.iter_mut().find(|doc| doc.id == reload.id) else {
            return;
        };
        doc.saving = false;
        match result {
            Err(error) => {
                doc.error = Some(error.clone());
                doc.failure_action = FailureAction::Reload;
                doc.failed_destination = None;
                if error.starts_with("Recovery unavailable:") {
                    doc.recovery_error = Some((reload.generation, error.clone()));
                }
                self.status = format!("Reload stopped; local text retained: {error}");
            }
            Ok(())
                if doc.generation != reload.generation
                    || doc.input.read(cx).value().as_str() != reload.local =>
            {
                self.status =
                    "Reload cancelled because local text changed. Newer edits are retained.".into();
            }
            Ok(()) => {
                let abandoned = doc.dirty(cx);
                let length = doc.input.read(cx).value().encode_utf16().count();
                doc.resolved_through = doc.resolved_through.max(reload.generation);
                doc.error = None;
                doc.failed_destination = None;
                doc.recovery_error = None;
                doc.disk = reload.disk;
                doc.saved = reload.text.clone().into();
                doc.external = None;
                doc.restored = false;
                doc.input.update(cx, |input, cx| {
                    input.replace_text_in_range(Some(0..length), &reload.text, window, cx)
                });
                if abandoned
                    && self.request.wait
                    && !self.finished
                    && self.requested.iter().any(|(id, _)| *id == doc.id)
                {
                    let _ = lifecycle::notify(b'1');
                    self.finished = true;
                }
                self.status = "Reloaded the observed disk version. Local text is available through Undo and history.".into();
            }
        }
        self.check_document(reload.id);
        cx.notify();
    }
}
