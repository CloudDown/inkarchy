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
fn hand_moves_and_resizes_images_without_panning() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Direct images");
    let app = &mut fixture.app;
    app.open_note(id);
    app.insert_png(&png(240, 120), 0, pos2(180.0, 180.0), None);
    assert_eq!(app.tool, Tool::Hand);
    assert_eq!(app.sel.len(), 1, "new image is ready to manipulate");
    let mut frames = Frames::new(app);
    drawing_view(app, &mut frames);
    let camera = app.camera;
    app.sel.clear();
    app.tool = Tool::Hand;
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
    assert_eq!(app.tool, Tool::Hand);
    assert!(app.canvas_touch.is_none() && app.drag_last.is_none());
    frames.settle(app);
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
fn hand_selects_and_moves_ink_while_a_brush_pans() {
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
    app.tool = Tool::Hand;
    let camera = app.camera;
    let p = camera.to_screen(pos2(300.0, 235.0), app.canvas_rect);
    frames.touch(app, TouchPhase::Start, p);
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
    app.tool = Tool::Brush;
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
fn lost_touch_end_does_not_block_later_finger_pan() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Stale touch");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    drawing_view(app, &mut frames);
    let origin = app.canvas_rect.center();
    app.plant_stale_canvas_touch(origin);
    assert!(app.canvas_touch.is_some());
    frames.step(app, vec![]);
    assert!(
        app.canvas_touch.is_none(),
        "a gesture with no live finger must clear"
    );
    let pan_before = app.camera.pan;
    frames.touch(app, TouchPhase::Start, origin);
    frames.touch(app, TouchPhase::Move, origin + vec2(48.0, 12.0));
    frames.touch(app, TouchPhase::End, origin + vec2(48.0, 12.0));
    assert!(
        (app.camera.pan - pan_before).length() > 1.0,
        "finger pan works again after a lost End"
    );
    assert!(app.canvas_touch.is_none());
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
    app.media_import = Some(MediaImport {
        note: id,
        cell: (0, 0),
        local: Pos2::ZERO,
        replace: None,
        result,
    });
    send.send(MediaImportResult::Image(png(20, 20)))
        .ok()
        .unwrap();
    app.poll_media_import(&Context::default());
    assert!(app.note.as_ref().unwrap().pages[0].images.is_empty());
    assert!(app.media_import.is_none());
}

