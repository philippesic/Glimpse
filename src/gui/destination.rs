use super::*;

impl Shell {
    pub(super) fn confirm_destination(
        &mut self,
        destination: SaveDestination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
    pub(super) fn cancel_destination(&mut self) {
        self.destination_busy = false;
        self.destination_document = None;
        self.abandon_destination = None;
        self.close_pending = None;
        self.status = "Save cancelled. The document remains open.".into();
    }
}
