use super::*;
use gpui_component::Disableable;

pub(super) struct Startup {
    pub(super) inventory_pending: bool,
    pub(super) paths: std::collections::VecDeque<PathBuf>,
    pub(super) current: Option<u64>,
    pub(super) restored: usize,
    pub(super) failed: usize,
}
impl Startup {
    pub(super) fn new() -> Self {
        Self {
            inventory_pending: true,
            paths: Default::default(),
            current: None,
            restored: 0,
            failed: 0,
        }
    }
}
impl Shell {
    pub(super) fn restore_next_startup(&mut self) {
        let Some(startup) = &mut self.startup_recovery else {
            return;
        };
        if let Some(path) = startup.paths.pop_front() {
            let id = self.next_id;
            self.next_id += 1;
            startup.current = Some(id);
            self.importing = true;
            self.io.send(Job::Import(id, path));
        } else {
            let startup = self.startup_recovery.take().unwrap();
            if startup.failed != 0 {
                self.warnings.push(format!("{} abandoned drafts could not be restored. Open Recovery to retry. The source records were retained.", startup.failed));
            }
            if startup.restored != 0 {
                self.status = format!(
                    "Restored {} abandoned drafts. Original files are unchanged.",
                    startup.restored
                );
            }
        }
    }

    pub(super) fn recovery_view(&self, cx: &Context<Self>) -> impl IntoElement {
        let query = self.query.read(cx).value().to_lowercase();
        div().flex().flex_col().min_h_0()
            .child(div().px_2().child(format!(
                "{} storage files use {:.2} MiB, including active sessions{}.{}",
                self.recovery_usage.files, self.recovery_usage.bytes as f64 / 1_048_576.,
                if self.recovery_usage.partial { " (incomplete scan)" } else { "" },
                if self.records_truncated { " Showing the first 256 available records." } else { "" }
            )))
            .child(div().px_2().child("Active sessions are protected. Drafts restore their original binding; version snapshots open separate copies."))
            .child(self.button("Refresh recovery", "refresh-recovery", cx))
            .child(div().id("records").max_h(px(240.)).overflow_y_scroll()
                .when(self.records.is_empty(), |list| list.child("No abandoned drafts or history records are available."))
                .children(self.records.iter().enumerate()
                    .filter(|(_, record)| paths::display(&record.original).to_lowercase().contains(&query))
                    .map(|(i, record)| {
                        let restore = record.path.clone();
                        let observed = record.path.clone();
                        let trash = record.path.clone();
                        let snapshot = record.history || record.candidate;
                        div().p_2().border_b_1().border_color(cx.theme().border)
                            .child(div().child(format!("{} · generation {} · {} bytes", if record.history { "History" } else if record.candidate { "Save candidate" } else { "Abandoned draft" }, record.generation, record.bytes)))
                            .child(div().child(paths::display(&record.original)))
                            .child(div().flex().flex_wrap().gap_2()
                                .child(Button::new(("restore-record", i)).label(if snapshot { "Restore local copy" } else { "Restore draft" }).small().outline().disabled(self.importing)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if this.importing { return; }
                                        this.importing = true;
                                        let id = this.next_id;
                                        this.next_id += 1;
                                        this.io.send(if snapshot { Job::RestoreHistory(id, restore.clone()) } else { Job::Import(id, restore.clone()) });
                                        cx.notify();
                                    })))
                                .when(snapshot, |row| row.child(Button::new(("restore-observed", i)).label("Restore observed disk copy").small().outline().disabled(self.importing)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if this.importing { return; }
                                        this.importing = true;
                                        let id = this.next_id;
                                        this.next_id += 1;
                                        this.io.send(Job::RestoreObserved(id, observed.clone()));
                                        cx.notify();
                                    }))))
                                .child(Button::new(("trash-record", i)).label("Move record to Trash").small().outline().disabled(self.importing)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.ask_record_deletion(trash.clone(), window, cx);
                                    }))))
                    })))
    }
    fn ask_record_deletion(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.ask_decision(
            DecisionAction::TrashRecord(path.clone()),
            "Move recovery record to Trash?".into(),
            format!(
                "This removes the selected recovery copy. The original file is unchanged.\n{}",
                paths::display(&path)
            ),
            vec!["Cancel", "Move record to Trash"],
            window,
            cx,
        );
    }
}

pub(super) struct UnrecoveredSave {
    id: u64,
    generation: u64,
    text: String,
    disk: DiskFile,
}

