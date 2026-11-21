use super::{
    Draft, Header, RecoveryText, atomic_json, open_recovery, read_recovery_bytes,
    read_recovery_json, validate_record_limits,
};
use gpui_component::input::Rope;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read},
    ops::Range,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

pub(super) const MIN_SNAPSHOT_BYTES: usize = 1024 * 1024;
const MAX_EDIT_BYTES: usize = 64 * 1024;
const MAX_DELTA_BYTES: u64 = 512 * 1024;
const EDIT_VERSION: u8 = 1;

pub(super) fn is_revision(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "draft")
}

pub(super) fn sizes(path: &Path) -> io::Result<(u64, u64)> {
    let length = fs::symlink_metadata(path.join("base.json"))?.len();
    match fs::symlink_metadata(path.join("edit.json")) {
        Ok(edit) => Ok((2, length.saturating_add(edit.len()))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok((1, length)),
        Err(error) => Err(error),
    }
}

pub(super) fn header(path: &Path) -> io::Result<Header> {
    let mut header: Header = read_recovery_json(&path.join("base.json"))?;
    if let Some(edit) = current_edit(path, header.generation)? {
        header.generation = edit.generation;
    }
    Ok(header)
}

pub(super) fn generation(path: &Path) -> Result<u64, String> {
    Ok(read(path)?.generation)
}

pub(super) fn read(path: &Path) -> Result<Draft, String> {
    let base = read_recovery_bytes(&path.join("base.json")).map_err(|error| error.to_string())?;
    let mut draft: Draft = serde_json::from_slice(&base).map_err(|error| error.to_string())?;
    validate_record_limits(draft.generation, draft.text.len(), draft.base.len())?;
    if let Some(edit) = current_edit(path, draft.generation).map_err(|error| error.to_string())? {
        if <[u8; 32]>::from(Sha256::digest(&base)) != edit.base_digest {
            return Err("Recovery edit belongs to a different checkpoint".into());
        }
        drop(base);
        let Some(prefix) = draft.text.get(..edit.range.start) else {
            return Err("Recovery edit has an invalid start".into());
        };
        let Some(suffix) = draft.text.get(edit.range.end..) else {
            return Err("Recovery edit has an invalid end".into());
        };
        validate_record_limits(
            edit.generation,
            prefix.len() + edit.text.len() + suffix.len(),
            draft.base.len(),
        )?;
        draft.generation = edit.generation;
        let mut text = String::with_capacity(prefix.len() + edit.text.len() + suffix.len());
        text.push_str(prefix);
        text.push_str(&edit.text);
        text.push_str(suffix);
        draft.text = text.into();
    }
    Ok(draft)
}

#[derive(Serialize, Deserialize)]
struct Edit {
    version: u8,
    base_generation: u64,
    base_digest: [u8; 32],
    generation: u64,
    range: Range<usize>,
    text: String,
    digest: [u8; 32],
}

impl Edit {
    fn digest(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update([self.version]);
        hash.update(self.base_generation.to_le_bytes());
        hash.update(self.base_digest);
        hash.update(self.generation.to_le_bytes());
        hash.update((self.range.start as u64).to_le_bytes());
        hash.update((self.range.end as u64).to_le_bytes());
        hash.update(&self.text);
        hash.finalize().into()
    }
}

fn current_edit(path: &Path, base_generation: u64) -> io::Result<Option<Edit>> {
    let (file, len) = match open_recovery(&path.join("edit.json")) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if len > MAX_DELTA_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Recovery edit is larger than its format permits",
        ));
    }
    let mut bytes = Vec::with_capacity(len as usize);
    file.take(MAX_DELTA_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_DELTA_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Recovery edit is larger than its format permits",
        ));
    }
    let edit: Edit = serde_json::from_slice(&bytes)?;
    if edit.generation <= base_generation {
        return Ok(None);
    }
    if edit.version != EDIT_VERSION
        || edit.base_generation != base_generation
        || edit.generation > i64::MAX as u64
        || edit.range.end < edit.range.start
        || edit.text.len() > MAX_EDIT_BYTES
        || edit.digest != edit.digest()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Recovery edit has an invalid checkpoint or range",
        ));
    }
    Ok(Some(edit))
}

