use accesskit::NodeId;
use alloc::{boxed::Box, sync::Arc, vec, vec::Vec};
use hashbrown::{HashMap, HashSet, hash_map::Entry};

use crate::node::{FullNodeId, NodeState};

const PAGE_SIZE: usize = 128;

fn location(id: FullNodeId) -> (FullNodeId, u8) {
    let (local, tree) = id.to_components();
    let slot = local.0 % PAGE_SIZE as u64;
    (FullNodeId::new(NodeId(local.0 - slot), tree), slot as u8)
}

#[derive(Clone, Debug)]
enum Page {
    Sparse(Vec<(u8, NodeState)>),
    Indexed {
        nodes: Box<[Option<NodeState>]>,
        len: usize,
    },
}

impl Page {
    fn len(&self) -> usize {
        match self {
            Self::Sparse(nodes) => nodes.len(),
            Self::Indexed { len, .. } => *len,
        }
    }

    fn get(&self, slot: u8) -> Option<&NodeState> {
        match self {
            Self::Sparse(nodes) => {
                let index = nodes.binary_search_by_key(&slot, |(slot, _)| *slot).ok()?;
                Some(&nodes[index].1)
            }
            Self::Indexed { nodes, .. } => nodes[usize::from(slot)].as_ref(),
        }
    }

    fn get_mut(&mut self, slot: u8) -> Option<&mut NodeState> {
        match self {
            Self::Sparse(nodes) => {
                let index = nodes.binary_search_by_key(&slot, |(slot, _)| *slot).ok()?;
                Some(&mut nodes[index].1)
            }
            Self::Indexed { nodes, .. } => nodes[usize::from(slot)].as_mut(),
        }
    }

    fn insert(&mut self, slot: u8, data: NodeState) {
        match self {
            Self::Sparse(nodes) => {
                match nodes.binary_search_by_key(&slot, |(slot, _)| *slot) {
                    Ok(index) => nodes[index].1 = data,
                    Err(index) => nodes.insert(index, (slot, data)),
                }
                if nodes.len() > PAGE_SIZE / 2 {
                    let len = nodes.len();
                    let mut indexed: Box<[_]> =
                        core::iter::repeat_with(|| None).take(PAGE_SIZE).collect();
                    for (slot, node) in nodes.drain(..) {
                        indexed[usize::from(slot)] = Some(node);
                    }
                    *self = Self::Indexed {
                        nodes: indexed,
                        len,
                    };
                }
            }
            Self::Indexed { nodes, len } => {
                if nodes[usize::from(slot)].replace(data).is_none() {
                    *len += 1;
                }
            }
        }
    }

    fn remove(&mut self, slot: u8) -> Option<NodeState> {
        match self {
            Self::Sparse(nodes) => {
                let index = nodes.binary_search_by_key(&slot, |(slot, _)| *slot).ok()?;
                Some(nodes.remove(index).1)
            }
            Self::Indexed { nodes, len } => {
                let removed = nodes[usize::from(slot)].take()?;
                *len -= 1;
                if *len <= PAGE_SIZE / 4 {
                    *self = Self::Sparse(
                        nodes
                            .iter_mut()
                            .enumerate()
                            .filter_map(|(slot, node)| node.take().map(|node| (slot as u8, node)))
                            .collect(),
                    );
                }
                Some(removed)
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct NodePages {
    pages: HashMap<FullNodeId, Arc<Page>>,
    changed: HashSet<FullNodeId>,
}

impl NodePages {
    pub(crate) fn contains_key(&self, id: &FullNodeId) -> bool {
        self.get(id).is_some()
    }

    pub(crate) fn get(&self, id: &FullNodeId) -> Option<&NodeState> {
        let (key, slot) = location(*id);
        self.pages.get(&key)?.get(slot)
    }

    pub(crate) fn get_mut(&mut self, id: &FullNodeId) -> Option<&mut NodeState> {
        let (key, slot) = location(*id);
        let page = self.pages.get_mut(&key)?;
        page.get(slot)?;
        self.changed.insert(key);
        Arc::make_mut(page).get_mut(slot)
    }

    pub(crate) fn insert(&mut self, id: FullNodeId, data: NodeState) {
        let (key, slot) = location(id);
        self.changed.insert(key);
        match self.pages.entry(key) {
            Entry::Occupied(mut page) => Arc::make_mut(page.get_mut()).insert(slot, data),
            Entry::Vacant(page) => {
                page.insert(Arc::new(Page::Sparse(vec![(slot, data)])));
            }
        }
    }

    pub(crate) fn remove(&mut self, id: &FullNodeId) -> Option<NodeState> {
        let (key, slot) = location(*id);
        let page = self.pages.get_mut(&key)?;
        page.get(slot)?;
        self.changed.insert(key);
        let page = Arc::make_mut(page);
        let removed = page.remove(slot);
        if page.len() == 0 {
            self.pages.remove(&key);
        }
        removed
    }

    pub(crate) fn clear_changes(&mut self) {
        self.changed.clear();
    }

    pub(crate) fn share_changes_from(&mut self, source: &mut Self) {
        for key in source.changed.drain() {
            if let Some(page) = source.pages.get(&key) {
                self.pages.insert(key, page.clone());
            } else {
                self.pages.remove(&key);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.pages.values().map(|page| page.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use accesskit::{Node, Role};

    use crate::tree::TreeIndex;

    fn id(tree: u32, local: u64) -> FullNodeId {
        FullNodeId::new(NodeId(local), TreeIndex(tree))
    }

    fn button() -> NodeState {
        NodeState {
            data: Node::new(Role::Button),
            parent_and_index: None,
        }
    }

    #[test]
    fn copies_only_changed_pages_and_releases_removed_pages() {
        let mut before = NodePages::default();
        for local in 0..PAGE_SIZE as u64 {
            before.insert(id(0, local), button());
        }
        before.insert(id(1, 0), button());
        before.insert(id(0, u64::MAX), button());
        before.clear_changes();
        let mut after = before.clone();
        let dense = id(0, 0);
        let other_tree = id(1, 0);
        assert!(matches!(&*after.pages[&dense], Page::Indexed { .. }));
        assert!(Arc::ptr_eq(&before.pages[&dense], &after.pages[&dense]));

        after.get_mut(&dense).unwrap().data.set_label("Updated");
        assert_eq!(before.get(&dense).unwrap().data.label(), None);
        assert!(Arc::ptr_eq(
            &before.pages[&other_tree],
            &after.pages[&other_tree]
        ));
        for local in 1..PAGE_SIZE as u64 {
            after.remove(&id(0, local)).unwrap();
        }
        after.remove(&id(0, u64::MAX)).unwrap();
        assert!(matches!(&*after.pages[&dense], Page::Sparse(_)));
        assert_eq!(before.len(), PAGE_SIZE + 2);

        before.share_changes_from(&mut after);
        assert_eq!(before.len(), 2);
        assert_eq!(before.get(&dense).unwrap().data.label(), Some("Updated"));
        assert!(Arc::ptr_eq(&before.pages[&dense], &after.pages[&dense]));
        assert!(before.get(&id(0, u64::MAX)).is_none());
    }
}
