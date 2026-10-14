use super::*;

fn png(w: u32, h: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(w, h, image::Rgba([80, 120, 180, 255]));
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(image)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    bytes
}

fn drawing_view(app: &mut InkarchyApp, frames: &mut Frames) {
    frames.settle(app);
    app.camera = Camera {
        pan: vec2(30.0, 30.0),
        zoom: 1.0,
        stretch_y: None,
    };
    app.fitted_cell = None;
    frames.settle(app);
}

#[test]
fn finger_moves_and_resizes_images_without_switching_tools_or_panning() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Direct images");
    let app = &mut fixture.app;
    app.open_note(id);
    app.insert_png(&png(240, 120), 0, pos2(180.0, 180.0), None);
    assert_eq!(app.tool, Tool::Lasso);
    assert_eq!(app.sel.len(), 1, "new image is ready to manipulate");
    let mut frames = Frames::new(app);
    drawing_view(app, &mut frames);
    let camera = app.camera;
    app.sel.clear();
    app.tool = Tool::Brush;
    app.undo.clear();
    let start = app.camera.to_screen(pos2(280.0, 235.0), app.canvas_rect);
    frames.touch(app, TouchPhase::Start, start);
    frames.touch(app, TouchPhase::Move, start + vec2(60.0, 40.0));
    frames.touch(app, TouchPhase::End, start + vec2(60.0, 40.0));
    assert_eq!(
        app.note.as_ref().unwrap().pages[0].images[0].pos,
        [240.0, 220.0]
    );
    assert_eq!(app.camera.pan, camera.pan);
    assert_eq!(app.tool, Tool::Brush);
    assert!(app.canvas_touch.is_none() && app.drag_last.is_none());
    frames.settle(app);
    frames.text_center("Replace");
    let before = app.note.as_ref().unwrap().pages[0].images[0].clone();
    let corner = app.camera.to_screen(before.rect().max, app.canvas_rect);
    let grip = corner + vec2(15.0, 0.0);
    frames.touch(app, TouchPhase::Start, grip);
    assert_eq!(
        app.note.as_ref().unwrap().pages[0].images[0].size,
        before.size,
        "wide handle must not jump"
    );
    let end = grip + vec2(120.0, 60.0);
    frames.touch(app, TouchPhase::Move, end);
    frames.touch(app, TouchPhase::End, end);
    let image = &app.note.as_ref().unwrap().pages[0].images[0];
    assert_eq!(image.pos, before.pos);
    assert!((image.size[0] - 360.0).abs() < 0.1);
    assert!((image.size[1] - 180.0).abs() < 0.1);
    assert_eq!(app.camera.pan, camera.pan);
    assert!(app.undo.undo(app.note.as_mut().unwrap()));
    assert_eq!(
        app.note.as_ref().unwrap().pages[0].images[0].size,
        before.size
    );
    assert!(app.undo.undo(app.note.as_mut().unwrap()));
    assert_eq!(
        app.note.as_ref().unwrap().pages[0].images[0].pos,
        [180.0, 180.0]
    );
    assert!(
        !app.undo.undo(app.note.as_mut().unwrap()),
        "one undo step per changed gesture"
    );
    frames.settle(app);
    frames.snapshot("touch-image-actions");
}

#[test]
fn finger_pan_ink_eraser_and_text_have_distinct_gestures() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Touch tools");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    drawing_view(app, &mut frames);
    let p = pos2(360.0, 320.0);
    let camera = app.camera;
    frames.touch(app, TouchPhase::Start, p);
    frames.touch(app, TouchPhase::Move, p + vec2(40.0, 20.0));
    frames.touch(app, TouchPhase::End, p + vec2(40.0, 20.0));
    assert_eq!(app.camera.pan, camera.pan + vec2(40.0, 20.0));
    assert!(app.note.as_ref().unwrap().pages[0].strokes.is_empty());
    app.lib.index.finger_draw = true;
    let camera = app.camera;
    for (index, tool) in [
        Tool::Fineliner,
        Tool::Brush,
        Tool::Pencil,
        Tool::Highlighter,
    ]
    .into_iter()
    .enumerate()
    {
        app.tool = tool;
        let p = p + vec2(0.0, index as f32 * 36.0);
        frames.touch(app, TouchPhase::Start, p);
        frames.touch(app, TouchPhase::Move, p + vec2(60.0, 8.0));
        frames.touch(app, TouchPhase::End, p + vec2(60.0, 8.0));
        assert_eq!(app.note.as_ref().unwrap().pages[0].strokes.len(), index + 1);
        assert!(app.live.is_none());
    }
    assert_eq!(app.camera.pan, camera.pan);
    app.lib.index.finger_draw = false;
    app.tool = Tool::EraserStroke;
    frames.touch(app, TouchPhase::Start, p);
    frames.touch(app, TouchPhase::End, p);
    assert_eq!(app.note.as_ref().unwrap().pages[0].strokes.len(), 3);
    assert_eq!(app.camera.pan, camera.pan);
    assert!(app.undo.undo(app.note.as_mut().unwrap()));
    assert_eq!(app.note.as_ref().unwrap().pages[0].strokes.len(), 4);
    app.tool = Tool::Text;
    frames.touch(app, TouchPhase::Start, pos2(650.0, 360.0));
    frames.touch(app, TouchPhase::End, pos2(650.0, 360.0));
    assert_eq!(app.note.as_ref().unwrap().pages[0].texts.len(), 1);
    assert!(app.editing_text.is_some());
}

