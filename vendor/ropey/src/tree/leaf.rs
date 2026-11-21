use std::{ops::Range, sync::Arc};

use super::{Text, TextInfo};
#[cfg(feature = "metric_chars")]
use str_indices::chars;

#[cfg(feature = "metric_utf16")]
use str_indices::utf16;

#[cfg(any(
    feature = "metric_lines_lf",
    feature = "metric_lines_lf_cr",
    feature = "metric_lines_unicode"
))]
use crate::{str_utils::lines, LineType};

#[derive(Clone, Debug)]
pub(crate) enum Leaf {
    Owned(Arc<Text>),
    Shared(Arc<SharedLeaf>),
}

#[derive(Debug)]
pub(crate) struct SharedLeaf {
    backing: Arc<str>,
    range: Range<usize>,
}

impl Leaf {
    pub fn owned(text: Text) -> Self {
        Self::Owned(Arc::new(text))
    }

    pub fn shared(backing: Arc<str>, range: Range<usize>) -> Self {
        Self::Shared(Arc::new(SharedLeaf { backing, range }))
    }

    pub fn backing(&self) -> Option<&Arc<str>> {
        match self {
            Self::Shared(shared) => Some(&shared.backing),
            Self::Owned(_) => None,
        }
    }

    pub fn text(&self) -> &str {
        match self {
            Self::Owned(text) => text.text(),
            Self::Shared(shared) => &shared.backing[shared.range.clone()],
        }
    }

    pub fn len(&self) -> usize {
        self.text().len()
    }

    pub fn text_info(&self) -> TextInfo {
        TextInfo::from_str(self.text())
    }

    pub fn is_char_boundary(&self, offset: usize) -> bool {
        match self {
            Self::Owned(text) => text.is_char_boundary(offset),
            Self::Shared(shared) => shared.backing[shared.range.clone()].is_char_boundary(offset),
        }
    }

    pub fn make_mut(&mut self) -> &mut Text {
        if matches!(self, Self::Shared(_)) {
            *self = Self::owned(Text::from_str(self.text()));
        }
        match self {
            Self::Owned(text) => Arc::make_mut(text),
            Self::Shared(_) => unreachable!(),
        }
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Owned(a), Self::Owned(b)) => Arc::ptr_eq(a, b),
            (Self::Shared(a), Self::Shared(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    //---------------------------------------------------------
    // Metric conversions.

    #[cfg(feature = "metric_chars")]
    #[inline(always)]
    pub fn byte_to_char_idx(&self, byte_idx: usize) -> usize {
        chars::from_byte_idx(self.text(), byte_idx)
    }

    #[cfg(feature = "metric_chars")]
    #[inline(always)]
    pub fn char_to_byte_idx(&self, char_idx: usize) -> usize {
        chars::to_byte_idx(self.text(), char_idx)
    }

    #[cfg(feature = "metric_utf16")]
    #[inline(always)]
    pub fn byte_to_utf16_idx(&self, byte_idx: usize) -> usize {
        utf16::from_byte_idx(self.text(), byte_idx)
    }

    #[cfg(feature = "metric_utf16")]
    #[inline(always)]
    pub fn utf16_to_byte_idx(&self, utf16_idx: usize) -> usize {
        utf16::to_byte_idx(self.text(), utf16_idx)
    }

    #[cfg(any(
        feature = "metric_lines_lf",
        feature = "metric_lines_lf_cr",
        feature = "metric_lines_unicode"
    ))]
    #[inline(always)]
    pub fn byte_to_line_idx(&self, byte_idx: usize, line_type: LineType) -> usize {
        lines::from_byte_idx(self.text(), byte_idx, line_type)
    }

    #[cfg(any(
        feature = "metric_lines_lf",
        feature = "metric_lines_lf_cr",
        feature = "metric_lines_unicode"
    ))]
    #[inline(always)]
    pub fn line_to_byte_idx(&self, line_idx: usize, line_type: LineType) -> usize {
        lines::to_byte_idx(self.text(), line_idx, line_type)
    }
}