fn sync_directory(path: &Path) -> Result<(), String> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| error.to_string())
}

pub(super) fn full(path: &Path, draft: &Draft<impl RecoveryText>) -> Result<[u8; 32], String> {
    let base = path.join("base.json");
    let digest = atomic_json(&base, draft)?;
    sync_directory(path)?;
    Ok(digest)
}

pub(super) fn create(path: &Path, draft: &Draft<impl RecoveryText>) -> Result<[u8; 32], String> {
    let parent = path.parent().ok_or("Missing recovery session")?;
    let staged = tempfile::tempdir_in(parent).map_err(|error| error.to_string())?;
    let digest = full(staged.path(), draft)?;
    crate::fileops::rename(staged.path(), path)?;
    sync_directory(parent)?;
    Ok(digest)
}

pub(super) fn retire(path: &Path) -> Result<(), String> {
    let parent = path.parent().ok_or("Missing recovery session")?;
    let stage = tempfile::tempdir_in(parent).map_err(|error| error.to_string())?;
    let retired = stage.path().join("retired");
    crate::fileops::rename(path, &retired)?;
    if let Err(error) = sync_directory(parent) {
        if let Err(restore) = crate::fileops::rename(&retired, path) {
            let retained = stage.keep();
            return Err(format!(
                "{error}. Recovery remains at {}: {restore}",
                retained.display()
            ));
        }
        return Err(error);
    }
    stage.close().map_err(|error| error.to_string())
}

#[derive(PartialEq, Eq)]
struct CheckpointFile {
    device: u64,
    inode: u64,
    len: u64,
    changed: (i64, i64),
}

impl CheckpointFile {
    fn read(path: &Path) -> io::Result<Self> {
        let (base, len) = open_recovery(&path.join("base.json"))?;
        let metadata = base.metadata()?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            len,
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }
}

pub(super) struct Anchor {
    generation: u64,
    text: Rope,
    path: PathBuf,
    target: Option<PathBuf>,
    disk: crate::document::FileBytes,
    digest: [u8; 32],
    checkpoint: CheckpointFile,
}

impl Anchor {
    pub(super) fn capture(
        path: &Path,
        draft: &Draft<impl RecoveryText>,
        text: &Rope,
        digest: [u8; 32],
    ) -> Result<Self, String> {
        Ok(Self {
            generation: draft.generation,
            text: text.clone(),
            path: draft.path.clone(),
            target: draft.target.clone(),
            disk: draft.base.clone(),
            digest,
            checkpoint: CheckpointFile::read(path).map_err(|error| error.to_string())?,
        })
    }

    pub(super) fn generation(&self, path: &Path) -> Result<u64, String> {
        if CheckpointFile::read(path).map_err(|error| error.to_string())? != self.checkpoint {
            return Err("Recovery checkpoint changed on disk; its files were retained".into());
        }
        let edit = current_edit(path, self.generation).map_err(|error| error.to_string())?;
        if edit.as_ref().is_some_and(|edit| {
            edit.base_digest != self.digest
                || edit.range.end > self.text.len()
                || !self.text.is_char_boundary(edit.range.start)
                || !self.text.is_char_boundary(edit.range.end)
                || self.text.len() - edit.range.len() + edit.text.len()
                    > crate::document::MAX_BYTES as usize
        }) {
            return Err("Recovery edit has an invalid checkpoint or range".into());
        }
        Ok(edit.map_or(self.generation, |edit| edit.generation))
    }

    pub(super) fn write_edit(
        &self,
        path: &Path,
        draft: &Draft<impl RecoveryText>,
        text: &Rope,
    ) -> Result<bool, String> {
        if self.path != draft.path || self.target != draft.target || self.disk != draft.base {
            return Ok(false);
        }
        let Some((range, replacement)) = replacement(&self.text, text) else {
            return Ok(false);
        };
        let mut edit = Edit {
            version: EDIT_VERSION,
            base_generation: self.generation,
            base_digest: self.digest,
            generation: draft.generation,
            range,
            text: replacement,
            digest: [0; 32],
        };
        edit.digest = edit.digest();
        atomic_json(&path.join("edit.json"), &edit)?;
        sync_directory(path)?;
        crate::metrics::event(
            "recovery-edit-bytes",
            draft.generation,
            edit.text.len() as u64,
        );
        Ok(true)
    }
}