#[test]
fn video_lands_as_a_media_object_with_a_poster_and_scales_like_an_image() {
    if std::process::Command::new("ffprobe")
        .arg("-version")
        .output()
        .is_err()
        || std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_err()
    {
        return;
    }
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Media");
    let clip = fixture.root.join("clip.mp4");
    assert!(std::process::Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=160x120:rate=10",
            "-t",
            "1",
            "-pix_fmt",
            "yuv420p",
            "-y",
        ])
        .arg(&clip)
        .status()
        .unwrap()
        .success());
    let app = &mut fixture.app;
    app.open_note(id);
    let meta = crate::video::probe(&clip).unwrap();
    app.insert_video_path(&clip, "mp4", meta, 0, pos2(250.0, 150.0), None)
        .unwrap();
    let image = app.note.as_ref().unwrap().pages[0].images[0].clone();
    assert!(crate::video::is_video_file(&image.file));
    assert_eq!(app.tool, Tool::Hand);
    assert_eq!(app.sel.len(), 1);
    assert_eq!(image.size, [160.0, 120.0]);
    let poster = app
        .lib
        .media_path(id, &crate::video::poster_name(&image.file));
    assert!(poster.is_file(), "a still poster is stored beside the clip");

    app.sync_videos(&Context::default());
    assert!(app.videos.contains_key(&image.id));
    app.scale_selection(pos2(0.0, 0.0), 2.0);
    assert_eq!(
        app.note.as_ref().unwrap().pages[0].images[0].size,
        [320.0, 240.0]
    );
    let mut frames = Frames::new(app);
    frames.settle(app);
    let button = app
        .video_button_rect(
            app.canvas_rect,
            0,
            &app.note.as_ref().unwrap().pages[0].images[0],
        )
        .unwrap();
    // The button's hit target stays present even if its player cannot load.
    assert_eq!(
        app.video_button_at(button.center(), app.canvas_rect)
            .unwrap()
            .1,
        image.id
    );

    let player = app.videos.remove(&image.id).unwrap();
    frames.step(app, vec![Event::PointerMoved(button.center())]);
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: button.center(),
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
    );
    assert_eq!(app.video_press, Some(image.id));
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: button.center(),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    app.videos.insert(image.id, player);

    // Pointer events are also emitted alongside touch by egui backends. They
    // must not steal the touch's ownership or swallow the following mouse click.
    frames.touch(app, TouchPhase::Start, button.center());
    frames.touch(app, TouchPhase::End, button.center());
    assert!(
        app.videos[&image.id].is_playing(),
        "the on-video button starts playback"
    );
    frames.touch(app, TouchPhase::Start, button.center());
    frames.touch(app, TouchPhase::End, button.center());
    assert!(
        !app.videos[&image.id].is_playing(),
        "the same button pauses playback"
    );

    frames.step(app, vec![Event::PointerMoved(button.center())]);
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: button.center(),
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
    );
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: button.center(),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert!(
        app.videos[&image.id].is_playing(),
        "a mouse click on the button starts playback"
    );
    frames.snapshot("video-playing");
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: button.center(),
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
    );
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: button.center(),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert!(
        !app.videos[&image.id].is_playing(),
        "a second mouse click pauses playback"
    );
    frames.snapshot("video-player");
}

#[test]
fn selected_image_stays_usable_on_small_windows_without_an_action_bar() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Small touch screen");
    let app = &mut fixture.app;
    app.open_note(id);
    app.insert_png(&png(240, 120), 0, pos2(180.0, 180.0), None);
    let image_id = app.note.as_ref().unwrap().pages[0].images[0].id;
    let mut frames = Frames::new(app);
    for size in [vec2(320.0, 280.0), vec2(360.0, 640.0), vec2(1280.0, 860.0)] {
        frames.size = size;
        app.fit_to_screen();
        app.sel = vec![Sel {
            page: 0,
            id: image_id,
            kind: SelKind::Image,
        }];
        frames.settle(app);
        assert!(frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("selection-actions")))
            .is_none());
        assert_eq!(app.note.as_ref().unwrap().pages[0].images.len(), 1);
        frames.snapshot(&format!("touch-image-{}", size.x as u32));
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
    let target = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("tool-well", Tool::Pencil))))
        .unwrap()
        .center();
    frames.touch(app, TouchPhase::Start, target);
    frames.time += 0.7;
    frames.step(app, vec![]);
    assert!(app.tin_open, "settings open during the hold itself");
    frames.touch(app, TouchPhase::End, target);
    frames.settle(app);
    assert_eq!(app.tool, Tool::Pencil);
    assert!(
        app.tin_open,
        "long press opens size settings without a follow-up click"
    );
    assert_eq!(app.ink_panel_view, InkPanelView::Nib);
    assert!(!app.color_mixer_open);
    frames.snapshot("touch-tool-settings");
    app.tool = Tool::EraserArea;
    app.last_eraser = Tool::EraserArea;
    app.width = 27.0;
    app.tin_open = false;
    frames.settle(app);
    let eraser = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("tool-well", Tool::EraserStroke))))
        .unwrap()
        .center();
    frames.touch(app, TouchPhase::Start, eraser);
    frames.time += 0.4;
    frames.step(app, vec![]);
    assert!(app.tin_open, "eraser hold opens size settings");
    assert_eq!(app.ink_panel_view, InkPanelView::Nib);
    assert_eq!(app.width, 27.0, "eraser hold preserves its width");
    frames.time += 0.5;
    frames.step(app, vec![]);
    assert!(
        app.tin_open,
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
                let well = app.ink_well(ui, false);
                assert_eq!(well.rect.size(), vec2(48.0, 48.0), "vertical={vertical}");
            });
        });
    }
}

