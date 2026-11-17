use super::*;
use crate::appearance;
use egui::RichText;

// Resample precise 60 Hz wheel updates across intervening display frames.
#[derive(Clone, Default)]
struct TrackpadCadence {
    pending: f32,
    time: Option<f64>,
}
impl TrackpadCadence {
    fn step(&mut self, delta: f32, time: f64, interpolate: bool) -> f32 {
        if self.time == Some(time) {
            return delta;
        }
        let dt = self
            .time
            .replace(time)
            .map_or(1. / 120., |previous| (time - previous) as f32);
        self.pending += delta;
        if !interpolate || dt > 0.05 || self.pending.abs() < 0.05 {
            return std::mem::take(&mut self.pending);
        }
        // Drain 90% within one 60 Hz input interval, without a long smoothing tail.
        let fraction = 1. - (-dt.max(0.) / 0.00724).exp();
        let movement = self.pending * fraction;
        self.pending -= movement;
        movement
    }
}

impl Navigator {
    fn keyboard(&mut self, ui: &mut egui::Ui, action: &mut NavigationAction) {
        let ctx = ui.ctx();
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)) {
            self.up();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)) {
            self.forward();
        }
        let down = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
        let up = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
        if down || up {
            self.resume();
            let folder = self.current.clone();
            let rows = self.cached_rows(&folder);
            let entries: Vec<_> = rows.iter().filter(|row| row.message.is_none()).collect();
            {
                if entries.is_empty() {
                    return;
                }
                let index = self
                    .selections
                    .get(&self.current)
                    .and_then(|selected| entries.iter().position(|e| &e.path == selected));
                let index = match index {
                    Some(index) if down => (index + 1).min(entries.len() - 1),
                    Some(index) => index.saturating_sub(1),
                    None if down => 0,
                    None => entries.len() - 1,
                };
                self.selections
                    .insert(self.current.clone(), entries[index].path.clone());
                self.reveal_selection = true;
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
            && let Some(selected) = self.selections.get(&self.current).cloned()
            && let Some(Listing::Ready(entries)) = self.folders.get(&self.current)
            && let Some(entry) = entries.iter().find(|e| e.path == selected)
        {
            if entry.directory {
                self.navigate(selected);
            } else {
                self.select_file(&selected);
                action.open = Some((selected, true));
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            action.edit = true;
        }
    }

    fn list(
        &mut self,
        ui: &mut egui::Ui,
        folder: &Path,
        enabled: bool,
        entering: bool,
    ) -> Option<Row> {
        if let Some(notice) = self.notices.get(folder) {
            ui.weak(notice);
        }
        let rows = self.cached_rows(folder);
        if rows.is_empty() {
            ui.weak("No matching items");
        }
        let mut clicked = None;
        let selected = self.selections.get(folder);
        let view_key = (folder.to_path_buf(), false);
        let mut scroll = egui::ScrollArea::vertical();
        if entering {
            // Restore position once; forcing the clamped offset every frame cancels rubber-banding.
            let id = ui.make_persistent_id(egui::IdSalt::new(("folder_contents", folder)));
            egui::scroll_area::State::default().store(ui.ctx(), id);
            scroll =
                scroll.vertical_scroll_offset(self.vertical.get(&view_key).copied().unwrap_or(0.));
        }
        if self.reveal_selection
            && folder == self.current
            && let Some(index) = rows.iter().position(|row| Some(&row.path) == selected)
        {
            let top = index as f32 * self.row_height;
            let offset = self.vertical.get(&view_key).copied().unwrap_or(0.);
            let bottom = top + self.row_height;
            let visible_height = ui.available_height();
            scroll = scroll.vertical_scroll_offset(if top < offset {
                top
            } else if bottom > offset + visible_height {
                bottom - visible_height
            } else {
                offset
            });
        }
        let scroll_id = ui.make_persistent_id(egui::IdSalt::new(("folder_contents", folder)));
        let painted_offset = egui::scroll_area::State::load(ui.ctx(), scroll_id)
            .unwrap_or_default()
            .unclamped_offset()
            .y;
        let cadence_id = scroll_id.with("trackpad_cadence");
        let (time, delta, source) =
            ui.input(|i| (i.time, i.smooth_scroll_delta.y, i.scroll_source()));
        let hovered = enabled
            && ui.rect_contains_pointer(ui.available_rect_before_wrap())
            && ui.input(|i| !i.pointer.any_down());
        let max_offset = rows.len() as f32 * self.row_height - ui.available_height();
        // Leave edge pulling and spring/momentum behavior to the existing scroll physics.
        let interpolate = hovered
            && !entering
            && !self.reveal_selection
            && painted_offset > self.row_height * 2.
            && painted_offset < max_offset - self.row_height * 2.
            && matches!(
                source,
                Some(egui::MouseWheelSource::Trackpad | egui::MouseWheelSource::Momentum)
            );
        let movement = ui.ctx().data_mut(|data| {
            let cadence = data.get_temp_mut_or_default::<TrackpadCadence>(cadence_id);
            if !hovered || entering || self.reveal_selection {
                *cadence = TrackpadCadence::default();
                return delta;
            }
            cadence.step(delta, time, interpolate)
        });
        if hovered {
            ui.input_mut(|i| i.smooth_scroll_delta.y = movement);
            if (movement - delta).abs() > 0.05 {
                ui.ctx().request_repaint();
            }
        }
        let wheel_input = enabled
            && !entering
            && !self.reveal_selection
            && ui.rect_contains_pointer(ui.available_rect_before_wrap())
            && ui.input(|i| i.smooth_scroll_delta.y != 0. && !i.pointer.any_down());
        let output = scroll
            .id_salt(("folder_contents", folder))
            .scroll_source(
                egui::scroll_area::ScrollSource::SCROLL_BAR
                    | egui::scroll_area::ScrollSource::MOUSE_WHEEL,
            )
            .auto_shrink([false, false])
            .show_rows(
                ui,
                self.row_height - ui.spacing().item_spacing.y,
                rows.len(),
                |ui, range| {
                    for index in range {
                        let row = &rows[index];
                        ui.horizontal(|ui| {
                            ui.set_height(self.row_height - ui.spacing().item_spacing.y);
                            ui.add_space(row.depth as f32 * 14.0);
                            if let Some(message) = &row.message {
                                ui.add(egui::Label::new(RichText::new(message).weak()).truncate())
                                    .on_hover_text(message);
                                return;
                            }
                            let height = self.row_height - ui.spacing().item_spacing.y;
                            let selected = selected == Some(&row.path);
                            let colors = appearance::palette(ui);
                            let label = format!(
                                "{}{}",
                                crate::name(&row.path),
                                if row.symlink { " · Link" } else { "" }
                            );
                            let response = ui
                                .push_id(&row.path, |ui| {
                                    let icon =
                                        egui::Atom::paint(egui::vec2(18., 18.), |ui, args| {
                                            appearance::file_symbol(
                                                ui,
                                                args.rect,
                                                &row.path,
                                                row.directory,
                                            );
                                        });
                                    let mut button = egui::Button::selectable(
                                        selected,
                                        (icon, label, egui::Atom::grow()),
                                    )
                                    .truncate()
                                    .min_size(egui::vec2(ui.available_width(), height));
                                    if selected {
                                        button = button.stroke(egui::Stroke::NONE).fill(
                                            if self.focused {
                                                colors.selection
                                            } else {
                                                colors.inactive_selection
                                            },
                                        );
                                    }
                                    ui.add_enabled(enabled, button)
                                })
                                .inner
                                .on_hover_text(crate::paths::display(&row.path));
                            if selected && self.focused && enabled && response.has_focus() {
                                ui.painter().rect_stroke(
                                    response.rect.shrink(1.),
                                    6.,
                                    egui::Stroke::new(1.5, colors.accent),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            if response.clicked() {
                                clicked = Some(row.clone());
                            }
                        });
                    }
                },
            );
        if wheel_input && (output.state.unclamped_offset().y - painted_offset).abs() > 0.1 {
            // ScrollArea consumes wheel input after laying out its children. Rebuild those
            // rows before presenting, so the content does not trail the scrollbar by a frame.
            ui.ctx()
                .request_discard("Paint explorer rows at the current scroll position");
        }
        self.vertical
            .insert(view_key, output.state.clamped_offset().y);
        clicked
    }

    pub fn show(&mut self, ui: &mut egui::Ui, enabled: bool) -> NavigationAction {
        let mut action = NavigationAction::default();
        self.row_height = self.row_height.max(
            ui.text_style_height(&egui::TextStyle::Button)
                + 2. * ui.spacing().button_padding.y
                + ui.spacing().item_spacing.y,
        );
        let rect = ui.available_rect_before_wrap();
        if let Some(started) = self.navigation_started.filter(|_| self.usable()) {
            if std::env::var_os("GLIMPSE_MEASURE").is_some() {
                eprintln!(
                    "FOLDER_UI_READY_MS {:.3} {}",
                    started.elapsed().as_secs_f64() * 1000.,
                    crate::paths::display(&self.current)
                );
            }
            self.navigation_started = None;
        }
        let pressed = enabled
            && ui.input(|i| {
                i.pointer.any_pressed()
                    && i.pointer.interact_pos().is_some_and(|p| rect.contains(p))
            });
        if enabled && ui.input(|i| i.pointer.any_pressed()) && !pressed {
            self.focused = false;
        }
        if pressed {
            self.focused = true;
            ui.memory_mut(|m| m.request_focus(egui::Id::unique("navigator")));
        }
        let hovered =
            enabled && ui.input(|i| i.pointer.hover_pos().is_some_and(|p| rect.contains(p)));
        if hovered {
            let (events, time) = ui.input(|i| (i.events.clone(), i.time));
            let mut consumed = false;
            for event in events {
                if let egui::Event::MouseWheel {
                    delta,
                    unit,
                    phase,
                    source,
                    ..
                } = event
                {
                    let multiplier = match unit {
                        egui::MouseWheelUnit::Point => 1.0,
                        egui::MouseWheelUnit::Line => 24.0,
                        egui::MouseWheelUnit::Page => rect.width(),
                    };
                    if source == egui::MouseWheelSource::Momentum {
                        // macOS reports momentum as a new phased sequence after fingers lift.
                        // It belongs to the previous swipe and must never emit another step.
                        consumed |= delta.x.abs() > delta.y.abs() * 1.75;
                        continue;
                    }
                    if source == egui::MouseWheelSource::Wheel {
                        self.gesture_phased = false;
                    }
                    consumed |= self.gesture(delta * multiplier, phase, time);
                }
            }
            if consumed {
                ui.input_mut(|i| {
                    i.smooth_scroll_delta.x = 0.0;
                });
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Tab)) {
            self.focused = false;
        }
        if self.focused && enabled {
            self.keyboard(ui, &mut action);
        }
        let current = self.current.clone();
        let time = ui.input(|i| i.time);
        let entering = self.last_shown.as_ref() != Some(&current);
        if entering {
            if let Some(previous) = &self.last_shown {
                self.transition_started = time;
                self.transition_direction = if previous.starts_with(&current) {
                    -1.
                } else {
                    1.
                };
            }
            self.last_shown = Some(current.clone());
        }
        let progress = if self.reduce_motion {
            1.
        } else {
            ((time - self.transition_started) / 0.10).clamp(0., 1.) as f32
        };
        let offset = 32. * (1. - progress).powi(3) * self.transition_direction;
        if progress < 1. {
            ui.ctx().request_repaint();
        }
        let list_rect = ui.available_rect_before_wrap();
        // A translated transition must not increase the resizable pane's minimum width.
        let mut list_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("navigator_list")
                .max_rect(list_rect.translate(egui::vec2(offset, 0.)))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        list_ui.set_clip_rect(ui.clip_rect().intersect(list_rect));
        let clicked = self.list(&mut list_ui, &current, enabled, entering);
        ui.advance_cursor_after_rect(list_rect);
        self.follow = false;
        self.reveal_selection = false;
        if let Some(row) = clicked {
            if row.directory {
                self.navigate(row.path);
            } else {
                self.select_file(&row.path);
                action.open = Some((row.path, false));
            }
            self.focused = true;
            ui.memory_mut(|m| m.request_focus(egui::Id::unique("navigator")));
        }
        if self.focused && enabled {
            ui.memory_mut(|m| m.request_focus(egui::Id::unique("navigator")));
        }
        action.back = self.gesture_step.take();
        if self.follow {
            ui.ctx().request_repaint();
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trackpad_resampling_fills_display_frames_and_preserves_distance() {
        let mut cadence = TrackpadCadence::default();
        let mut distance = 0.;
        let mut movements = Vec::new();
        for tick in 0..80 {
            let delta = if tick < 20 && tick % 2 == 0 { -12. } else { 0. };
            let movement = cadence.step(delta, tick as f64 / 120., true);
            distance += movement;
            movements.push(movement);
        }
        assert!(
            movements[2..20].iter().all(|movement| *movement < -0.1),
            "60Hz input must move on intervening120Hz frames: {movements:?}"
        );
        assert!(
            (distance + 120.).abs() < 0.001,
            "scroll distance changed: {distance}"
        );
        assert_eq!(
            movements[79], 0.,
            "must stop requesting animation after settling"
        );
        assert_eq!(
            cadence.step(7., 2., false),
            7.,
            "edge input must remain direct"
        );
    }

    #[test]
    fn scroll_input_moves_painted_rows_in_the_same_frame() {
        let ctx = egui::Context::default();
        let folder = PathBuf::from("/fixture");
        let mut nav = Navigator::new(folder.clone(), folder.clone());
        nav.receive(
            folder.clone(),
            Ok((0..100)
                .map(|i| Entry {
                    path: folder.join(format!("item-{i:03}")),
                    directory: false,
                    symlink: false,
                })
                .collect()),
        );
        let mut positions = Vec::new();
        for tick in 0..3 {
            let mut events = vec![egui::Event::PointerMoved(egui::pos2(180., 200.))];
            if tick == 2 {
                for (phase, delta) in [
                    (egui::TouchPhase::Start, 0.),
                    (egui::TouchPhase::Move, -28.),
                ] {
                    events.push(egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0., delta),
                        phase,
                        source: egui::MouseWheelSource::Trackpad,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(340., 440.),
                    )),
                    time: Some(tick as f64 / 120.),
                    events,
                    ..Default::default()
                },
                |ui| {
                    nav.list(ui, &folder, true, tick == 0);
                },
            );
            let top = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.job.text == "item-001" => {
                        Some(text.pos.y)
                    }
                    _ => None,
                })
                .expect("the second row must be painted");
            positions.push(top);
            output.drop_without_applying_deltas();
        }
        assert!(
            (positions[1] - positions[2] - 28.).abs() < 1.,
            "the input frame must paint the new position: {positions:?}"
        );
    }

    #[test]
    fn trackpad_rows_move_between_sixty_hz_input_events() {
        let ctx = egui::Context::default();
        let folder = PathBuf::from("/fixture");
        let mut nav = Navigator::new(folder.clone(), folder.clone());
        nav.receive(
            folder.clone(),
            Ok((0..100)
                .map(|i| Entry {
                    path: folder.join(format!("item-{i:03}")),
                    directory: false,
                    symlink: false,
                })
                .collect()),
        );
        nav.vertical.insert((folder.clone(), false), 350.);
        let mut positions = Vec::new();
        for tick in 0..12 {
            let mut events = vec![egui::Event::PointerMoved(egui::pos2(180., 200.))];
            if tick >= 2 && tick % 2 == 0 {
                for (phase, delta) in [
                    (egui::TouchPhase::Start, 0.),
                    (egui::TouchPhase::Move, -12.),
                ] {
                    events.push(egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0., delta),
                        phase,
                        source: egui::MouseWheelSource::Trackpad,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(340., 440.),
                    )),
                    time: Some(tick as f64 / 120.),
                    events,
                    ..Default::default()
                },
                |ui| {
                    nav.list(ui, &folder, true, tick == 0);
                },
            );
            let top = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.job.text == "item-020" => {
                        Some(text.pos.y)
                    }
                    _ => None,
                })
                .expect("the second row must be painted");
            positions.push(top);
            output.drop_without_applying_deltas();
        }
        assert!(
            positions[2..]
                .windows(2)
                .all(|pair| pair[0] - pair[1] > 0.2),
            "painted rows must move between input samples: {positions:?}"
        );
    }

    #[test]
    fn trackpad_overscroll_survives_between_input_frames() {
        let ctx = egui::Context::default();
        let folder = PathBuf::from("/fixture");
        let mut nav = Navigator::new(folder.clone(), folder.clone());
        nav.receive(
            folder.clone(),
            Ok((0..100)
                .map(|i| Entry {
                    path: folder.join(format!("item-{i:03}")),
                    directory: false,
                    symlink: false,
                })
                .collect()),
        );
        let mut offsets = Vec::new();
        for tick in 0..4 {
            let events = if tick == 0 {
                vec![]
            } else {
                vec![
                    egui::Event::PointerMoved(egui::pos2(180., 200.)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0., 30.),
                        phase: if tick == 1 {
                            egui::TouchPhase::Start
                        } else {
                            egui::TouchPhase::Move
                        },
                        source: egui::MouseWheelSource::Trackpad,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            };
            let mut offset = 0.;
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(340., 440.),
                    )),
                    time: Some(tick as f64 / 120.),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let id = ui.make_persistent_id(egui::IdSalt::new(("folder_contents", &folder)));
                    nav.list(ui, &folder, true, tick == 0);
                    offset = egui::scroll_area::State::load(&ctx, id)
                        .unwrap()
                        .unclamped_offset()
                        .y;
                },
            );
            output.drop_without_applying_deltas();
            offsets.push(offset);
        }
        assert!(offsets[2] < 0., "first pull must overscroll: {offsets:?}");
        assert!(
            offsets[3] < offsets[2] - 1.,
            "continued pull must retain displacement: {offsets:?}"
        );
    }

    #[test]
    fn navigation_animation_keeps_resizable_pane_width_stable() {
        let ctx = egui::Context::default();
        let mut nav = Navigator::new("/fixture".into(), "/fixture".into());
        let mut widths = Vec::new();
        for (step, path) in [
            "/fixture",
            "/fixture/one",
            "/fixture/one/two",
            "/fixture/one/two/three",
            "/fixture",
        ]
        .into_iter()
        .enumerate()
        {
            nav.navigate(path.into());
            for tick in 0..20 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1280., 800.),
                        )),
                        time: Some(step as f64 + tick as f64 / 60.),
                        ..Default::default()
                    },
                    |ui| {
                        let pane = egui::Panel::left("directory_list_regression")
                            .default_size(280.)
                            .min_size(220.)
                            .show(ui, |ui| {
                                nav.show(ui, true);
                            });
                        widths.push(pane.response.rect.width());
                    },
                );
                output.textures_delta.clear();
            }
        }
        assert!(
            widths.iter().all(|width| (*width - widths[0]).abs() < 1.),
            "directory navigation resized the pane: {widths:?}"
        );
    }

    #[test]
    fn keyboard_respects_visible_filter_and_tab_leaves_composite() {
        let ctx = egui::Context::default();
        let mut nav = Navigator::new("/fixture".into(), "/fixture".into());
        nav.receive(
            "/fixture".into(),
            Ok([".hidden.txt", "alpha.txt", "beta.txt"]
                .into_iter()
                .map(|name| Entry {
                    path: Path::new("/fixture").join(name),
                    directory: false,
                    symlink: false,
                })
                .collect()),
        );
        nav.show_hidden = false;
        nav.filter = "beta".into();
        nav.focused = true;
        for key in [egui::Key::ArrowDown, egui::Key::Tab] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(320., 400.),
                    )),
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ui| {
                    nav.show(ui, true);
                },
            );
            output.textures_delta.clear();
        }
        assert_eq!(nav.selection(), Some("/fixture/beta.txt".into()));
        assert!(!nav.focused, "Tab must not be trapped in the navigator");
    }
}