#[test]
fn long_hold_selects_ink_but_dragging_it_pans() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Hold stroke");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut stroke = InkStroke::new(Nib::Fineliner, Color32::BLACK, 2.0);
    stroke.push(InkPoint::new(pos2(200.0, 220.0), 0.7));
    stroke.push(InkPoint::new(pos2(400.0, 250.0), 0.7));
    let id = stroke.id;
    app.note.as_mut().unwrap().pages[0].strokes.push(stroke);
    let mut frames = Frames::new(app);
    drawing_view(app, &mut frames);
    let camera = app.camera;
    let p = camera.to_screen(pos2(300.0, 235.0), app.canvas_rect);
    frames.touch(app, TouchPhase::Start, p);
    frames.time += 0.4;
    frames.step(app, vec![]);
    assert_eq!(app.sel.len(), 1);
    assert_eq!(app.sel[0].id, id);
    frames.touch(app, TouchPhase::Move, p + vec2(32.0, 48.0));
    frames.touch(app, TouchPhase::End, p + vec2(32.0, 48.0));
    assert_eq!(app.camera.pan, camera.pan);
    assert_eq!(
        app.note.as_ref().unwrap().pages[0].strokes[0].points[0].pos(),
        pos2(232.0, 268.0)
    );
    app.sel.clear();
    let p = p + vec2(32.0, 48.0);
    frames.touch(app, TouchPhase::Start, p);
    frames.touch(app, TouchPhase::Move, p + vec2(45.0, 10.0));
    frames.touch(app, TouchPhase::End, p + vec2(45.0, 10.0));
    assert_eq!(app.camera.pan, camera.pan + vec2(45.0, 10.0));
    assert!(app.sel.is_empty());
}

fn contact(id: u64, phase: TouchPhase, pos: Pos2) -> Event {
    Event::Touch {
        device_id: TouchDeviceId(1),
        id: TouchId(id),
        phase,
        pos,
        force: None,
    }
}

#[test]
fn pinch_takes_over_object_drag_until_both_fingers_lift() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Pinch handoff");
    let app = &mut fixture.app;
    app.open_note(id);
    app.insert_png(&png(240, 120), 0, pos2(180.0, 180.0), None);
    let mut frames = Frames::new(app);
    drawing_view(app, &mut frames);
    let p = app.camera.to_screen(pos2(280.0, 235.0), app.canvas_rect);
    frames.touch(app, TouchPhase::Start, p);
    frames.touch(app, TouchPhase::Move, p + vec2(20.0, 0.0));
    let before = app.note.as_ref().unwrap().pages[0].images[0].clone();
    frames.step(
        app,
        vec![contact(2, TouchPhase::Start, p + vec2(140.0, 0.0))],
    );
    frames.step(
        app,
        vec![
            contact(1, TouchPhase::Move, p),
            contact(2, TouchPhase::Move, p + vec2(180.0, 10.0)),
        ],
    );
    assert!(app.camera.zoom > 1.0);
    frames.step(
        app,
        vec![contact(2, TouchPhase::End, p + vec2(180.0, 10.0))],
    );
    frames.touch(app, TouchPhase::Move, p - vec2(30.0, 40.0));
    frames.touch(app, TouchPhase::End, p - vec2(30.0, 40.0));
    assert_eq!(
        app.note.as_ref().unwrap().pages[0].images[0].pos,
        before.pos
    );
    assert_eq!(
        app.note.as_ref().unwrap().pages[0].images[0].size,
        before.size
    );
    assert!(app.canvas_touch.is_none() && app.drag_last.is_none() && !app.touch_navigation);
}