fn common_bytes<'a, 'b>(
    mut left: impl Iterator<Item = &'a str>,
    mut right: impl Iterator<Item = &'b str>,
    reverse: bool,
    limit: usize,
) -> usize {
    let mut matched = 0;
    let mut left_bytes = &[][..];
    let mut right_bytes = &[][..];
    while matched < limit {
        if left_bytes.is_empty() {
            let Some(chunk) = left.next() else { break };
            left_bytes = chunk.as_bytes();
        }
        if right_bytes.is_empty() {
            let Some(chunk) = right.next() else { break };
            right_bytes = chunk.as_bytes();
        }
        let count = left_bytes.len().min(right_bytes.len()).min(limit - matched);
        let split = |length: usize| if reverse { length - count } else { count };
        let (left_head, left_tail) = left_bytes.split_at(split(left_bytes.len()));
        let (right_head, right_tail) = right_bytes.split_at(split(right_bytes.len()));
        let (left_match, right_match);
        if reverse {
            (left_match, right_match) = (left_tail, right_tail);
            (left_bytes, right_bytes) = (left_head, right_head);
        } else {
            (left_match, right_match) = (left_head, right_head);
            (left_bytes, right_bytes) = (left_tail, right_tail);
        }
        if std::ptr::eq(left_match, right_match) || left_match == right_match {
            matched += count;
        } else {
            let matching = |(left, right): &(&u8, &u8)| left == right;
            matched += if reverse {
                left_match
                    .iter()
                    .rev()
                    .zip(right_match.iter().rev())
                    .take_while(matching)
                    .count()
            } else {
                left_match
                    .iter()
                    .zip(right_match)
                    .take_while(matching)
                    .count()
            };
            break;
        }
    }
    matched
}

