use super::*;
use gpui_component::Disableable;

impl Shell {
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