#[test]
fn saved_color_hold_arms_a_bin_and_removal_is_remembered() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Saved inks");
    let app = &mut fixture.app;
    app.open_note(id);
    app.set_ink_panel(true);
    app.ink_panel_view = InkPanelView::Colors;
    app.pick_free_ink(Color32::from_rgb(12, 34, 56));
    app.save_ink_swatch();
    assert_eq!(app.lib.index.custom_inks.len(), 1);
    let mut frames = Frames::new(app);
    frames.settle(app);
    let index = app.look.inks.len();
    let swatch = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("ink-swatch", index))))
        .unwrap();
    let theme = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("ink-swatch", 0_usize))))
        .unwrap();
    frames.touch(app, TouchPhase::Start, theme.center());
    frames.time += 0.7;
    frames.step(app, vec![]);
    frames.touch(app, TouchPhase::End, theme.center());
    assert_eq!(app.ink_delete, None, "theme pigments are never removable");
    frames.settle(app);
    frames.touch(app, TouchPhase::Start, swatch.center());
    frames.time += 0.7;
    frames.step(app, vec![]);
    frames.touch(app, TouchPhase::End, swatch.center());
    frames.settle(app);
    assert_eq!(app.ink_delete, Some(0), "a hold arms removal");
    assert_eq!(app.lib.index.custom_inks.len(), 1, "a hold never deletes");
    assert!(frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("ink-add-color")))
        .is_none());
    let bin = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("ink-delete-color")))
        .unwrap();
    assert_eq!(bin.size(), Vec2::splat(48.0));
    frames.touch(app, TouchPhase::Start, bin.center());
    frames.touch(app, TouchPhase::End, bin.center());
    frames.settle(app);
    assert!(app.lib.index.custom_inks.is_empty());
    assert_eq!(app.ink_delete, None);
    assert!(frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("ink-delete-color")))
        .is_none());
    assert!(frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("ink-add-color")))
        .is_some());
    let persisted: crate::library::Index =
        serde_json::from_slice(&std::fs::read(app.lib.root.join("library.json")).unwrap()).unwrap();
    assert!(persisted.custom_inks.is_empty(), "removal is saved");
}

#[test]
fn toolbar_touch_swipes_scroll_without_storing_or_drawing() {
    for (edge, size, delta) in [
        (DockEdge::Bottom, vec2(360.0, 500.0), vec2(-80.0, 0.0)),
        (DockEdge::Right, vec2(500.0, 360.0), vec2(0.0, -80.0)),
    ] {
        let mut fixture = Fixture::new();
        let id = fixture.notebook("Swipe tools");
        let app = &mut fixture.app;
        app.open_note(id);
        app.dock_edge = edge;
        let mut frames = Frames::new(app);
        frames.size = size;
        frames.settle(app);
        let rect = |frames: &Frames, tool| {
            frames
                .ctx
                .data(|d| d.get_temp::<Rect>(Id::new(("tool-well", tool))))
                .unwrap()
        };
        let before = rect(&frames, Tool::Fineliner);
        let from = rect(&frames, Tool::Brush).center();
        let tool = app.tool;
        let pages = serde_json::to_value(&app.note.as_ref().unwrap().pages).unwrap();
        frames.touch(app, TouchPhase::Start, from);
        for fraction in [0.2, 0.5, 0.8, 1.0] {
            frames.touch(app, TouchPhase::Move, from + delta * fraction);
        }
        frames.touch(app, TouchPhase::End, from + delta);
        frames.settle(app);
        let after = rect(&frames, Tool::Fineliner);
        let moved = if edge.vertical() {
            before.top() - after.top()
        } else {
            before.left() - after.left()
        };
        assert!(moved > 40.0, "{edge:?}: swipe scrolls tools, moved {moved}");
        assert!(!app.ink_panel_open());
        assert!(app.dock_tool_drag.is_none() && app.parked_count() == 0);
        assert_eq!(app.tool, tool, "swiping never selects a tool");
        assert_eq!(
            serde_json::to_value(&app.note.as_ref().unwrap().pages).unwrap(),
            pages
        );
    }
}

