use super::*;

impl Shell {
    pub(super) fn confirm_destination(
        &mut self,
        destination: SaveDestination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if destination.recovery_copy && destination.expected.is_some() {
            self.destination_failed(
                &destination,
                "Recovery copies require a new filename. The existing file was not changed.".into(),
            );
            return;
        }
        let requested = self
            .requested
            .iter()
            .filter(|(id, _)| *id == destination.id)
            .map(|(_, path)| paths::display(path))
            .collect::<Vec<_>>();
        if !destination.copy
            && self.request.wait
            && !self.finished
            && !requested.is_empty()
            && !self
                .requested
                .iter()
                .any(|(id, path)| *id == destination.id && *path == destination.path)
        {
            let detail = format!(
                "Original requested destinations:\n{}\nNew destination: {}\nSuccessful Save As rebinds this document and ends the waiting CLI request with exit 1. Save Copy preserves the original request. Cancellation or a failed save keeps the request waiting.",
                requested.join("\n"),
                paths::display(&destination.path)
            );
            self.ask_decision(
                DecisionAction::AbandonDestination(Box::new(destination)),
                "Save As and abandon the original CLI destination?".into(),
                detail,
                vec!["Cancel", "Continue with Save As"],
                window,
                cx,
            );
        } else {
            self.authorize_destination(destination, window, cx);
        }
    }
    pub(super) fn authorize_destination(
        &mut self,
        destination: SaveDestination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(disk) = &destination.expected {
            let detail = format!(
                "Destination: {}\nResolved file: {}\nThis replaces the observed {}-byte disk version with {} bytes of local text. A later disk change stops the save. The observed version and local text must be preserved in recovery first.",
                paths::display(&destination.path),
                paths::display(&disk.target),
                disk.bytes.len(),
                destination.text.len()
            );
            self.ask_decision(
                DecisionAction::ReplaceDestination(Box::new(destination)),
                "Replace the observed destination?".into(),
                detail,
                vec!["Cancel", "Replace Disk Version"],
                window,
                cx,
            );
        } else {
            self.io.send(Job::SaveTo(destination));
        }
    }
    pub(super) fn destination_failed(&mut self, destination: &SaveDestination, error: String) {
        if let Some(doc) = self
            .documents
            .iter_mut()
            .find(|doc| doc.id == destination.id)
        {
            let kind = if destination.recovery_copy {
                DestinationKind::RecoveryCopy
            } else if destination.copy {
                DestinationKind::Copy
            } else {
                DestinationKind::SaveAs
            };
            doc.failure_action = FailureAction::Save;
            doc.failed_destination = Some((destination.path.clone(), kind));
            doc.error = Some(format!(
                "Could not save {}: {error}",
                paths::display(&destination.path)
            ));
            if error.starts_with("Recovery unavailable:") {
                doc.recovery_error = Some((destination.generation, error.clone()));
            }
        }
        self.cancel_destination();
        self.status = error;
    }
    pub(super) fn failure_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self
            .documents
            .iter()
            .find(|doc| Some(doc.id) == self.active)
        else {
            return;
        };
        let Some(error) = &doc.error else {
            return;
        };
        let destination = doc
            .failed_destination
            .as_ref()
            .map(|(path, _)| path)
            .unwrap_or(&doc.disk.path);
        let (title, retry) = match doc.failure_action {
            FailureAction::Save => ("Save did not complete", "Retry Save"),
            FailureAction::Reload => ("Reload did not complete", "Retry Reload"),
            FailureAction::Check => ("File unavailable", "Retry File Check"),
        };
        self.ask_decision(DecisionAction::FailureDetails(doc.id), title.into(), format!("File: {}\n{error}\nThe current text remains open. Retry checks this file again before continuing.", paths::display(destination)), vec!["Keep Editing", retry, "Save Copy Elsewhere…"], window, cx);
    }
    pub(super) fn retry_operation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.destination_busy {
            return;
        }
        self.cancel_cleanup_for_active(window, cx);
        let Some(doc) = self
            .documents
            .iter()
            .find(|doc| Some(doc.id) == self.active)
        else {
            return;
        };
        match doc.failure_action {
            FailureAction::Reload => {
                self.conflict_action("reload", window, cx);
                return;
            }
            FailureAction::Check => {
                self.check_document(doc.id);
                return;
            }
            FailureAction::Save => {}
        }
        if let Some((path, kind)) = &doc.failed_destination {
            if *kind == DestinationKind::RecoveryCopy {
                self.save_as(DestinationKind::RecoveryCopy, window, cx);
                return;
            }
            let destination = SaveDestination {
                id: doc.id,
                path: path.clone(),
                text: doc.input.read(cx).value().to_string(),
                copy: *kind != DestinationKind::SaveAs,
                bom: doc.disk.bom,
                generation: doc.generation,
                expected: None,
                original: doc.disk.clone(),
                without_recovery: false,
                recovery_copy: *kind == DestinationKind::RecoveryCopy,
            };
            self.destination_document = Some(doc.id);
            self.destination_busy = true;
            self.io.send(Job::Prepare(destination));
        } else {
            self.save(window, cx);
        }
    }
    pub(super) fn cancel_destination(&mut self) {
        self.destination_busy = false;
        self.destination_document = None;
        self.abandon_destination = None;
        self.close_pending = None;
        self.status = "Save cancelled. The document remains open.".into();
    }
}