impl Shell {
    pub(super) fn recovery_failure_view(
        &self,
        doc: &Document,
        error: &str,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        div()
            .p_2()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child("Recovery unavailable")
            .child(
                self.button("Recovery Options…", "recovery-options", cx)
                    .tooltip(error.to_owned())
                    .disabled(
                        doc.saving
                            || (self.destination_busy && self.destination_document == Some(doc.id)),
                    ),
            )
    }
    pub(super) fn recovery_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self
            .documents
            .iter()
            .find(|doc| Some(doc.id) == self.active)
        else {
            return;
        };
        let Some((_, error)) = &doc.recovery_error else {
            return;
        };
        let context = doc
            .error
            .as_ref()
            .map(|error| format!("\nLast save failure: {error}"))
            .unwrap_or_default();
        let mut actions = vec!["Keep Editing", "Retry Recovery", "Save Copy Elsewhere…"];
        let blocked = doc.external.is_some()
            || doc.disk.target.as_os_str().is_empty()
            || doc.saving
            || self.destination_busy;
        if !blocked {
            actions.push("Save Without Recovery…");
        }
        self.ask_decision(DecisionAction::RecoveryOptions(doc.id), "Recovery unavailable".into(),
            format!("File: {}\n{error}{context}\nThe current text remains open. Save Copy Elsewhere requires a new filename and retains this document’s file association.{}", paths::display(&doc.disk.path), if blocked { " Save Without Recovery is unavailable for a new file, a pending save or an unresolved conflict." } else { " Save Without Recovery requires a separate confirmation for one attempt." }),
            actions, window, cx);
    }
    pub(super) fn retry_recovery(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self
            .documents
            .iter_mut()
            .find(|doc| Some(doc.id) == self.active)
        else {
            return;
        };
        doc.generation += 1;
        self.io.send(Job::Recover(
            doc.id,
            storage::Draft {
                path: doc.disk.path.clone(),
                target: Some(doc.disk.target.clone()),
                text: doc.input.read(cx).value().to_string(),
                base: doc.disk.bytes.clone(),
                generation: doc.generation,
            },
        ));
        self.status = "Retrying recovery storage".into();
    }
    pub(super) fn ask_unrecovered_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_cleanup_for_active(window, cx);
        let Some(doc) = self
            .documents
            .iter()
            .find(|doc| Some(doc.id) == self.active && !doc.saving && doc.recovery_error.is_some())
        else {
            return;
        };
        if doc.external.is_some() || doc.disk.target.as_os_str().is_empty() || self.destination_busy
        {
            self.status = "Save Without Recovery requires an existing file with no unresolved conflict. Save a separate recovery copy to a new filename.".into();
            return;
        }
        let text = doc.input.read(cx).value().to_string();
        let detail = format!(
            "File: {}\nGlimpse cannot preserve its usual recovery copies of the {}-byte local text and {}-byte observed disk version. These copies may be unavailable after this attempt. The destination is still validated before replacement. Recovery remains enabled for subsequent saves.",
            paths::display(&doc.disk.path),
            text.len(),
            doc.disk.bytes.len()
        );
        let snapshot = UnrecoveredSave {
            id: doc.id,
            generation: doc.generation,
            text,
            disk: doc.disk.clone(),
        };
        self.ask_decision(
            DecisionAction::UnrecoveredSave(Box::new(snapshot)),
            "Save once without recovery?".into(),
            detail,
            vec!["Cancel", "Save Without Recovery"],
            window,
            cx,
        );
    }
    pub(super) fn save_unrecovered(&mut self, snapshot: UnrecoveredSave, cx: &mut Context<Self>) {
        let Some(doc) = self.documents.iter_mut().find(|doc| doc.id == snapshot.id) else {
            return;
        };
        if doc.generation != snapshot.generation
            || doc.input.read(cx).value().as_str() != snapshot.text
            || doc.saving
            || doc.external.is_some()
            || doc.disk.target != snapshot.disk.target
            || doc.disk.bytes != snapshot.disk.bytes
        {
            self.status =
                "The document or disk observation changed. Review the current state before saving."
                    .into();
            return;
        }
        doc.deferred_external = None;
        doc.saving = true;
        doc.unrecovered_save = Some(snapshot.generation);
        self.checks.invalidate(doc.id);
        self.io.send(Job::Save {
            id: doc.id,
            generation: snapshot.generation,
            text: snapshot.text,
            disk: snapshot.disk,
            without_recovery: true,
        });
    }
}
