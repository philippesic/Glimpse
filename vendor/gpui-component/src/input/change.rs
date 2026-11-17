use std::fmt::Debug;

use gpui::SharedString;

use crate::{history::HistoryItem, input::Selection};

#[derive(Debug, PartialEq, Clone)]
pub struct Change {
    pub(crate) old_range: Selection,
    pub(crate) old_text: SharedString,
    pub(crate) new_range: Selection,
    pub(crate) new_text: SharedString,
    version: usize,
}

impl Change {
    pub fn new(
        old_range: impl Into<Selection>,
        old_text: &str,
        new_range: impl Into<Selection>,
        new_text: &str,
    ) -> Self {
        Self::from_shared(
            old_range,
            SharedString::new(old_text),
            new_range,
            SharedString::new(new_text),
        )
    }

    pub(crate) fn from_shared(
        old_range: impl Into<Selection>,
        old_text: SharedString,
        new_range: impl Into<Selection>,
        new_text: SharedString,
    ) -> Self {
        Self {
            old_range: old_range.into(),
            old_text,
            new_range: new_range.into(),
            new_text,
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
}