#[test]
fn active_pen_tap_opens_spaced_touch_size_presets() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Tap for size");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    for tool in [
        Tool::Fineliner,
        Tool::Brush,
        Tool::Pencil,
        Tool::Highlighter,
    ] {
        app.tool = tool;
        app.set_ink_panel(true);
        app.ink_panel_view = InkPanelView::Colors;
        app.color_mixer_open = true;
        frames.settle(app);
        let pen = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new(("tool-well", tool))))
            .unwrap();
        assert_eq!(pen.size(), Vec2::splat(48.0));
        frames.touch(app, TouchPhase::Start, pen.center());
        frames.touch(app, TouchPhase::End, pen.center());
        frames.settle(app);
        assert_eq!(app.ink_panel_view, InkPanelView::Nib);
        assert!(app.ink_panel_open() && !app.color_mixer_open);
        let dock = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
            .unwrap();
        let tin = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("tin-rect")))
            .unwrap();
        assert!(
            (dock.size() - tin.size()).length() < 1.0,
            "size bar matches the toolbar"
        );
        let mut presets = Vec::new();
        for i in 0..32 {
            match frames
                .ctx
                .data(|d| d.get_temp::<Rect>(Id::new(("nib-preset", i))))
            {
                Some(preset) => presets.push(preset),
                None => break,
            }
        }
        assert!(
            presets.len() >= 4,
            "size options follow the bar, got {}",
            presets.len()
        );
        let first = *presets.first().unwrap();
        let last = *presets.last().unwrap();
        assert!(
            first.left() - tin.left() < 28.0 && tin.right() - last.right() < 28.0,
            "dots fill the bar, tin {tin:?} first {first:?} last {last:?}"
        );
        let mut previous_width = f32::MAX;
        for (i, preset) in presets.iter().enumerate() {
            frames.touch(app, TouchPhase::Start, preset.center());
            frames.touch(app, TouchPhase::End, preset.center());
            if i > 0 {
                assert!(
                    app.width <= previous_width,
                    "dots go from thick to thin, {previous_width} then {}",
                    app.width
                );
            }
            previous_width = app.width;
            assert_eq!(app.last_ink_width, app.width);
        }
        assert!(app.width < 4.0, "the last dot is a hairline");
        frames.touch(app, TouchPhase::Start, pen.center());
        frames.touch(app, TouchPhase::End, pen.center());
        assert!(!app.ink_panel_open(), "tap again closes size settings");
        assert_eq!(app.tool, tool);
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
    app.tool = Tool::Hand;
    let mut frames = Frames::new(app);
    drawing_view(app, &mut frames);
    let p = app.camera.to_screen(pos2(300.0, 240.0), app.canvas_rect);
    frames.touch(app, TouchPhase::Start, p);
    frames.touch(app, TouchPhase::Move, p + vec2(20.0, 0.0));
    frames.touch(app, TouchPhase::Cancel, p + vec2(20.0, 0.0));
    assert!(app.canvas_touch.is_none() && app.drag_last.is_none());
    frames.settle(app);
    assert_eq!(app.sel.len(), 1);
    frames.touch(app, TouchPhase::Start, p);
    frames.touch(app, TouchPhase::End, p);
    frames.touch(app, TouchPhase::Start, p);
    frames.touch(app, TouchPhase::End, p);
    assert_eq!(
        app.editing_text,
        Some((0, text_id)),
        "a second tap opens the text"
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
    app.tool = Tool::Lasso;
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
