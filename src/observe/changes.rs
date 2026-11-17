use notify::{EventKind, event::ModifyKind};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub struct Changes {
    paths: HashSet<PathBuf>,
    entries: HashSet<PathBuf>,
    all: bool,
    pub error: Option<String>,
}
impl Changes {
    pub(super) fn record(&mut self, event: notify::Result<notify::Event>) -> bool {
        let event = match event {
            Ok(event) => event,
            Err(error) => {
                self.registered(Some(error.to_string()));
                return true;
            }
        };
        if event.need_rescan() {
            self.all = true;
        } else if matches!(event.kind, EventKind::Access(_)) {
            return false;
        } else if matches!(event.kind, EventKind::Any | EventKind::Other) || event.paths.is_empty()
        {
            self.all = true;
        }
        let entries = !matches!(
            event.kind,
            EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Metadata(_))
        );
        for path in event.paths {
            if self.all {
                break;
            }
            if entries && let Some(parent) = path.parent() {
                self.entries.insert(parent.to_owned());
            }
            self.paths.insert(path);
            self.bound();
        }
        !self.empty()
    }
    pub(super) fn registered(&mut self, error: Option<String>) {
        self.all = true;
        if self.error.is_none() {
            self.error = error;
        }
        self.bound();
    }
    pub(super) fn merge(&mut self, other: Self) {
        self.all |= other.all;
        if self.error.is_none() {
            self.error = other.error;
        }
        if !self.all {
            self.paths.extend(other.paths);
            self.entries.extend(other.entries);
        }
        self.bound();
    }
    fn bound(&mut self) {
        if self.all || self.paths.len() + self.entries.len() > 4096 {
            self.all = true;
            self.paths.clear();
            self.entries.clear();
        }
    }
    pub(super) fn empty(&self) -> bool {
        !self.all && self.paths.is_empty() && self.entries.is_empty() && self.error.is_none()
    }
    pub fn affects(&self, path: &Path) -> bool {
        self.all || self.paths.iter().any(|changed| path.starts_with(changed))
    }
    pub fn listing_changed(&self, directory: &Path) -> bool {
        self.all || self.entries.contains(directory) || self.paths.contains(directory)
    }
    #[cfg(test)]
    fn interested(&self, directory: &Path, documents: &HashSet<PathBuf>) -> bool {
        self.listing_changed(directory) || documents.iter().any(|path| self.affects(path))
    }
    pub(super) fn mapped(
        &self,
        observed_directory: &Path,
        directory: &Path,
        documents: &[(PathBuf, PathBuf)],
    ) -> Option<Self> {
        if self.all {
            return Some(Self {
                all: true,
                error: self.error.clone(),
                ..Default::default()
            });
        }
        let mut mapped = Self::default();
        if self.listing_changed(observed_directory) {
            mapped.entries.insert(directory.to_owned());
        }
        for (observed, requested) in documents {
            if self.affects(observed) {
                mapped.paths.insert(requested.clone());
            }
        }
        mapped.bound();
        (!mapped.empty()).then_some(mapped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, CreateKind, DataChange, RemoveKind, RenameMode};
    fn change(kind: EventKind, paths: &[&str]) -> Changes {
        let mut changes = Changes::default();
        changes.record(Ok(paths
            .iter()
            .fold(notify::Event::new(kind), |event, path| {
                event.add_path(PathBuf::from(path))
            })));
        changes
    }
    #[test]
    fn content_writes_only_wake_their_documents_and_structural_changes_refresh_entries() {
        let documents = HashSet::from([
            PathBuf::from("/work/source.txt"),
            PathBuf::from("/target/linked.txt"),
        ]);
        let data = EventKind::Modify(ModifyKind::Data(DataChange::Any));
        let irrelevant = change(data, &["/work/app.log"]);
        assert!(!irrelevant.interested(Path::new("/work"), &documents));
        let relevant = change(data, &["/work/source.txt", "/target/linked.txt"]);
        assert!(relevant.affects(Path::new("/work/source.txt")));
        assert!(relevant.affects(Path::new("/target/linked.txt")));
        assert!(!relevant.affects(Path::new("/work/other.txt")));
        assert!(!relevant.listing_changed(Path::new("/work")));
        let renamed = change(
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            &["/work/source.txt", "/work/new.txt"],
        );
        assert!(renamed.affects(Path::new("/work/source.txt")));
        assert!(renamed.listing_changed(Path::new("/work")));
        for kind in [
            EventKind::Create(CreateKind::File),
            EventKind::Remove(RemoveKind::File),
        ] {
            assert!(change(kind, &["/work/item"]).listing_changed(Path::new("/work")));
        }
        assert!(
            change(EventKind::Remove(RemoveKind::Folder), &["/work"])
                .affects(Path::new("/work/source.txt"))
        );
        assert!(
            !change(EventKind::Access(AccessKind::Any), &["/work/source.txt"])
                .interested(Path::new("/work"), &documents)
        );
    }
    #[test]
    fn coalescing_and_overflow_never_drop_a_needed_rescan_or_an_error() {
        let mut combined = Changes::default();
        for i in 0..5000 {
            combined.merge(change(
                EventKind::Create(CreateKind::File),
                &[&format!("/work/{i}")],
            ));
        }
        assert!(combined.paths.len() + combined.entries.len() <= 4096);
        assert!(combined.affects(Path::new("/other/source.txt")));
        assert!(combined.listing_changed(Path::new("/other")));
        combined.record(Err(notify::Error::generic("watch unavailable")));
        combined.registered(None);
        assert_eq!(combined.error.as_deref(), Some("watch unavailable"));
        for kind in [EventKind::Any, EventKind::Other] {
            assert!(change(kind, &[]).affects(Path::new("/unrelated")));
        }
        assert!(change(EventKind::Access(AccessKind::Any), &[]).empty());
        let mut flagged = Changes::default();
        flagged.record(Ok(notify::Event::new(EventKind::Access(AccessKind::Any))
            .set_flag(notify::event::Flag::Rescan)));
        assert!(flagged.affects(Path::new("/source")));
    }
}
