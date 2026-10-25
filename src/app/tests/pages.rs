use super::*;

fn control(frames: &Frames, group: &str, name: &str) -> Pos2 {
    frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new((group, name))))
        .unwrap()
        .center()
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

fn prepare(app: &mut InkarchyApp) -> Frames {
    let note = app.note.as_mut().unwrap();
    note.sheet_join = SheetJoin::Separate;
    note.pages = (0..8).map(|i| crate::document::Page::at(0, i)).collect();
    let mut frames = Frames::new(app);
    frames.settle(app);
    app.enter_pages_mode();
    frames.settle(app);
    frames
}

#[test]
fn column_scrub_wheel_keyboard_and_picker_preserve_content_and_group_undo() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Columns");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = prepare(app);
    app.undo.clear();
    let original = serde_json::to_value(app.note.as_ref().unwrap()).unwrap();
    let count = control(&frames, "strip-flow", "Count");
    frames.step(app, vec![Event::PointerMoved(count), button(count, true)]);
    for delta in [24.0, 48.0, 72.0] {
        frames.step(app, vec![Event::PointerMoved(count + vec2(delta, 0.0))]);
    }
    frames.step(app, vec![button(count + vec2(72.0, 0.0), false)]);
    assert_eq!(app.page_arrangement(), Some(PageArrangement::FourColumns));
    assert!(app.undo.undo(app.note.as_mut().unwrap()));
    assert_eq!(
        serde_json::to_value(app.note.as_ref().unwrap()).unwrap(),
        original
    );
    assert!(
        !app.undo.undo(app.note.as_mut().unwrap()),
        "one scrub is one history entry"
    );

    frames.settle(app);
    let count = control(&frames, "strip-flow", "Count");
    let camera = app.camera;
    for _ in 0..2 {
        frames.step(
            app,
            vec![
                Event::PointerMoved(count),
                Event::MouseWheel {
                    unit: MouseWheelUnit::Line,
                    delta: vec2(0.0, 1.0),
                    modifiers: Modifiers::NONE,
                },
            ],
        );
    }
    assert_eq!(app.page_arrangement(), Some(PageArrangement::ThreeColumns));
    assert_eq!(app.camera.zoom, camera.zoom);
    assert!(app.undo.undo(app.note.as_mut().unwrap()));
    assert_eq!(app.page_arrangement(), Some(PageArrangement::Column));
    assert!(
        !app.undo.undo(app.note.as_mut().unwrap()),
        "one wheel burst is one history entry"
    );

    frames.drag(app, count, count);
    frames.settle(app);
    frames.snapshot("pages-column-picker");
    let popup = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("pages-settings-popup-rect")))
        .unwrap();
    assert!(InkarchyApp::pen_over_chrome(&frames.ctx, popup.center()));
    let pick = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("pages-column-choice", 5_u8))))
        .unwrap()
        .center();
    frames.touch(app, TouchPhase::Start, pick);
    frames.touch(app, TouchPhase::End, pick);
    frames.settle(app);
    assert_eq!(app.page_arrangement(), Some(PageArrangement::FiveColumns));
    assert!(app.sheet_tabs_on);
    assert!(app
        .note
        .as_ref()
        .unwrap()
        .pages
        .iter()
        .all(|p| p.strokes.is_empty()));

    // Focus the count without using an application-global shortcut.
    let count_id = Id::new("pages-edit-strip").with(("strip-flow", "Count"));
    frames.ctx.memory_mut(|m| m.request_focus(count_id));
    frames.key(app, Key::ArrowLeft);
    assert_eq!(app.page_arrangement(), Some(PageArrangement::FourColumns));
}

