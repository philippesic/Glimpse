use std::fmt::Debug;

use gpui::SharedString;
use ropey::Rope;

use crate::{history::HistoryItem, input::Selection};

use super::text_metadata::LineEndings;

#[derive(Debug, PartialEq, Clone)]
pub(super) struct Revision {
    pub(super) text: Rope,
    pub(super) endings: LineEndings,
}

#[derive(Debug, PartialEq, Clone)]
pub(super) enum HistoryText {
    Fragment(SharedString),
    Revision(Revision),
}

impl HistoryText {
    fn len(&self) -> usize {
        match self {
            Self::Fragment(text) => text.len(),
            Self::Revision(revision) => revision.text.len(),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct Change {
    pub(crate) old_range: Selection,
    pub(super) old_text: HistoryText,
    pub(crate) new_range: Selection,
    pub(super) new_text: HistoryText,
    version: usize,
}

impl Change {
    pub fn new(
        old_range: impl Into<Selection>,
        old_text: &str,
        new_range: impl Into<Selection>,
        new_text: &str,
    ) -> Self {
        Self {
            old_range: old_range.into(),
            old_text: HistoryText::Fragment(SharedString::new(old_text)),
            new_range: new_range.into(),
            new_text: HistoryText::Fragment(SharedString::new(new_text)),
            version: 0,
        }
    }

    pub(super) fn from_revisions(before: Revision, after: Revision) -> Self {
        Self {
            old_range: (0..before.text.len()).into(),
            old_text: HistoryText::Revision(before),
            new_range: (0..after.text.len()).into(),
            new_text: HistoryText::Revision(after),
            version: 0,
        }
    }
}

impl HistoryItem for Change {
    fn version(&self) -> usize {
        self.version
    }

    fn set_version(&mut self, version: usize) {
        self.version = version;
    }

    fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.old_text.len() + self.new_text.len()
    }
}
