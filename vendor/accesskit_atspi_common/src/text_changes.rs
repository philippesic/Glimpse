use accesskit::{Node, NodeId, Role, TreeId};
use accesskit_consumer::NodeRef;
use std::collections::HashMap;

fn complete_leaf_run(node: &Node) -> bool {
    node.role() == Role::TextRun
        && node.tree_id().is_none()
        && node.children().is_empty()
        && node.value().is_some_and(|value| {
            node.character_lengths()
                .iter()
                .map(|&length| usize::from(length))
                .sum::<usize>()
                == value.len()
        })
}

#[derive(Default)]
pub(crate) struct FlatText {
    children: Vec<NodeId>,
    indices: HashMap<NodeId, usize>,
    lengths: Vec<usize>,
    sums: Vec<usize>,
}
impl FlatText {
    fn add(&mut self, index: usize, old: usize, new: usize) {
        let mut next = index + 1;
        while next < self.sums.len() {
            self.sums[next] = self.sums[next] - old + new;
            next += next & next.wrapping_neg();
        }
    }
    fn prefix(&self, index: usize) -> usize {
        let mut next = index;
        let mut sum = 0;
        while next > 0 {
            sum += self.sums[next];
            next &= next - 1;
        }
        sum
    }
    fn initialize(&mut self, node: &NodeRef) -> Option<()> {
        *self = Self::default();
        let mut state = Self {
            children: node.data().children().to_vec(),
            ..Self::default()
        };
        for (index, child) in node.children().enumerate() {
            if !complete_leaf_run(child.data()) {
                return None;
            }
            state.indices.insert(state.children[index], index);
            state.lengths.push(child.data().value()?.chars().count());
        }
        state.sums = vec![0; state.lengths.len() + 1];
        for index in 0..state.lengths.len() {
            state.add(index, 0, state.lengths[index]);
        }
        *self = state;
        Some(())
    }
    pub(crate) fn position(
        &mut self,
        node: &NodeRef,
        position: accesskit::TextPosition,
    ) -> Option<usize> {
        if node.is_password_input() || node.is_graft() {
            return None;
        }
        if self.children != node.data().children() || self.children.is_empty() {
            self.initialize(node)?;
        }
        let index = *self.indices.get(&position.node)?;
        let child = node
            .tree_state
            .node_by_tree_local_id(position.node, node.locate().1)?;
        let byte = child
            .data()
            .character_lengths()
            .get(..position.character_index)?
            .iter()
            .map(|&length| usize::from(length))
            .sum::<usize>();
        Some(self.prefix(index) + child.data().value()?.get(..byte)?.chars().count())
    }
    pub(crate) fn extract(
        &mut self,
        old: &NodeRef,
        new: &NodeRef,
        tree_id: TreeId,
        updates: &[(NodeId, Node)],
    ) -> Option<(i32, String, String)> {
        if old.id() != new.id()
            || old.is_password_input()
            || new.is_password_input()
            || old.is_graft()
            || new.is_graft()
            || old.locate().1 != tree_id
            || new.locate().1 != tree_id
            || old.data().children() != new.data().children()
        {
            *self = Self::default();
            return None;
        }
        if self.children != old.data().children() || self.children.is_empty() {
            self.initialize(old)?;
        }
        let mut changed = Vec::new();
        for (id, node) in updates {
            let Some(&index) = self.indices.get(id) else {
                continue;
            };
            if !complete_leaf_run(node) {
                *self = Self::default();
                return None;
            }
            let old_child = old.tree_state.node_by_tree_local_id(*id, tree_id)?;
            if old_child.data().value() != node.value() {
                changed.push(index);
            }
        }
        changed.sort_unstable();
        changed.dedup();
        let Some(&first) = changed.first() else {
            return Some((0, String::new(), String::new()));
        };
        let last = *changed.last().unwrap();
        let base = self.prefix(first).try_into().ok()?;
        let mut before = String::new();
        let mut after = String::new();
        for &id in &self.children[first..=last] {
            before.push_str(
                old.tree_state
                    .node_by_tree_local_id(id, tree_id)?
                    .data()
                    .value()?,
            );
            after.push_str(
                new.tree_state
                    .node_by_tree_local_id(id, tree_id)?
                    .data()
                    .value()?,
            );
        }
        for index in changed {
            let id = self.children[index];
            let length = new
                .tree_state
                .node_by_tree_local_id(id, tree_id)?
                .data()
                .value()?
                .chars()
                .count();
            self.add(index, self.lengths[index], length);
            self.lengths[index] = length;
        }
        Some((base, before, after))
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Adapter, AdapterCallback, AppContext, Event, FullNodeId, InterfaceSet, ObjectEvent,
        WindowBounds,
    };
    use accesskit::{
        ActionHandler, ActionRequest, Node, NodeId, Role, TextPosition, TextSelection, TreeId,
        TreeInfo, TreeUpdate,
    };
    use std::sync::{Arc, Mutex};
    struct Capture(Arc<Mutex<Vec<ObjectEvent>>>);
    impl AdapterCallback for Capture {
        fn register_interfaces(&self, _: &Adapter, _: FullNodeId, _: InterfaceSet) {}
        fn unregister_interfaces(&self, _: &Adapter, _: FullNodeId, _: InterfaceSet) {}
        fn emit_event(&self, _: &Adapter, event: Event) {
            if let Event::Object { event, .. } = event {
                self.0.lock().unwrap().push(event);
            }
        }
    }
    struct Actions;
    impl ActionHandler for Actions {
        fn do_action(&mut self, _: ActionRequest) {}
    }
    fn run(text: &str) -> Node {
        let mut node = Node::new(Role::TextRun);
        node.set_value(text.to_owned());
        node.set_character_lengths(text.chars().map(|c| c.len_utf8() as u8).collect::<Vec<_>>());
        node
    }
    fn input(ids: &[NodeId], focus: TextPosition) -> Node {
        let mut n = Node::new(Role::MultilineTextInput);
        n.set_label("Source");
        n.set_children(ids.to_vec());
        n.set_text_selection(TextSelection {
            anchor: focus,
            focus,
        });
        n
    }
    fn update(nodes: Vec<(NodeId, Node)>) -> TreeUpdate {
        TreeUpdate {
            nodes,
            tree: None,
            tree_id: TreeId::ROOT,
            focus: NodeId(1),
        }
    }
    fn build(values: &[String]) -> (Adapter, Arc<Mutex<Vec<ObjectEvent>>>) {
        build_with_role(values, Role::MultilineTextInput)
    }
    fn build_with_role(values: &[String], role: Role) -> (Adapter, Arc<Mutex<Vec<ObjectEvent>>>) {
        let ids: Vec<_> = (0..values.len()).map(|i| NodeId(i as u64 + 10)).collect();
        let mut window = Node::new(Role::Window);
        window.set_children(vec![NodeId(1)]);
        let mut nodes = vec![
            (NodeId(0), window),
            (
                NodeId(1),
                input(
                    &ids,
                    TextPosition {
                        node: ids[0],
                        character_index: 0,
                    },
                ),
            ),
        ];
        nodes[1].1.set_role(role);
        nodes.extend(ids.iter().zip(values).map(|(&id, text)| (id, run(text))));
        let events = Arc::new(Mutex::new(Vec::new()));
        let adapter = Adapter::new(
            &AppContext::new(None),
            Capture(events.clone()),
            TreeUpdate {
                tree: Some(TreeInfo::new(NodeId(0))),
                ..update(nodes)
            },
            true,
            WindowBounds::default(),
            Actions,
        );
        events.lock().unwrap().clear();
        (adapter, events)
    }
    fn apply(text: &str, events: Vec<ObjectEvent>) -> (String, Vec<i32>) {
        let mut text: Vec<char> = text.chars().collect();
        let mut carets = Vec::new();
        for event in events {
            match event {
                ObjectEvent::TextRemoved {
                    start_index,
                    length,
                    content,
                } => {
                    let a = start_index as usize;
                    let b = a + length as usize;
                    assert_eq!(text[a..b].iter().collect::<String>(), content);
                    text.drain(a..b);
                }
                ObjectEvent::TextInserted {
                    start_index,
                    length,
                    content,
                } => {
                    assert_eq!(content.chars().count(), length as usize);
                    text.splice(start_index as usize..start_index as usize, content.chars());
                }
                ObjectEvent::CaretMoved(offset) => carets.push(offset),
                _ => {}
            }
        }
        (text.iter().collect(), carets)
    }
    #[test]
    fn changed_runs_preserve_unicode_events_and_global_caret() {
        let mut values = vec![
            "first α🙂\r\n".into(),
            "é middle\n".into(),
            "last 日本語".into(),
        ];
        let (mut adapter, events) = build(&values);
        let ids = [NodeId(10), NodeId(11), NodeId(12)];
        let mut previous = TextPosition {
            node: ids[0],
            character_index: 0,
        };
        for step in 0..300 {
            let before = values.concat();
            let index = step % 3;
            values[index] =
                ["🙂", "ά\r\n", "日本語 ", "", "café"][(step / 3) % 5].repeat(step % 11 + 1);
            let mut nodes = vec![(ids[index], run(&values[index]))];
            if step % 7 == 0 {
                let other = (index + 2) % 3;
                values[other].push('ß');
                nodes.push((ids[other], run(&values[other])));
            }
            let position = TextPosition {
                node: ids[index],
                character_index: values[index].chars().count(),
            };
            nodes.push((NodeId(1), input(&ids, position)));
            adapter.update(update(nodes));
            let (observed, carets) = apply(&before, std::mem::take(&mut *events.lock().unwrap()));
            assert_eq!(observed, values.concat(), "step {step}");
            if position != previous {
                assert_eq!(
                    carets.last(),
                    Some(
                        &(values[..=index]
                            .iter()
                            .map(|v| v.chars().count())
                            .sum::<usize>() as i32)
                    ),
                    "step {step}"
                );
            }
            previous = position;
        }
    }
    #[test]
    fn reordered_added_and_removed_runs_invalidate_cached_offsets() {
        let values = vec![
            "first 🙂\n".into(),
            "middle é\n".into(),
            "last 日本語".into(),
        ];
        let (mut adapter, events) = build(&values);
        let mut current = values.concat();
        for order in [
            vec![10, 11, 12],
            vec![12, 10, 11],
            vec![12, 10, 13, 11],
            vec![13, 12, 11],
        ] {
            let ids: Vec<_> = order.iter().copied().map(NodeId).collect();
            let texts: Vec<_> = order.iter().map(|id| format!("{id} α🙂\n")).collect();
            let position = TextPosition {
                node: ids[ids.len() - 1],
                character_index: texts.last().unwrap().chars().count(),
            };
            let mut nodes = vec![(NodeId(1), input(&ids, position))];
            nodes.extend(ids.iter().zip(&texts).map(|(&id, v)| (id, run(v))));
            adapter.update(update(nodes));
            let (observed, _) = apply(&current, std::mem::take(&mut *events.lock().unwrap()));
            assert_eq!(observed, texts.concat());
            current = observed;
        }
    }
    #[test]
    fn focused_or_active_descendant_run_still_becomes_exposed() {
        for active in [false, true] {
            let (mut adapter, events) = build(&["first".into(), "second".into()]);
            let ids = [NodeId(10), NodeId(11)];
            let position = TextPosition {
                node: ids[0],
                character_index: 0,
            };
            let mut node = input(&ids, position);
            if active {
                node.set_active_descendant(ids[0]);
            }
            let mut change = update(vec![(NodeId(1), node)]);
            if !active {
                change.focus = ids[0];
            }
            adapter.update(change);
            assert!(
                events
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|e| matches!(e, ObjectEvent::ChildAdded(..))),
                "active={active}"
            );
            events.lock().unwrap().clear();
            adapter.update(update(vec![(NodeId(1), input(&ids, position))]));
            assert!(
                events
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|e| matches!(e, ObjectEvent::ChildRemoved(..))),
                "active={active}"
            );
        }
    }
    #[test]
    fn a_nested_control_uses_the_complete_text_and_child_fallback() {
        let (mut adapter, events) = build(&["first".into(), "second".into()]);
        let ids = [NodeId(10), NodeId(11)];
        let mut container = Node::new(Role::GenericContainer);
        container.set_children(vec![NodeId(20), NodeId(21)]);
        let position = TextPosition {
            node: NodeId(20),
            character_index: 6,
        };
        adapter.update(update(vec![
            (NodeId(1), input(&ids, position)),
            (NodeId(10), container),
            (NodeId(20), run("nested")),
            (NodeId(21), Node::new(Role::Button)),
        ]));
        let captured = std::mem::take(&mut *events.lock().unwrap());
        assert!(
            captured
                .iter()
                .any(|e| matches!(e, ObjectEvent::ChildAdded(..)))
        );
        let (observed, _) = apply("firstsecond", captured);
        assert_eq!(observed, "nestedsecond");
        adapter.update(update(vec![
            (NodeId(20), run("α🙂")),
            (
                NodeId(1),
                input(
                    &ids,
                    TextPosition {
                        node: NodeId(20),
                        character_index: 2,
                    },
                ),
            ),
        ]));
        let (observed, carets) = apply(&observed, std::mem::take(&mut *events.lock().unwrap()));
        assert_eq!(observed, "α🙂second");
        assert_eq!(carets.last(), Some(&2));
    }
    #[test]
    fn character_metadata_changes_preserve_complete_range_events() {
        let (mut adapter, events) = build(&["first".into(), "second".into()]);
        let ids = [NodeId(10), NodeId(11)];
        let mut current = "firstsecond".to_owned();
        for omit in [true, false, true, false] {
            let mut first = run("first");
            if omit {
                first.set_character_lengths(Vec::<u8>::new());
            }
            adapter.update(update(vec![
                (NodeId(10), first),
                (
                    NodeId(1),
                    input(
                        &ids,
                        TextPosition {
                            node: ids[0],
                            character_index: 0,
                        },
                    ),
                ),
            ]));
            let (observed, _) = apply(&current, std::mem::take(&mut *events.lock().unwrap()));
            assert_eq!(observed, if omit { "second" } else { "firstsecond" });
            current = observed;
        }
    }

    #[test]
    fn password_changes_keep_masked_text_events() {
        let mut values = vec!["secret α".to_owned(), "last 🙂".to_owned()];
        let (mut adapter, events) = build_with_role(&values, Role::PasswordInput);
        let before = "•".repeat(values.iter().map(|s| s.chars().count()).sum());
        values[0] = "new private 🙂".into();
        let ids = [NodeId(10), NodeId(11)];
        let mut input = input(
            &ids,
            TextPosition {
                node: ids[0],
                character_index: values[0].chars().count(),
            },
        );
        input.set_role(Role::PasswordInput);
        adapter.update(update(vec![(NodeId(1), input), (ids[0], run(&values[0]))]));
        let captured = std::mem::take(&mut *events.lock().unwrap());
        for event in &captured {
            if let ObjectEvent::TextInserted { content, .. }
            | ObjectEvent::TextRemoved { content, .. } = event
            {
                assert!(content.chars().all(|c| c == '•'));
            }
        }
        let (observed, _) = apply(&before, captured);
        assert_eq!(
            observed,
            "•".repeat(values.iter().map(|s| s.chars().count()).sum())
        );
    }
}