fn replacement(base: &Rope, latest: &Rope) -> Option<(Range<usize>, String)> {
    let shorter = base.len().min(latest.len());
    let mut prefix = common_bytes(base.chunks(), latest.chunks(), false, shorter);
    while !base.is_char_boundary(prefix) || !latest.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let mut suffix = common_bytes(
        base.chunks_at(base.len()).0.reversed(),
        latest.chunks_at(latest.len()).0.reversed(),
        true,
        shorter - prefix,
    );
    while !base.is_char_boundary(base.len() - suffix)
        || !latest.is_char_boundary(latest.len() - suffix)
    {
        suffix -= 1;
    }
    let end = latest.len() - suffix;
    (end - prefix <= MAX_EDIT_BYTES).then(|| {
        let replacement = latest.slice(prefix..end).chunks().collect();
        (prefix..base.len() - suffix, replacement)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::DiskFile, io::TextSnapshot, storage::Recovery};

    fn draft(disk: &DiskFile, text: &Rope, generation: u64) -> Draft<TextSnapshot> {
        Draft {
            generation,
            path: disk.path.clone(),
            target: Some(disk.target.clone()),
            base: disk.bytes.clone(),
            text: TextSnapshot::Revision(text.clone()),
        }
    }

    #[test]
    fn large_recovery_reopens_exact_edits_and_moves_the_whole_draft() {
        let folder = tempfile::tempdir().unwrap();
        let original = folder.path().join("source.md");
        let source = "λ alpha\r\n🪴 βeta\n".repeat(70_000);
        fs::write(&original, format!("\u{feff}{source}")).unwrap();
        let (disk, _) = DiskFile::open(original.clone()).unwrap();
        let root = folder.path().join("drafts");
        let recovery = Recovery::new(&root).unwrap();
        let record = recovery.directory.join("1.draft");
        let mut text = Rope::from_str(&source);
        text.insert(0, "🇵🇹 ");
        recovery
            .write_revision(1, &draft(&disk, &text, 1), &text)
            .unwrap();
        let checkpoint = fs::read(record.join("base.json")).unwrap();
        text.insert(8, "🌅");
        recovery
            .write_revision(1, &draft(&disk, &text, 2), &text)
            .unwrap();
        let latest = String::from(text.clone());
        assert_eq!(fs::read(record.join("base.json")).unwrap(), checkpoint);
        assert!(sizes(&record).unwrap().1 - (checkpoint.len() as u64) < 1024);
        recovery.saved(1, 1).unwrap();
        assert!(crate::storage::inventory(&root).unwrap().records.is_empty());
        drop(recovery);
        let records = crate::storage::inventory(&root).unwrap().records;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].generation, 2);
        assert_eq!(records[0].original, original);

        let recovery = Recovery::new(&root).unwrap();
        let (restored, _) = recovery.claim(&record, 4).unwrap();
        assert_eq!(&*restored.text, latest);
        assert_eq!(restored.base, disk.bytes);
        assert_eq!(restored.generation, 2);
        assert!(!record.exists());
        let moved = recovery.directory.join("4.draft");
        let renamed = folder.path().join("renamed.md");
        fs::write(&renamed, b"\xef\xbb\xbfnew disk base\r\n").unwrap();
        let (new_target, _) = DiskFile::open(renamed.clone()).unwrap();
        recovery.rebind(4, 1, &disk, &new_target).unwrap();
        let rebound = read(&moved).unwrap();
        assert_eq!(&*rebound.text, latest);
        assert_eq!(rebound.path, renamed);
        assert_eq!(rebound.base, new_target.bytes);
        recovery.saved(4, 1).unwrap();
        assert!(moved.exists());
        recovery.saved(4, 2).unwrap();
        assert!(!moved.exists());
    }

    #[test]
    fn replacement_compacts_distant_edits_and_retires_the_previous_delta() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("source");
        let initial = "abc λ\n".repeat(190_000);
        fs::write(&path, &initial).unwrap();
        let (disk, _) = DiskFile::open(path).unwrap();
        let recovery = Recovery::new(&folder.path().join("drafts")).unwrap();
        let record = recovery.directory.join("1.draft");
        let mut text = Rope::from_str(&initial);
        recovery
            .write_revision(1, &draft(&disk, &text, 1), &text)
            .unwrap();
        text.insert(0, "β ");
        recovery
            .write_revision(1, &draft(&disk, &text, 2), &text)
            .unwrap();
        assert!(current_edit(&record, 1).unwrap().is_some());
        text.insert(text.len(), "🪴");
        recovery
            .write_revision(1, &draft(&disk, &text, 3), &text)
            .unwrap();
        assert!(current_edit(&record, 3).unwrap().is_none());
        assert_eq!(&*read(&record).unwrap().text, String::from(text.clone()));
        let stale = Rope::from_str(&initial);
        recovery
            .write_revision(1, &draft(&disk, &stale, 2), &stale)
            .unwrap();
        assert_eq!(read(&record).unwrap().generation, 3);
    }

    #[test]
    fn changed_checkpoint_and_invalid_edit_remain_intact() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("source");
        let source = "a".repeat(1024 * 1024);
        fs::write(&path, &source).unwrap();
        let (disk, _) = DiskFile::open(path).unwrap();
        let recovery = Recovery::new(&folder.path().join("drafts")).unwrap();
        let record = recovery.directory.join("1.draft");
        let mut text = Rope::from_str(&source);
        recovery
            .write_revision(1, &draft(&disk, &text, 1), &text)
            .unwrap();
        text.insert(0, "different");
        recovery
            .write_revision(1, &draft(&disk, &text, 2), &text)
            .unwrap();
        let mut edit = current_edit(&record, 1).unwrap().unwrap();
        edit.range = usize::MAX - 10..usize::MAX;
        edit.digest = edit.digest();
        let bad_edit = serde_json::to_vec(&edit).unwrap();
        fs::write(record.join("edit.json"), &bad_edit).unwrap();
        assert!(
            recovery
                .write_revision(1, &draft(&disk, &text, 3), &text)
                .is_err()
        );
        assert!(recovery.saved(1, 3).is_err());
        assert!(read(&record).is_err());
        assert_eq!(fs::read(record.join("edit.json")).unwrap(), bad_edit);
        fs::write(record.join("base.json"), b"unexpected").unwrap();
        assert!(
            recovery
                .write_revision(1, &draft(&disk, &text, 4), &text)
                .is_err()
        );
        assert_eq!(fs::read(record.join("base.json")).unwrap(), b"unexpected");
    }
}
