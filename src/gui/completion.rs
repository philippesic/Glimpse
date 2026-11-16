use super::*;

impl Shell {
    pub(super) fn finish_requested(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.request.wait || self.finished {
            self.status = "No CLI editing request is waiting".into();
            return;
        }
        if self.io.pending_mutations() > 0
            || self.destination_busy
            || !self.closings.is_empty()
            || self.finishing.pending()
        {
            self.status = "Wait for pending writes or recovery cleanup".into();
            return;
        }
        let documents = self
            .requested
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| !self.completed.contains(id))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        self.close_pending = Some(CloseDecision {
            quit: false,
            documents,
            saving: false,
            finish: true,
            validating: false,
            discard: false,
        });
        self.continue_close(window, cx);
    }
    pub(super) fn validate_before_cleanup(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(decision) = &self.close_pending else {
            return false;
        };
        if !self.request.wait || self.finished {
            return false;
        }
        if decision.validating {
            return true;
        }
        let abandons = decision.discard
            && self.documents.iter().any(|doc| {
                decision.documents.contains(&doc.id)
                    && doc.dirty(cx)
                    && self.requested.iter().any(|(id, _)| *id == doc.id)
            });
        if abandons {
            return false;
        }
        let mut paths: std::collections::BTreeMap<u64, Vec<PathBuf>> =
            std::collections::BTreeMap::new();
        for (id, path) in &self.requested {
            if decision.documents.contains(id)
                && !self.completed.contains(id)
                && !self.validated.contains(id)
            {
                paths.entry(*id).or_default().push(path.clone());
            }
        }
        if paths.is_empty() {
            return false;
        }
        let snapshots: Vec<_> = paths
            .keys()
            .filter_map(|id| {
                self.documents
                    .iter()
                    .find(|doc| doc.id == *id)
                    .map(|doc| (doc.id, doc.generation))
            })
            .collect();
        if snapshots.len() != paths.len() {
            self.close_pending = None;
            self.status =
                "A requested document is unavailable. The CLI request remains waiting.".into();
            return true;
        }
        self.validation_disks.clear();
        let attempt = self.finishing.begin(snapshots);
        self.close_pending.as_mut().unwrap().validating = true;
        for (id, aliases) in paths {
            let doc = self.documents.iter().find(|doc| doc.id == id).unwrap();
            self.io.send(Job::Finish(
                id,
                doc.generation,
                attempt,
                doc.disk.clone(),
                aliases,
            ));
        }
        self.status = "Validating requested destinations".into();
        true
    }
    pub(super) fn finished_validation(
        &mut self,
        (id, generation, attempt, disk): (u64, u64, u64, DiskFile),
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        if !self.finishing.accepts(attempt, id, generation) {
            return;
        }
        self.validation_disks.insert(id, disk);
        glimpse::metrics::event("finish-reply", id, generation);
        match self.finishing.acknowledge(attempt, id, generation, result) {
            Ok(Some(snapshots)) => {
                let valid = snapshots.iter().all(|(id, generation)| {
                    self.documents.iter().any(|doc| {
                        doc.id == *id
                            && doc.generation == *generation
                            && !doc.dirty(cx)
                            && !doc.saving
                            && self.validation_disks.get(id).is_some_and(|disk| {
                                disk.target == doc.disk.target && disk.bytes == doc.disk.bytes
                            })
                    })
                });
                self.validation_disks.clear();
                if !valid {
                    self.close_pending = None;
                    self.status =
                        "Documents changed during validation. The request remains waiting.".into();
                    return;
                }
                self.validated
                    .extend(snapshots.into_iter().map(|(id, _)| id));
                if let Some(decision) = self.close_pending.as_mut() {
                    decision.validating = false;
                    let discard = decision.discard;
                    self.commit_close(discard, cx);
                }
            }
            Ok(None) => {}
            Err(error) => {
                self.validation_disks.clear();
                self.close_pending = None;
                self.status = format!("Completion stopped: {error}");
            }
        }
    }
    pub(super) fn complete_request(&mut self) {
        if self.request.wait
            && !self.finished
            && !self.requested.is_empty()
            && self
                .requested
                .iter()
                .all(|(id, _)| self.completed.contains(id))
        {
            self.finished = true;
            let _ = lifecycle::notify(b'0');
            self.status = "CLI request finished".into();
        }
    }
}