#[test]
fn image_replacement_is_undoable_and_stale_imports_cannot_change_another_note() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Images");
    let other = fixture.notebook("Other note");
    let app = &mut fixture.app;
    app.open_note(id);
    app.insert_png(&png(240, 120), 0, pos2(180.0, 180.0), None);
    let before = app.note.as_ref().unwrap().pages[0].images[0].clone();
    app.insert_png(&png(100, 100), 0, Pos2::ZERO, Some(before.id));
    let after = &app.note.as_ref().unwrap().pages[0].images[0];
    assert_eq!(after.id, before.id);
    assert_eq!(after.pos, before.pos);
    assert!((after.size[0] - 120.0).abs() < 0.01 && (after.size[1] - 120.0).abs() < 0.01);
    assert_ne!(after.file, before.file);
    assert!(app.undo.undo(app.note.as_mut().unwrap()));
    assert_eq!(
        app.note.as_ref().unwrap().pages[0].images[0].file,
        before.file
    );
    app.close_desk();
    app.open_note(other);
    let (send, result) = mpsc::channel();
    app.image_import = Some(ImageImport {
        note: id,
        cell: (0, 0),
        local: Pos2::ZERO,
        replace: None,
        result,
    });
    send.send(ImageImportResult::Ready(png(20, 20)))
        .ok()
        .unwrap();
    app.poll_image_import(&Context::default());
    assert!(app.note.as_ref().unwrap().pages[0].images.is_empty());
    assert!(app.image_import.is_none());
}

#[test]
fn selected_image_actions_remain_accessible_on_small_windows() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Small touch screen");
    let app = &mut fixture.app;
    app.open_note(id);
    app.insert_png(&png(240, 120), 0, pos2(180.0, 180.0), None);
    let mut frames = Frames::new(app);
    for size in [vec2(320.0, 280.0), vec2(360.0, 640.0), vec2(1280.0, 860.0)] {
        frames.size = size;
        app.fit_to_screen();
        frames.settle(app);
        let actions = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("selection-actions")))
            .unwrap();
        let dock = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
            .unwrap();
        assert!(
            Rect::from_min_size(Pos2::ZERO, size).contains_rect(actions),
            "{size:?}: {actions:?}"
        );
        assert!(
            !actions.intersects(dock),
            "{size:?}: actions {actions:?} overlap dock {dock:?}"
        );
        for label in ["Replace", "Duplicate", "Delete", "Done"] {
            let point = frames.text_center(label);
            assert!(
                frames.output.as_ref().unwrap().shapes.iter().any(|shape| {
                    matches!(&shape.shape, Shape::Text(text) if text.galley.text() == label)
                        && shape.clip_rect.contains(point)
                }),
                "{size:?}: {label} is clipped"
            );
        }
        frames.snapshot(&format!("touch-image-{}", size.x as u32));
        let duplicate = frames.text_center("Duplicate");
        frames.touch(app, TouchPhase::Start, duplicate);
        frames.touch(app, TouchPhase::End, duplicate);
        assert_eq!(app.note.as_ref().unwrap().pages[0].images.len(), 2);
        assert!(app.undo.undo(app.note.as_mut().unwrap()));
        app.sel = vec![Sel {
            page: 0,
            id: app.note.as_ref().unwrap().pages[0].images[0].id,
            kind: SelKind::Image,
        }];
    }
}

