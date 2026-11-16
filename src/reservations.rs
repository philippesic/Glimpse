//! Destination reservations prevent a local alias becoming dirty during replacement.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub struct Reservation {
    pub path: PathBuf,
    pub target: Option<PathBuf>,
    pub aliases: Vec<(u64, u64)>,
}
#[derive(Default)]
pub struct Reservations(BTreeMap<u64, Reservation>);
impl Reservations {
    pub fn blocks(&self, path: &Path, target: Option<&Path>) -> bool {
        self.0
            .values()
            .any(|r| r.path == path || target.is_some_and(|t| r.target.as_deref() == Some(t)))
    }
    pub fn conflicts(&self, owner: u64, path: &Path, target: Option<&Path>) -> bool {
        self.0.iter().any(|(&id, r)| {
            id != owner
                && (r.path == path || target.is_some_and(|t| r.target.as_deref() == Some(t)))
        })
    }
    pub fn reserve(&mut self, owner: u64, reservation: Reservation) -> Result<(), &'static str> {
        if self.conflicts(owner, &reservation.path, reservation.target.as_deref()) {
            return Err("Another save is using this destination. Wait for it to finish.");
        }
        if self.0.contains_key(&owner) {
            return Err("This document already has a destination operation pending.");
        }
        self.0.insert(owner, reservation);
        Ok(())
    }
    pub fn contains(&self, id: u64) -> bool {
        self.0
            .values()
            .any(|r| r.aliases.iter().any(|(alias, _)| *alias == id))
    }
    pub fn take(&mut self, owner: u64) -> Option<Reservation> {
        self.0.remove(&owner)
    }
    pub fn owners(&self) -> Vec<u64> {
        self.0.keys().copied().collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn reservation(path: &str, target: Option<&str>, id: u64) -> Reservation {
        Reservation {
            path: path.into(),
            target: target.map(Into::into),
            aliases: vec![(id, 7)],
        }
    }
    #[test]
    fn reservations_exclude_same_path_and_resolved_alias_until_release() {
        let mut reservations = Reservations::default();
        reservations
            .reserve(1, reservation("/link", Some("/target"), 2))
            .unwrap();
        assert!(reservations.contains(2));
        assert!(!reservations.conflicts(1, Path::new("/link"), Some(Path::new("/target"))));
        assert!(
            reservations
                .reserve(3, reservation("/other-link", Some("/target"), 4))
                .is_err()
        );
        assert!(
            reservations
                .reserve(3, reservation("/link", None, 4))
                .is_err()
        );
        let old = reservations.take(1).unwrap();
        assert_eq!(old.aliases, [(2, 7)]);
        assert!(!reservations.contains(2));
        reservations
            .reserve(3, reservation("/other-link", Some("/target"), 4))
            .unwrap();
    }
}