#[test]
fn paper_preview_never_changes_saved_note_and_paper_choices_are_undoable() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Paper samples");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = prepare(app);
    app.undo.clear();
    let original = app.note.as_ref().unwrap().paper;
    let next = original.cycle();
    let sample = control(&frames, "strip-paper", next.label());
    frames.step(app, vec![Event::PointerMoved(sample)]);
    frames.settle(app);
    assert_eq!(app.paper_preview, Some(next));
    assert_eq!(app.note.as_ref().unwrap().paper, original);
    assert!(!app.undo.undo(app.note.as_mut().unwrap()));
    frames.step(app, vec![Event::PointerMoved(pos2(10.0, 10.0))]);
    assert!(app.paper_preview.is_none());

    frames.touch(app, TouchPhase::Start, sample);
    frames.touch(app, TouchPhase::End, sample);
    assert_eq!(app.note.as_ref().unwrap().paper, next);
    let black = control(&frames, "strip-tone", "Black");
    frames.drag(app, black, black);
    assert_eq!(app.note.as_ref().unwrap().paper_tone, PaperTone::Black);
    app.set_paper_tone(PaperTone::Black);
    assert!(app.undo.undo(app.note.as_mut().unwrap()));
    assert_eq!(app.note.as_ref().unwrap().paper_tone, PaperTone::Default);
    assert!(app.undo.undo(app.note.as_mut().unwrap()));
    assert_eq!(app.note.as_ref().unwrap().paper, original);
    assert!(!app.undo.undo(app.note.as_mut().unwrap()));
    // A native stylus sends no egui button events: the chrome bridge must
    // deliver both contact and release, including inside the column popup.
    for pressed in [true, false] {
        app.tablet.set_test_pen(Some(PenSnapshot {
            pos: Some(sample),
            down: pressed,
            pressed,
            released: !pressed,
            in_proximity: true,
            ..Default::default()
        }));
        let mut raw = RawInput::default();
        app.inject_pen_pointer(&frames.ctx, &mut raw);
        assert!(raw.events.iter().any(|event| matches!(event, Event::PointerButton { pressed: value, .. } if *value == pressed)));
        frames.step(app, raw.events);
    }
    app.tablet.set_test_pen(None);
    assert_eq!(app.note.as_ref().unwrap().paper, next);
    assert!(app.undo.undo(app.note.as_mut().unwrap()));
    assert_eq!(app.note.as_ref().unwrap().paper, original);
}

#[test]
fn page_drag_escape_and_cancelled_touch_leave_pages_in_place() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Page drag cancel");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = prepare(app);
    app.arrange_pages(PageArrangement::FourColumns);
    app.camera = Camera {
        pan: vec2(60.0, 90.0),
        zoom: 0.28,
        stretch_y: None,
    };
    app.fitted_cell = None;
    frames.settle(app);
    frames.snapshot("pages-management");
    app.undo.clear();
    let paper = app.unit_screen_rect(app.canvas_rect, 0, 0);
    let from = paper.left_top() + vec2(paper.width() * 0.22, paper.height() * 0.22);
    let target = app.unit_screen_rect(app.canvas_rect, 1, 0).center();
    let original = app.note.as_ref().unwrap().pages[0].clone();
    app.start_page_drag(0, 0, from, app.canvas_rect);
    frames.step(app, vec![Event::PointerMoved(from), button(from, true)]);
    frames.step(app, vec![Event::PointerMoved(target)]);
    frames.text_center("Swap pages · release to apply · Esc to cancel");
    frames.snapshot("pages-management-drag");
    frames.key(app, Key::Escape);
    frames.step(app, vec![button(target, false)]);
    assert!(
        app.sheet_tabs_on,
        "Esc cancels the drag before closing Pages"
    );
    assert!(app.page_drag.is_none());
    assert_eq!(app.note.as_ref().unwrap().pages[0].col, original.col);
    assert!(!app.undo.undo(app.note.as_mut().unwrap()));
    app.start_page_drag(0, 0, from, app.canvas_rect);
    frames.touch(app, TouchPhase::Move, target);
    frames.touch(app, TouchPhase::Cancel, target);
    assert!(app.page_drag.is_none());
    assert_eq!(app.note.as_ref().unwrap().pages[0].col, original.col);
    assert!(
        !app.undo.undo(app.note.as_mut().unwrap()),
        "cancelled touch cannot swap pages"
    );
}

#[test]
fn page_drag_scrolls_at_edges_and_never_drops_through_the_tray() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Edge scrolling");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = prepare(app);
    app.arrange_pages(PageArrangement::FourColumns);
    app.camera = Camera {
        pan: vec2(60.0, 90.0),
        zoom: 0.28,
        stretch_y: None,
    };
    app.fitted_cell = None;
    frames.settle(app);
    app.undo.clear();
    let paper = app.unit_screen_rect(app.canvas_rect, 0, 0);
    let from = paper.left_top() + vec2(paper.width() * 0.22, paper.height() * 0.22);
    app.start_page_drag(0, 0, from, app.canvas_rect);
    frames.step(app, vec![Event::PointerMoved(from), button(from, true)]);
    let pan = app.camera.pan;
    frames.step(
        app,
        vec![Event::PointerMoved(pos2(
            app.canvas_rect.right() - 8.0,
            300.0,
        ))],
    );
    frames.settle(app);
    assert!(app.camera.pan.x < pan.x);
    let tray = control(&frames, "strip-flow", "Count");
    frames.step(app, vec![Event::PointerMoved(tray)]);
    frames.step(app, vec![button(tray, false)]);
    assert_eq!(app.note.as_ref().unwrap().pages[0].col, 0);
    assert!(!app.undo.undo(app.note.as_mut().unwrap()));
}