#[test]
fn held_tool_opens_settings_and_all_swatches_have_finger_sized_targets() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Tool settings");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    let fill = app.tool_well_fill(Tool::Pencil, false);
    let target = frames
        .output
        .as_ref()
        .unwrap()
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            Shape::Rect(r) if r.fill == fill && (r.rect.width() - 40.0).abs() < 0.1 => {
                Some(r.rect.center())
            }
            _ => None,
        })
        .unwrap();
    frames.touch(app, TouchPhase::Start, target);
    frames.time += 0.7;
    frames.step(app, vec![]);
    assert!(app.tin_open, "settings open during the hold itself");
    frames.touch(app, TouchPhase::End, target);
    frames.settle(app);
    assert_eq!(app.tool, Tool::Pencil);
    assert!(
        app.tin_open,
        "long press opens the colour wheel without a follow-up click"
    );
    frames.snapshot("touch-tool-settings");
    app.tool = Tool::EraserArea;
    app.last_eraser = Tool::EraserArea;
    app.width = 27.0;
    app.tin_open = false;
    frames.settle(app);
    let fill = app.tool_well_fill(Tool::EraserArea, true);
    let eraser = frames
        .output
        .as_ref()
        .unwrap()
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            Shape::Rect(r) if r.fill == fill && (r.rect.width() - 40.0).abs() < 0.1 => {
                Some(r.rect.center())
            }
            _ => None,
        })
        .unwrap();
    frames.touch(app, TouchPhase::Start, eraser);
    frames.time += 0.4;
    frames.step(app, vec![]);
    assert!(!app.tin_open, "eraser hold does not open the colour wheel");
    assert_eq!(app.width, 27.0, "eraser hold preserves its width");
    frames.time += 0.5;
    frames.step(app, vec![]);
    assert!(
        !app.tin_open,
        "a hold fires once, including egui's later long-touch event"
    );
    frames.touch(app, TouchPhase::End, eraser);
    assert_eq!(
        app.tool,
        Tool::EraserArea,
        "release does not toggle back to ink"
    );
    let ctx = Context::default();
    for vertical in [true, false] {
        let _ = ctx.run(RawInput::default(), |ctx| {
            CentralPanel::default().show(ctx, |ui| {
                let chip = app.palette_chip(ui, Color32::BLACK, false, vertical);
                assert_eq!(chip.rect.size(), vec2(44.0, 44.0));
                let thickness = ui.next_auto_id();
                app.thick_dot(ui, 4.4, true, "Medium");
                assert_eq!(
                    ctx.read_response(thickness).unwrap().rect.size(),
                    vec2(44.0, 44.0)
                );
            });
        });
    }
}

#[test]
fn finger_text_selection_offers_edit_and_cancel_releases_object_grab() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Text gestures");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut text = TextBox::new(pos2(200.0, 200.0), Color32::BLACK);
    text.text = "Tap to select, edit to type".into();
    text.size = [260.0, 90.0];
    let text_id = text.id;
    app.note.as_mut().unwrap().pages[0].texts.push(text);
    let mut frames = Frames::new(app);
    drawing_view(app, &mut frames);
    let p = app.camera.to_screen(pos2(300.0, 240.0), app.canvas_rect);
    frames.touch(app, TouchPhase::Start, p);
    frames.touch(app, TouchPhase::Move, p + vec2(20.0, 0.0));
    frames.touch(app, TouchPhase::Cancel, p + vec2(20.0, 0.0));
    assert!(app.canvas_touch.is_none() && app.drag_last.is_none());
    frames.settle(app);
    let edit = frames.text_center("Edit text");
    frames.snapshot("edit-text-action");
    assert_eq!(app.sel.len(), 1);
    frames.touch(app, TouchPhase::Start, edit);
    assert_eq!(app.sel.len(), 1, "tap stays owned by the selection toolbar");
    frames.touch(app, TouchPhase::End, edit);
    assert_eq!(
        app.editing_text,
        Some((0, text_id)),
        "edit activates on release"
    );
    frames.step(app, vec![]);
    assert_eq!(app.editing_text, Some((0, text_id)));
    assert!(app.sel.is_empty());
}

#[test]
fn finger_lasso_selects_a_group_of_images_and_keeps_camera_still() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Finger lasso");
    let app = &mut fixture.app;
    app.open_note(id);
    app.insert_png(&png(100, 80), 0, pos2(200.0, 200.0), None);
    app.insert_png(&png(100, 80), 0, pos2(400.0, 200.0), None);
    app.sel.clear();
    let mut frames = Frames::new(app);
    drawing_view(app, &mut frames);
    let camera = app.camera;
    let canvas = app.canvas_rect;
    let points = [
        pos2(160.0, 160.0),
        pos2(540.0, 160.0),
        pos2(540.0, 320.0),
        pos2(160.0, 320.0),
    ];
    frames.touch(app, TouchPhase::Start, camera.to_screen(points[0], canvas));
    for point in &points[1..] {
        frames.touch(app, TouchPhase::Move, camera.to_screen(*point, canvas));
    }
    frames.touch(app, TouchPhase::End, camera.to_screen(points[0], canvas));
    assert_eq!(app.sel.len(), 2);
    assert_eq!(app.camera.pan, camera.pan);
    assert!(app.lasso.is_empty());
}
