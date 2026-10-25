use super::*;
use eframe::App;
mod gestures;
mod pages;

#[test]
fn pen_hides_cursor_until_real_mouse_motion_after_proximity_ends() {
    let mut fixture = Fixture::new();
    let app = &mut fixture.app;
    let ctx = Context::default();
    let frame = |app: &mut InkarchyApp, events: Vec<Event>| {
        let mut input = RawInput {
            events,
            ..Default::default()
        };
        app.raw_input_hook(&ctx, &mut input);
        let output = ctx.run(input, |ctx| app.sync_pen_cursor(ctx));
        let commands = &output.viewport_output[&ViewportId::ROOT].commands;
        (
            commands
                .iter()
                .any(|command| matches!(command, ViewportCommand::CursorVisible(false))),
            commands
                .iter()
                .any(|command| matches!(command, ViewportCommand::CursorVisible(true))),
        )
    };
    app.tablet.set_test_pen(Some(PenSnapshot {
        in_proximity: true,
        pos: Some(pos2(100.0, 100.0)),
        ..Default::default()
    }));
    assert_eq!(frame(app, vec![]), (true, false));
    // An injected pen hover is not a real mouse movement.
    let mut raw = RawInput::default();
    app.raw_input_hook(&ctx, &mut raw);
    app.inject_pen_pointer(&ctx, &mut raw);
    assert!(raw
        .events
        .iter()
        .any(|event| matches!(event, Event::PointerMoved(_))));
    assert!(!app.mouse_moved);
    // Even native pointer movement while the pen is active must not reappear later.
    assert_eq!(
        frame(app, vec![Event::PointerMoved(pos2(120.0, 100.0))]),
        (false, false)
    );
    app.tablet.set_test_pen(None);
    assert_eq!(frame(app, vec![]), (false, false));
    assert!(app.cursor_off);
    assert_eq!(
        frame(app, vec![Event::PointerMoved(pos2(150.0, 100.0))]),
        (false, true)
    );
    assert!(!app.cursor_off);
}

#[test]
fn coach_settings_remember_notebook_and_do_not_shift_the_canvas() {
    if !coach::COACH_UI_VISIBLE {
        return;
    }
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Exercise coach");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    let canvas = app.canvas_rect;
    assert!(!app.coach_enabled());
    frames.snapshot("exercise-coach-header");
    let cap = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("coach-button-rect")))
        .unwrap();
    assert!(
        !frames.output.as_ref().unwrap().shapes.iter().any(|shape| {
            matches!(&shape.shape, Shape::Rect(r)
            if r.rect.width() >= 35.0 && cap.contains_rect(r.rect) && r.fill.a() > 0)
        }),
        "graduation cap has no round button backing"
    );
    frames.drag(app, cap.center(), cap.center());
    assert!(app.coach.open);
    frames.settle(app);
    frames.snapshot("exercise-coach-settings");
    let toggle = frames.text_center("Coach this book");
    frames.drag(app, toggle, toggle);
    assert!(app.coach_enabled());
    assert_eq!(canvas, app.canvas_rect);
    let saved: crate::library::Index =
        serde_json::from_slice(&std::fs::read(app.lib.root.join("library.json")).unwrap()).unwrap();
    assert!(saved.ai.enabled_notebooks.contains(&id));
    frames.size = vec2(360.0, 480.0);
    frames.settle(app);
    frames.snapshot("exercise-coach-compact");
    frames.key(app, Key::Escape);
    frames.settle(app);
    frames.snapshot("exercise-coach-header-compact");
    frames.size = vec2(1280.0, 860.0);
    frames.settle(app);
    let bar = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("coach-meter-rect")))
        .unwrap();
    frames.drag(app, bar.center(), bar.center());
    assert!(app.coach.open, "the bar opens the coach panel too");
    frames.settle(app);
    app.close_desk();
    app.open_note(id);
    frames.settle(app);
    assert!(app.coach_enabled());
    assert!(app.coach.session.result.is_none());
    assert_eq!(canvas, app.canvas_rect);
}

#[test]
fn coach_keeps_completed_step_during_ink_but_discards_it_on_undo() {
    if !coach::COACH_UI_VISIBLE {
        return;
    }
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Exercise");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    app.coach.session.result = Some(crate::ai::Analysis {
        exercise_found: true,
        readable: true,
        complete_step: true,
        verdict: crate::ai::Verdict::OnTrack,
        statement: "Solve 2x+3=11".into(),
        recognized_step: "2x=8".into(),
        explanation: "Valid subtraction.".into(),
        region: None,
    });
    let mut stroke = InkStroke::new(Nib::Fineliner, Color32::BLACK, 2.0);
    stroke.push(InkPoint::new(pos2(100.0, 100.0), 0.7));
    app.push_snapshot();
    app.live = Some((0, stroke));
    app.finish_live();
    assert!(app.coach.session.result.is_some());
    assert!(app.undo.undo(app.note.as_mut().unwrap()));
    app.mark_dirty();
    assert!(app.coach.session.result.is_none());
}

#[test]
fn writing_coach_shows_verdict_on_meter_without_cluttering_toggle_panel() {
    if !coach::COACH_UI_VISIBLE {
        return;
    }
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Exercise");
    let app = &mut fixture.app;
    app.open_note(id);
    app.lib.index.ai.enabled_notebooks.insert(id);
    app.coach.session.reset(Some(id));
    app.coach.session.result = Some(crate::ai::Analysis {
        exercise_found: true,
        readable: true,
        complete_step: true,
        verdict: crate::ai::Verdict::OnTrack,
        statement: "Solve 2x+3=11".into(),
        recognized_step: "2x=8".into(),
        explanation: "Valid subtraction.".into(),
        region: None,
    });
    app.coach.session.evaluated = app.coach.session.revision;
    app.open_coach();
    let mut frames = Frames::new(app);
    frames.settle(app);
    assert!(
        app.coach.session.result.is_some(),
        "coach result must survive rendering"
    );
    frames.snapshot("writing-coach-verdict");
    frames.text_center("On track");
    frames.text_center("Coach this book");
}

#[test]
fn coach_panel_only_shows_toggle_and_escape_closes_without_changing_document() {
    if !coach::COACH_UI_VISIBLE {
        return;
    }
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Coach fields");
    let app = &mut fixture.app;
    app.open_note(id);
    app.tool = Tool::Brush;
    let mut frames = Frames::new(app);
    app.open_coach();
    frames.settle(app);
    frames.text_center("Coach this book");
    assert_eq!(app.tool, Tool::Brush);
    assert!(app.note.as_ref().unwrap().pages[0].strokes.is_empty());
    frames.key(app, Key::Escape);
    assert!(!app.coach.open);
    assert!(matches!(app.scene, Scene::Desk));
}

#[test]
fn object_selection_moves_resizes_and_offers_actions_with_mouse_and_touch() {
    for touch in [false, true] {
        let mut fixture = Fixture::new();
        let id = fixture.notebook("Manipulation");
        let app = &mut fixture.app;
        app.open_note(id);
        let mut text = TextBox::new(pos2(180.0, 180.0), Color32::BLACK);
        text.text = "Move me".into();
        text.size = [160.0, 100.0];
        let text_id = text.id;
        app.note.as_mut().unwrap().pages[0].texts.push(text);
        app.tool = Tool::Hand;
        let mut frames = Frames::new(app);
        frames.settle(app);
        app.camera = Camera {
            pan: vec2(30.0, 30.0),
            zoom: 1.0,
            stretch_y: None,
        };
        app.fitted_cell = None;
        frames.settle(app);
        let canvas = app.canvas_rect;
        let start = app.camera.to_screen(pos2(240.0, 225.0), canvas);
        let end = start + vec2(55.0, 35.0);
        let camera = app.camera;
        if touch {
            frames.touch(app, TouchPhase::Start, start);
            frames.touch(app, TouchPhase::Move, end);
            frames.touch(app, TouchPhase::End, end);
        } else {
            frames.drag(app, start, end);
        }
        frames.settle(app);
        assert_eq!(app.sel.len(), 1);
        assert_eq!(app.sel[0].id, text_id);
        let text = &app.note.as_ref().unwrap().pages[0].texts[0];
        assert!(text.min().distance(pos2(235.0, 215.0)) < 0.1);
        assert_eq!(
            app.camera.pan, camera.pan,
            "object dragging must not pan the page"
        );
        let original = text.clone();
        let from = app.camera.to_screen(text.rect().max, canvas);
        let to = from + vec2(80.0, 50.0);
        if touch {
            frames.touch(app, TouchPhase::Start, from);
            frames.touch(app, TouchPhase::Move, to);
            frames.touch(app, TouchPhase::End, to);
        } else {
            frames.drag(app, from, to);
        }
        frames.settle(app);
        let text = &app.note.as_ref().unwrap().pages[0].texts[0];
        assert!((text.size[0] - 240.0).abs() < 0.1);
        assert!((text.size_pt - original.size_pt * 1.5).abs() < 0.1);
        assert_eq!(text.pos, original.pos);
        assert!(app.undo.undo(app.note.as_mut().unwrap()));
        assert_eq!(
            app.note.as_ref().unwrap().pages[0].texts[0].size,
            original.size
        );
        frames.settle(app);
        frames.snapshot(if touch {
            "selection-touch"
        } else {
            "selection-mouse"
        });
        assert!(frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("selection-actions")))
            .is_none());
        assert_eq!(app.sel.len(), 1);
        app.duplicate_selection();
        assert_eq!(app.note.as_ref().unwrap().pages[0].texts.len(), 2);
        app.delete_selection();
        assert_eq!(app.note.as_ref().unwrap().pages[0].texts.len(), 1);
        assert_eq!(app.note.as_ref().unwrap().pages[0].texts[0].id, text_id);
    }
}

#[test]
fn selection_scale_preserves_group_offsets_across_linked_pages() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Group");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut stroke = InkStroke::new(Nib::Fineliner, Color32::BLACK, 2.0);
    stroke.push(InkPoint::new(pos2(100.0, 100.0), 0.5));
    stroke.push(InkPoint::new(pos2(150.0, 150.0), 0.8));
    stroke.tessellate();
    let stroke_id = stroke.id;
    let image_id = Uuid::new_v4();
    let note = app.note.as_mut().unwrap();
    note.pages[0].strokes.push(stroke);
    let mut page = crate::document::Page::at(1, 0);
    page.images.push(ImageObj {
        id: image_id,
        pos: [10.0, 100.0],
        size: [80.0, 60.0],
        file: "fixture.png".into(),
    });
    note.pages.push(page);
    app.sel = vec![
        Sel {
            page: 0,
            id: stroke_id,
            kind: SelKind::Stroke,
        },
        Sel {
            page: 1,
            id: image_id,
            kind: SelKind::Image,
        },
    ];
    app.push_snapshot();
    app.scale_selection(pos2(100.0, 100.0), 2.0);
    let note = app.note.as_ref().unwrap();
    assert_eq!(note.pages[0].strokes[0].points[1].pos(), pos2(200.0, 200.0));
    assert!(note.pages[0].strokes[0].mesh.is_none());
    assert_eq!(note.pages[0].strokes[0].width, 4.0);
    assert_eq!(note.pages[1].images[0].size, [160.0, 120.0]);
    assert_eq!(note.pages[1].images[0].pos, [PAGE_W - 80.0, 100.0]);
    assert!(app.undo.undo(app.note.as_mut().unwrap()));
    assert_eq!(
        app.note.as_ref().unwrap().pages[1].images[0].pos,
        [10.0, 100.0]
    );
}

struct Fixture {
    app: InkarchyApp,
    root: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("inkarchy-ui-{}", Uuid::new_v4()));
        let lib = Library::fresh(root.clone());
        Self {
            app: InkarchyApp::with_library(Look::load(), lib),
            root,
        }
    }

    fn notebook(&mut self, name: &str) -> Uuid {
        let note = Note::blank(name, 0);
        let id = note.id;
        self.app.lib.insert_new(&note).expect("save notebook");
        id
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.root.exists() {
            std::fs::remove_dir_all(&self.root).expect("remove isolated test library");
        }
    }
}

fn shelf_ids(app: &InkarchyApp) -> Vec<Uuid> {
    app.shelf_slots.iter().map(|(id, _)| *id).collect()
}

#[test]
fn shelf_search_still_matches_the_title() {
    let mut fixture = Fixture::new();
    let alpha = fixture.notebook("Alpha");
    let _beta = fixture.notebook("Beta");
    fixture.app.scene = Scene::Shelf {
        query: "beta".into(),
    };
    let mut frames = Frames::new(&fixture.app);
    frames.settle(&mut fixture.app);
    let ids = shelf_ids(&fixture.app);
    assert!(ids.contains(&_beta));
    assert!(!ids.contains(&alpha));
}

#[test]
fn shelf_search_matches_typed_text_inside_a_notebook() {
    let mut fixture = Fixture::new();
    let hidden = fixture.notebook("Alpha");
    let other = fixture.notebook("Beta");
    let mut note = fixture.app.lib.load_note(hidden).unwrap();
    let mut text = TextBox::new(pos2(24.0, 24.0), Color32::BLACK);
    text.text = "Fourier series on the board".into();
    note.pages[0].texts.push(text);
    note.touch();
    fixture.app.lib.save_note(&note).expect("save body");
    fixture.app.scene = Scene::Shelf {
        query: "fourier".into(),
    };
    let mut frames = Frames::new(&fixture.app);
    frames.settle(&mut fixture.app);
    let ids = shelf_ids(&fixture.app);
    assert!(ids.contains(&hidden), "ids {ids:?}");
    assert!(!ids.contains(&other));
    let snippet = fixture.app.shelf_hit_snippet(hidden, "fourier");
    assert!(
        snippet
            .as_deref()
            .is_some_and(|s| s.to_lowercase().contains("fourier")),
        "snippet {snippet:?}"
    );
    let base = DOS_PAD + PAPER_PEEK + DOS_H + TITLE_ROW;
    assert!(
        fixture
            .app
            .shelf_grid
            .iter()
            .any(|cell| cell.height() >= base + SNIPPET_ROW - 0.1),
        "search cells must leave a line under the title"
    );
    let mut title = None;
    let mut snip = None;
    for shape in &frames.output.as_ref().unwrap().shapes {
        let Shape::Text(text) = &shape.shape else {
            continue;
        };
        let rect = Rect::from_min_size(text.pos, text.galley.size());
        if text.galley.text() == "Alpha" {
            title = Some(rect);
        } else if text.galley.text().contains("fourier") && text.pos.y > 80.0 {
            snip = Some(rect);
        }
    }
    let title = title.expect("title");
    let snip = snip.expect("snippet under title");
    assert!(
        snip.top() >= title.bottom() - 0.5,
        "snippet {snip:?} overlaps title {title:?}"
    );
}

#[test]
fn shelf_search_matches_pdf_unicode() {
    let mut fixture = Fixture::new();
    let mut note = Note::blank("Lecture", 0);
    let id = note.id;
    note.pages[0].background = Some(PageBackground {
        file: "source.pdf".into(),
        pdf_page: Some(0),
        source_size: Some([200.0, 100.0]),
    });
    fixture.app.lib.insert_new(&note).expect("save notebook");
    let dir = fixture.app.lib.note_dir(id);
    std::fs::create_dir_all(dir.join("media")).unwrap();
    std::fs::write(dir.join("source.pdf"), crate::pdf::text_pdf("Fourier")).unwrap();
    let decoy = fixture.notebook("Sketch");
    fixture.app.scene = Scene::Shelf {
        query: "fourier".into(),
    };
    let mut frames = Frames::new(&fixture.app);
    let mut found = false;
    for _ in 0..120 {
        frames.step(&mut fixture.app, vec![]);
        if fixture.app.shelf_hit_matches(id, "fourier") {
            found = true;
            break;
        }
    }
    assert!(found, "PDF unicode should reach the shelf search");
    let ids = shelf_ids(&fixture.app);
    assert!(ids.contains(&id), "ids {ids:?}");
    assert!(!ids.contains(&decoy));
}

#[test]
fn empty_shelf_query_keeps_the_slot_grid() {
    let mut fixture = Fixture::new();
    fixture.notebook("One");
    let mut frames = Frames::new(&fixture.app);
    frames.settle(&mut fixture.app);
    assert_eq!(fixture.app.shelf_grid.len(), SHELF_SLOTS as usize);
}

#[test]
fn many_pdf_pages_do_not_block_the_first_frames() {
    let mut fixture = Fixture::new();
    let pages: Vec<_> = (0..40)
        .map(|_| crate::pdf::MarkedPage {
            width: 200.0,
            height: 280.0,
            rotate: 0,
            marker: "P",
        })
        .collect();
    let bytes = crate::pdf::marked_pdf(&pages);
    let mut note = Note::blank("Talk", 0);
    note.paper = PaperKind::Blank;
    note.sheet_join = SheetJoin::Separate;
    let id = note.id;
    note.pages = (0..40)
        .map(|i| {
            let mut page = crate::document::Page::at(0, i);
            page.background = Some(PageBackground {
                file: "source.pdf".into(),
                pdf_page: Some(i as u32),
                source_size: Some([200.0, 280.0]),
            });
            page
        })
        .collect();
    fixture.app.lib.insert_new(&note).expect("save notebook");
    let dir = fixture.app.lib.note_dir(id);
    std::fs::create_dir_all(dir.join("media")).unwrap();
    std::fs::write(dir.join("source.pdf"), bytes).unwrap();
    fixture.app.open_note(id);
    assert_eq!(fixture.app.land_page, Some(0));
    let started = std::time::Instant::now();
    let mut frames = Frames::new(&fixture.app);
    frames.settle(&mut fixture.app);
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_millis(1500),
        "opening 40 PDF pages blocked the UI for {elapsed:?}"
    );
    assert!(matches!(fixture.app.scene, Scene::Desk));
    assert_eq!(fixture.app.land_page, None);
    assert_eq!(fixture.app.fitted_cell, Some((0, 0)));
    let (pw, ph) = fixture.app.note.as_ref().unwrap().page_size();
    let origin = fixture.app.origin_of(0);
    let mut fitted = Camera::default();
    fitted.fit_page(fixture.app.canvas_rect, origin, pw, ph);
    assert!((fixture.app.camera.zoom - fitted.zoom).abs() < 0.001);
}

#[test]
fn pdf_page_chip_steps_once_and_accelerates_when_held() {
    let mut fixture = Fixture::new();
    let mut note = Note::blank("Pages", 0);
    note.sheet_join = SheetJoin::Separate;
    note.pages = (0..30)
        .map(|i| {
            let mut page = crate::document::Page::at(0, i);
            page.background = Some(PageBackground {
                file: "source.pdf".into(),
                pdf_page: Some(i as u32),
                source_size: None,
            });
            page
        })
        .collect();
    let id = note.id;
    fixture.app.lib.insert_new(&note).unwrap();
    fixture.app.open_note(id);
    let app = &mut fixture.app;
    let mut frames = Frames::new(app);
    frames.settle(app);
    let chip = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("pdf-page-number-rect")));
    let chip = chip.expect("PDF page chip is visible");
    frames.drag(app, chip.center(), chip.center());
    frames.settle(app);
    frames.snapshot("pdf-page-picker");
    let next = frames.text_center("+");
    frames.drag(app, next, next);
    frames.settle(app);
    assert_eq!(app.pdf_page_target, 2);
    assert_eq!(app.page_in_view(app.canvas_rect), 1);
    frames.step(app, vec![Event::PointerMoved(next)]);
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: next,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
    );
    assert_eq!(app.pdf_page_target, 3, "a page turns when the finger lands");
    for _ in 0..150 {
        frames.step(app, vec![]);
    }
    assert!(
        app.pdf_page_target >= 12,
        "holding accelerates the page turns"
    );
    let target = app.pdf_page_target;
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: next,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    frames.settle(app);
    assert_eq!(
        app.pdf_page_target, target,
        "release does not turn an extra page"
    );
    assert_eq!(app.page_in_view(app.canvas_rect), target - 1);
    assert_eq!(
        app.page_cells()[1],
        (0, 1),
        "holding the stepper must not move pages"
    );
}

#[test]
fn word_search_button_stays_in_the_header_without_a_pdf() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Plain search");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    let icon = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("pdf-search-button-rect")))
        .expect("word search stays in the header for every notebook");
    assert!(icon.max.y <= 52.0 && icon.min.y >= 0.0);
    frames.drag(app, icon.center(), icon.center());
    frames.settle(app);
    assert!(app.pdf_search_open);
    assert!(frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("pdf-search-field-rect")))
        .is_some());
}

#[test]
fn pdf_search_field_opens_in_header_and_stays_open_while_editing() {
    let mut fixture = Fixture::new();
    let mut note = Note::blank("Search PDF", 0);
    note.pages.push(crate::document::Page::at(0, 1));
    note.pages[0].background = Some(PageBackground {
        file: "source.pdf".into(),
        pdf_page: Some(0),
        source_size: None,
    });
    note.pages[1].background = Some(PageBackground {
        file: "source.pdf".into(),
        pdf_page: Some(1),
        source_size: None,
    });
    let id = note.id;
    fixture.app.lib.insert_new(&note).unwrap();
    fixture.app.open_note(id);
    let app = &mut fixture.app;
    app.pdf_text
        .insert(format!("{id}:source.pdf:0"), "formula formula".into());
    app.pdf_text
        .insert(format!("{id}:source.pdf:1"), "a formula".into());
    let mut frames = Frames::new(app);
    frames.settle(app);
    frames.step_with_modifiers(
        app,
        vec![Event::Key {
            key: Key::F,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        }],
        Modifiers::COMMAND,
    );
    let field = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("pdf-search-field-rect")))
        .unwrap();
    assert!(
        field.width() > 100.0,
        "Ctrl+F opens the search bubble: {field:?}"
    );
    assert!(
        frames.ctx.memory(|memory| memory.focused()).is_some(),
        "Ctrl+F focuses search input"
    );
    frames.key(app, Key::Escape);
    frames.settle(app);
    let icon = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("pdf-search-button-rect")))
        .unwrap();
    frames.drag(app, icon.center(), icon.center());
    frames.settle(app);
    let field = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("pdf-search-field-rect")))
        .unwrap();
    assert!(
        field.right() <= icon.left() && field.bottom() <= 52.0,
        "field {field:?} icon {icon:?}"
    );
    frames.drag(app, field.center(), field.center());
    let tool = app.tool;
    frames.step(app, vec![Event::Text("formula".into())]);
    assert_eq!(app.pdf_search_query, "formula");
    frames.text_center("1/3");
    assert_eq!(app.page_in_view(app.canvas_rect), 0);
    let next = frames.text_center("↓");
    frames.drag(app, next, next);
    frames.settle(app);
    frames.text_center("2/3");
    frames.drag(app, next, next);
    frames.settle(app);
    frames.text_center("3/3");
    assert_eq!(app.page_in_view(app.canvas_rect), 1);
    frames.drag(app, next, next);
    frames.settle(app);
    frames.text_center("1/3");
    assert_eq!(app.page_in_view(app.canvas_rect), 0);
    frames.key(app, Key::Enter);
    frames.settle(app);
    frames.text_center("2/3");
    frames.key(app, Key::L);
    assert_eq!(
        app.tool, tool,
        "typing a search must not activate a drawing tool"
    );
    assert!(app.pdf_search_open);
    frames.key(app, Key::Escape);
    frames.settle(app);
    assert!(!app.pdf_search_open);
    frames.size = vec2(360.0, 640.0);
    frames.settle(app);
    let icon = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("pdf-search-button-rect")))
        .unwrap();
    frames.drag(app, icon.center(), icon.center());
    frames.settle(app);
    let field = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("pdf-search-field-rect")))
        .unwrap();
    assert!(
        field.width() >= 60.0 && field.left() >= 0.0 && field.right() <= icon.left(),
        "compact header: field {field:?} icon {icon:?}"
    );
}

#[test]
fn pdf_detail_replaces_preview_without_late_results_downgrading_it() {
    let mut fixture = Fixture::new();
    let ctx = Context::default();
    let key = "pdf:page:0".to_string();
    let raster = |side: usize, scale| crate::pdf::RasterImage {
        image: ColorImage::new([side, side], vec![Color32::WHITE; side * side]),
        scale,
    };
    fixture.app.bg_hot.insert(key.clone());
    fixture
        .app
        .store_background(&ctx, key.clone(), raster(10, 0.5));
    fixture
        .app
        .store_background(&ctx, key.clone(), raster(60, 3.0));
    fixture
        .app
        .store_background(&ctx, key.clone(), raster(10, 0.5));
    assert_eq!(fixture.app.bg_textures[&key].size(), [60, 60]);
    assert_eq!(fixture.app.bg_tex_scale[&key], 3.0);
    assert_eq!(fixture.app.bg_order.len(), 1);
    assert_eq!(fixture.app.bg_bytes, 60 * 60 * 4);
}

#[test]
fn pdf_background_loads_asynchronously_and_rerenders_at_zoom_resolution() {
    let mut fixture = Fixture::new();
    let note = Note::blank("Zoom", 0);
    let id = note.id;
    fixture.app.lib.insert_new(&note).unwrap();
    let path = fixture.app.lib.note_dir(id).join("source.pdf");
    std::fs::write(&path, crate::pdf::text_pdf("Sharp zoom")).unwrap();
    let bg = PageBackground {
        file: "source.pdf".into(),
        pdf_page: Some(0),
        source_size: Some([200.0, 100.0]),
    };
    let ctx = Context::default();
    let key = format!("{id}:source.pdf:0");
    for (screen, wanted) in [(vec2(200.0, 100.0), 1.0), (vec2(800.0, 400.0), 4.0)] {
        let started = Instant::now();
        loop {
            fixture.app.ingest_pdf_rasters(&ctx);
            fixture
                .app
                .background_tex(&ctx, id, &bg, Rect::from_min_size(Pos2::ZERO, screen), 1.0);
            if fixture
                .app
                .bg_tex_scale
                .get(&key)
                .is_some_and(|scale| *scale >= wanted)
            {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "PDF detail never arrived"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    assert_eq!(fixture.app.bg_textures[&key].size(), [800, 400]);
    assert!(fixture.app.bg_pending.is_empty());
}

struct Frames {
    ctx: Context,
    time: f64,
    textures: HashMap<TextureId, ColorImage>,
    output: Option<FullOutput>,
    size: Vec2,
}

impl Frames {
    fn new(app: &InkarchyApp) -> Self {
        let ctx = Context::default();
        crate::fonts::install(&ctx);
        app.look.apply(&ctx);
        Self {
            ctx,
            time: 0.0,
            textures: HashMap::new(),
            output: None,
            size: vec2(1280.0, 860.0),
        }
    }

    fn step(&mut self, app: &mut InkarchyApp, events: Vec<Event>) {
        self.step_with_modifiers(app, events, Modifiers::NONE);
    }

    fn step_with_modifiers(
        &mut self,
        app: &mut InkarchyApp,
        events: Vec<Event>,
        modifiers: Modifiers,
    ) {
        self.time += 1.0 / 60.0;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            modifiers,
            events,
            ..Default::default()
        };
        let output = self.ctx.run(input, |ctx| {
            app.shortcuts(ctx);
            app.ingest_pdf_rasters(ctx);
            match app.scene {
                Scene::Shelf { .. } => app.ui_shelf(ctx),
                Scene::Desk => app.ui_desk(ctx),
            }
        });
        for (id, delta) in &output.textures_delta.set {
            let ImageData::Color(image) = &delta.image;
            if let Some([x, y]) = delta.pos {
                let target = self.textures.get_mut(id).expect("texture allocated");
                for row in 0..image.height() {
                    let start = (y + row) * target.width() + x;
                    target.pixels[start..start + image.width()].copy_from_slice(
                        &image.pixels[row * image.width()..(row + 1) * image.width()],
                    );
                }
            } else {
                self.textures.insert(*id, (**image).clone());
            }
        }
        self.output = Some(output);
    }

    fn settle(&mut self, app: &mut InkarchyApp) {
        for _ in 0..20 {
            self.step(app, vec![]);
        }
    }

    fn text_center(&self, label: &str) -> Pos2 {
        self.output
            .as_ref()
            .unwrap()
            .shapes
            .iter()
            .find_map(|shape| {
                if let Shape::Text(text) = &shape.shape {
                    if text.galley.text() == label {
                        return Some(text.pos + text.galley.rect.center().to_vec2());
                    }
                }
                None
            })
            .unwrap_or_else(|| panic!("visible label: {label}"))
    }

    fn drag(&mut self, app: &mut InkarchyApp, from: Pos2, to: Pos2) {
        self.step(app, vec![Event::PointerMoved(from)]);
        self.step(
            app,
            vec![Event::PointerButton {
                pos: from,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }],
        );
        self.step(app, vec![Event::PointerMoved(to)]);
        self.settle(app);
        self.step(
            app,
            vec![Event::PointerButton {
                pos: to,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
        );
    }

    fn key(&mut self, app: &mut InkarchyApp, key: Key) {
        for pressed in [true, false] {
            self.step(
                app,
                vec![Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
            );
        }
    }

    fn touch(&mut self, app: &mut InkarchyApp, phase: TouchPhase, pos: Pos2) {
        let mut events = vec![
            Event::Touch {
                device_id: TouchDeviceId(1),
                id: TouchId(1),
                phase,
                pos,
                force: None,
            },
            Event::PointerMoved(pos),
        ];
        if phase != TouchPhase::Move {
            events.push(Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: phase == TouchPhase::Start,
                modifiers: Modifiers::NONE,
            });
        }
        if matches!(phase, TouchPhase::End | TouchPhase::Cancel) {
            events.push(Event::PointerGone);
        }
        self.step(app, events);
    }

    // Optional CPU snapshots of the actual egui meshes, without opening a desktop window.
    fn snapshot(&self, name: &str) {
        let Some(directory) = std::env::var_os("INKARCHY_TEST_ARTIFACTS") else {
            return;
        };
        let output = self.output.as_ref().unwrap();
        let scale = output.pixels_per_point;
        let mut image =
            image::RgbaImage::new((self.size.x * scale) as u32, (self.size.y * scale) as u32);
        for primitive in self.ctx.tessellate(output.shapes.clone(), scale) {
            let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                continue;
            };
            let texture = &self.textures[&mesh.texture_id];
            let clip = primitive.clip_rect;
            for indices in mesh.indices.chunks_exact(3) {
                let v = [
                    mesh.vertices[indices[0] as usize],
                    mesh.vertices[indices[1] as usize],
                    mesh.vertices[indices[2] as usize],
                ];
                let points = v.map(|v| (v.pos.to_vec2() * scale).to_pos2());
                let cross = |a: Vec2, b: Vec2| a.x * b.y - a.y * b.x;
                let area = cross(points[1] - points[0], points[2] - points[0]);
                if area.abs() < 0.00001 {
                    continue;
                }
                let bounds = Rect::from_points(&points).intersect(Rect::from_min_max(
                    (clip.min.to_vec2() * scale).to_pos2(),
                    (clip.max.to_vec2() * scale).to_pos2(),
                ));
                let colors = v.map(|v| v.color.to_array());
                for y in bounds.min.y.max(0.0).floor() as u32
                    ..(bounds.max.y.ceil() as u32).min(image.height())
                {
                    for x in bounds.min.x.max(0.0).floor() as u32
                        ..(bounds.max.x.ceil() as u32).min(image.width())
                    {
                        let point = pos2(x as f32 + 0.5, y as f32 + 0.5);
                        let a = cross(points[1] - point, points[2] - point) / area;
                        let b = cross(points[2] - point, points[0] - point) / area;
                        let c = 1.0 - a - b;
                        if a < 0.0 || b < 0.0 || c < 0.0 {
                            continue;
                        }
                        let uv =
                            v[0].uv.to_vec2() * a + v[1].uv.to_vec2() * b + v[2].uv.to_vec2() * c;
                        let tx =
                            ((uv.x * texture.width() as f32) as usize).min(texture.width() - 1);
                        let ty =
                            ((uv.y * texture.height() as f32) as usize).min(texture.height() - 1);
                        let texel = texture.pixels[ty * texture.width() + tx].to_array();
                        let source: [f32; 4] = std::array::from_fn(|i| {
                            (colors[0][i] as f32 * a
                                + colors[1][i] as f32 * b
                                + colors[2][i] as f32 * c)
                                * texel[i] as f32
                                / 255.0
                        });
                        let target = image.get_pixel_mut(x, y);
                        for i in 0..4 {
                            target.0[i] = (source[i]
                                + target.0[i] as f32 * (1.0 - source[3] / 255.0))
                                .round()
                                .clamp(0.0, 255.0) as u8;
                        }
                    }
                }
            }
        }
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        image.save(directory.join(format!("{name}.png"))).unwrap();
    }
}

#[test]
fn fitted_canvas_tracks_same_page_through_resizes() {
    let mut fixture = Fixture::new();
    let app = &mut fixture.app;
    let mut note = Note::blank("Fit", 0);
    note.pages.push(crate::document::Page::at(1, 0));
    app.note = Some(note);
    app.need_fit = false;
    app.canvas_rect = Rect::from_min_size(pos2(0.0, 64.0), vec2(1000.0, 800.0));
    app.camera
        .fit_page(app.canvas_rect, app.origin_of(1), app.pw(), app.ph());
    app.fit_to_screen();
    for size in [vec2(800.0, 480.0), vec2(1800.0, 900.0), vec2(820.0, 1200.0)] {
        let rect = Rect::from_min_size(pos2(18.0, 80.0), size);
        assert!(app.update_canvas_rect(rect));
        assert_eq!(app.fitted_cell, Some((1, 0)));
        assert_eq!(app.page_in_view(rect), 1);
        assert_eq!(app.note.as_ref().unwrap().page_size(), (PAGE_W, PAGE_H));
        assert_uniform_page_fit(app, app.origin_of(1), PAGE_W, PAGE_H);
    }
}

#[test]
fn manual_canvas_keeps_focus_through_minimize_and_reopen() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Manual");
    let app = &mut fixture.app;
    app.open_note(id);
    app.update_canvas_rect(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0)));
    app.fit_to_screen();
    app.update_canvas_rect(app.canvas_rect);
    app.set_zoom_level(2.0);
    assert_eq!(app.fitted_cell, None);
    let zoom = app.camera.zoom;
    let focus = app
        .camera
        .to_paper(app.canvas_rect.center(), app.canvas_rect);
    for size in [
        vec2(800.0, 1000.0),
        Vec2::ZERO,
        vec2(1300.0, 2.0),
        vec2(1600.0, 700.0),
    ] {
        let previous = app.canvas_rect;
        let rect = Rect::from_min_size(pos2(20.0, 80.0), size);
        let valid = app.update_canvas_rect(rect);
        assert_eq!(valid, Camera::valid_viewport(rect));
        if !valid {
            assert_eq!(app.canvas_rect, previous);
        }
        assert_eq!(app.camera.zoom, zoom);
        assert!(
            app.camera
                .to_paper(app.canvas_rect.center(), app.canvas_rect)
                .distance(focus)
                < 0.001
        );
    }
    app.close_desk();
    app.open_note(id);
    assert_eq!(app.canvas_rect, Rect::ZERO);
    assert_eq!(app.fitted_cell, None);
    assert_eq!(app.land_page, Some(0));
}

#[test]
fn manual_scroll_leaves_fitted_mode() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Scroll");
    let app = &mut fixture.app;
    app.open_note(id);
    app.fit_to_screen();
    let mut frames = Frames::new(app);
    frames.settle(app);
    assert_eq!(app.fitted_cell, Some((0, 0)));
    frames.snapshot("page-fit-wide");
    frames.size = vec2(820.0, 620.0);
    frames.settle(app);
    assert_eq!(app.fitted_cell, Some((0, 0)));
    frames.snapshot("page-fit-small");
    let center = app.canvas_rect.center();
    frames.step(app, vec![Event::PointerMoved(center)]);
    frames.step(
        app,
        vec![Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: vec2(0.0, -30.0),
            modifiers: Modifiers::NONE,
        }],
    );
    assert_eq!(app.fitted_cell, None);
    let zoom = app.camera.zoom;
    let focus = app
        .camera
        .to_paper(app.canvas_rect.center(), app.canvas_rect);
    frames.size = vec2(1000.0, 1000.0);
    frames.settle(app);
    assert_eq!(app.camera.zoom, zoom);
    assert!(
        app.camera
            .to_paper(app.canvas_rect.center(), app.canvas_rect)
            .distance(focus)
            < 0.001
    );
    frames.snapshot("page-manual-tall");
}

#[test]
fn trackpad_two_finger_point_scroll_pans_without_zoom() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Pad pan");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    let center = app.canvas_rect.center();
    let zoom = app.camera.zoom;
    let pan = app.camera.pan;
    frames.step(
        app,
        vec![
            Event::PointerMoved(center),
            Event::MouseWheel {
                unit: MouseWheelUnit::Point,
                delta: vec2(0.0, 80.0),
                modifiers: Modifiers::NONE,
            },
        ],
    );
    assert_eq!(app.camera.zoom, zoom);
    assert_ne!(app.camera.pan, pan);
    assert!((app.camera.pan.y - pan.y).abs() > 1.0);
}

#[test]
fn responsive_note_controls_do_not_overlap_through_resizes() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("A very long notebook title that must stay inside its header");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    for edge in [
        DockEdge::Bottom,
        DockEdge::Left,
        DockEdge::Top,
        DockEdge::Right,
    ] {
        app.dock_edge = edge;
        app.dock_drag_edge = None;
        app.dock_float = None;
        app.dock_slide = None;
        app.dock_fold = 0.0;
        for size in [
            vec2(1280.0, 860.0),
            vec2(320.0, 280.0),
            vec2(360.0, 640.0),
            vec2(1024.0, 320.0),
            vec2(760.0, 560.0),
            vec2(600.0, 400.0),
        ] {
            frames.size = size;
            let mut palette_rect: Option<Rect> = None;
            for accessory in 0..5 {
                app.set_ink_panel(accessory != 0);
                app.ink_panel_view = match accessory {
                    2 => InkPanelView::Nib,
                    3 => InkPanelView::Tools,
                    _ => InkPanelView::Colors,
                };
                app.color_mixer_open = accessory == 4;
                frames.settle(app);
                let panel = frames.ctx.data(|d| d.get_temp::<Rect>(Id::new("tin-rect")));
                if accessory == 1 {
                    palette_rect = panel;
                } else if accessory == 4 {
                    let palette = palette_rect.unwrap();
                    let mixer = panel.unwrap();
                    if edge.vertical() {
                        assert!(
                            (palette.height() - mixer.height()).abs() < 1.0,
                            "{edge:?} {size:?}: palette {palette:?} and mixer {mixer:?} keep the same height"
                        );
                    } else {
                        assert!(
                            (palette.height() - mixer.height()).abs() < 1.0,
                            "{edge:?} {size:?}: horizontal strips share height {palette:?} vs {mixer:?}"
                        );
                    }
                    let dock = frames
                        .ctx
                        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
                        .unwrap();
                    let palette_against = match edge {
                        DockEdge::Bottom => (dock.top() - palette.bottom()).abs() < 11.0,
                        DockEdge::Top => (palette.top() - dock.bottom()).abs() < 11.0,
                        DockEdge::Left => (palette.left() - dock.right()).abs() < 11.0,
                        DockEdge::Right => (dock.left() - palette.right()).abs() < 11.0,
                    };
                    let mixer_against = match edge {
                        DockEdge::Bottom => (dock.top() - mixer.bottom()).abs() < 11.0,
                        DockEdge::Top => (mixer.top() - dock.bottom()).abs() < 11.0,
                        DockEdge::Left => (mixer.left() - dock.right()).abs() < 11.0,
                        DockEdge::Right => (dock.left() - mixer.right()).abs() < 11.0,
                    };
                    assert!(
                        palette_against && mixer_against,
                        "{edge:?} {size:?}: colors {palette:?} and mixer {mixer:?} stay against the dock {dock:?}"
                    );
                }
                let screen = Rect::from_min_size(Pos2::ZERO, size);
                let controls: Vec<_> = ["top-rect", "dock-rect", "tin-rect"]
                    .into_iter()
                    .filter_map(|key| {
                        frames
                            .ctx
                            .data(|d| d.get_temp::<Rect>(Id::new(key)))
                            .map(|r| (key, r))
                    })
                    .collect();
                for (i, (name, rect)) in controls.iter().enumerate() {
                    assert!(
                        screen.expand(0.5).contains_rect(*rect),
                        "{edge:?} {size:?} {accessory}: {name} {rect:?} outside screen"
                    );
                    for (other, other_rect) in &controls[i + 1..] {
                        // The transparent header band and a top dock share the same strip on purpose.
                        if edge == DockEdge::Top
                            && ((*name == "top-rect" && *other == "dock-rect")
                                || (*name == "dock-rect" && *other == "top-rect"))
                        {
                            continue;
                        }
                        assert!(!rect.shrink(0.5).intersects(*other_rect), "{edge:?} {size:?} {accessory}: {name} {rect:?} overlaps {other} {other_rect:?}");
                    }
                }
                let header_faces: Vec<_> = frames
                    .output
                    .as_ref()
                    .unwrap()
                    .shapes
                    .iter()
                    .filter_map(|shape| {
                        if let Shape::Rect(r) = &shape.shape {
                            if r.fill.a() == 255
                                && (r.rect.height() - 40.0).abs() < 0.1
                                && r.rect.top() < 52.0
                            {
                                return Some(r.rect);
                            }
                        }
                        None
                    })
                    .collect();
                for (i, face) in header_faces.iter().enumerate() {
                    assert!(screen.contains_rect(*face));
                    for other in &header_faces[i + 1..] {
                        assert!(!face.intersects(*other), "header buttons overlap");
                    }
                }
                let [menu, cap, meter] = [
                    "notebook-menu-button-rect",
                    "coach-button-rect",
                    "coach-meter-rect",
                ]
                .map(|key| {
                    frames
                        .ctx
                        .data(|d| d.get_temp::<Rect>(Id::new(key)))
                        .unwrap()
                });
                if edge == DockEdge::Bottom && size.x == 1024.0 && accessory == 1 {
                    frames.snapshot("exercise-coach-header-1024");
                }
                for (name, rect) in [("menu", menu), ("meter", meter)] {
                    assert!(
                        screen.contains_rect(rect),
                        "header control outside {size:?}: {name} {rect:?}"
                    );
                }
                if screen.intersects(cap) {
                    assert!(
                        screen.contains_rect(cap),
                        "coach badge remains on-screen {edge:?} {size:?}: {cap:?}"
                    );
                }
                if let Some(well) = frames
                    .ctx
                    .data(|d| d.get_temp::<Rect>(Id::new("ink-well-rect")))
                {
                    if screen.contains(well.center()) && screen.contains(cap.center()) {
                        assert!(screen.contains(well.center()));
                    }
                }
                if size.x >= 760.0 {
                    assert!(
                        meter
                            .center()
                            .distance(pos2(size.x * 0.5, meter.center().y))
                            < 1.0
                    );
                    assert!(
                        meter.right() < menu.left(),
                        "center bar stays left of the right icon cluster"
                    );
                    assert!(
                        meter.width() >= 80.0,
                        "wide screens keep a visible centered evaluation bar: {meter:?}"
                    );
                }
                if edge == DockEdge::Bottom && size.x == 320.0 && accessory == 1 {
                    frames.snapshot("responsive-note-small");
                }
            }
        }
    }
}

#[test]
fn pages_mode_adds_a_sheet_and_toggle_hides_the_pluses() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Pages");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    assert!(
        app.sheet_tabs(app.canvas_rect).is_empty(),
        "tabs stay hidden while writing"
    );
    app.camera = Camera {
        pan: vec2(180.0, 180.0),
        zoom: 0.35,
        stretch_y: None,
    };
    app.fitted_cell = None;
    frames.settle(app);
    let pages = app.note.as_ref().unwrap().pages.len();
    let more = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("notebook-menu-button-rect")))
        .unwrap()
        .center();
    frames.drag(app, more, more);
    frames.settle(app);
    let edit = frames.text_center("Edit pages");
    frames.drag(app, edit, edit);
    frames.settle(app);
    assert!(app.sheet_tabs_on);
    let canvas = app.canvas_rect;
    let tabs = app.sheet_tabs(canvas);
    assert!(!tabs.is_empty(), "Pages mode shows the + tabs");
    let hit = tabs
        .iter()
        .map(|(_, _, rect, ..)| rect.center())
        .find(|pos| canvas.contains(*pos))
        .expect("a + tab sits on the canvas");
    frames.drag(app, hit, hit);
    frames.settle(app);
    assert_eq!(
        app.note.as_ref().unwrap().pages.len(),
        pages + 1,
        "a + tab adds a page while managing"
    );
    frames.key(app, Key::Escape);
    frames.settle(app);
    assert!(!app.sheet_tabs_on);
    assert!(
        app.sheet_tabs(app.canvas_rect).is_empty(),
        "leaving Pages mode hides the + tabs"
    );
}

#[test]
fn pages_mode_arranges_six_pages_without_losing_the_visible_page_or_its_content() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Six pages");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut pages: Vec<_> = (0..6)
        .map(|row| crate::document::Page::at(0, row))
        .collect();
    let mut label = TextBox::new(pos2(80.0, 120.0), Color32::BLACK);
    label.text = "Keep with page five".into();
    let label_id = label.id;
    pages[4].texts.push(label);
    app.note.as_mut().unwrap().pages = pages;
    app.note.as_mut().unwrap().sheet_join = SheetJoin::Separate;
    let mut frames = Frames::new(app);
    frames.settle(app);
    let (pw, ph) = app.note.as_ref().unwrap().page_size();
    app.camera
        .fit_page(app.canvas_rect, app.origin_of(4), pw, ph);
    app.fitted_cell = Some((0, 4));
    app.enter_pages_mode();
    frames.settle(app);
    let more = frames
        .ctx
        .data(|data| data.get_temp::<Rect>(Id::new(("strip-flow", "More"))))
        .expect("the per-row stepper appears in Pages mode");
    frames.drag(app, more.center(), more.center());
    frames.settle(app);
    let note = app.note.as_ref().unwrap();
    let cells: Vec<_> = note.pages.iter().map(|page| (page.col, page.row)).collect();
    assert_eq!(cells, vec![(0, 0), (1, 0), (0, 1), (1, 1), (0, 2), (1, 2)]);
    assert_eq!(note.pages[4].texts[0].id, label_id);
    assert_eq!(app.fitted_cell, Some((0, 2)));
    assert_eq!(app.page_in_view(app.canvas_rect), 4);
    assert!(app.undo.undo(app.note.as_mut().unwrap()));
    assert_eq!(app.note.as_ref().unwrap().pages[4].row, 4);
}

#[test]
fn responsive_shelf_reflows_without_changing_saved_slots() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("A notebook with a deliberately long title");
    let bin_id = fixture.notebook("In the bin");
    fixture.app.lib.trash_note(bin_id).expect("trash notebook");
    let original_slot = fixture
        .app
        .lib
        .index
        .notes
        .iter()
        .find(|n| n.id == id)
        .unwrap()
        .slot;
    let app = &mut fixture.app;
    let mut frames = Frames::new(app);
    for size in [
        vec2(1280.0, 860.0),
        vec2(320.0, 280.0),
        vec2(360.0, 640.0),
        vec2(1024.0, 320.0),
    ] {
        frames.size = size;
        for bin in [false, true] {
            app.shelf_trash = bin;
            frames.settle(app);
            let screen = Rect::from_min_size(Pos2::ZERO, size);
            let search = frames
                .ctx
                .memory(|m| m.area_rect(Id::new("shelf-search")))
                .unwrap();
            let actions = frames
                .ctx
                .memory(|m| m.area_rect(Id::new("shelf-top-right")))
                .unwrap();
            assert!(screen.contains_rect(search), "search {search:?} {size:?}");
            assert!(screen.contains_rect(actions));
            assert!(!search.intersects(actions), "shelf header overlap {size:?}");
            if bin {
                assert!(screen.contains_rect(app.bin_close_rect));
            }
            for rect in app
                .shelf_grid
                .iter()
                .chain(&app.trash_grid)
                .filter(|r| r.is_positive())
            {
                assert!(screen.contains_rect(*rect));
                assert!(!rect.intersects(search));
                assert!(!rect.intersects(actions));
            }
            assert_eq!(
                app.lib
                    .index
                    .notes
                    .iter()
                    .find(|n| n.id == id)
                    .unwrap()
                    .slot,
                original_slot
            );
            if size.x == 320.0 {
                frames.snapshot(if bin {
                    "responsive-bin-small"
                } else {
                    "responsive-shelf-small"
                });
            }
        }
    }
    frames.size = vec2(320.0, 280.0);
    app.shelf_trash = false;
    app.lib.index.fiche_pliee = false;
    frames.settle(app);
    let help = frames
        .ctx
        .memory(|m| m.area_rect(Id::new("shelf-tuto-fiche")))
        .unwrap();
    assert!(Rect::from_min_size(Pos2::ZERO, frames.size).contains_rect(help));
    frames.snapshot("responsive-help-small");
    frames.key(app, Key::Escape);
    assert!(app.lib.index.fiche_pliee);
}

#[test]
fn compact_controls_keep_drag_selection_and_wheel_actions_accessible() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Compact interactions");
    let app = &mut fixture.app;
    let mut frames = Frames::new(app);
    frames.size = vec2(320.0, 280.0);
    frames.settle(app);
    let start = pos2(28.0, 160.0);
    frames.touch(app, TouchPhase::Start, start);
    app.open_spine_wheel(id, start, &frames.ctx, false);
    frames.settle(app);
    let (center, scale) = fitted_wheel(Rect::from_min_size(Pos2::ZERO, frames.size), start);
    let wheel = Rect::from_center_size(center, Vec2::splat(WHEEL_OUT * scale * 2.0));
    assert!(Rect::from_min_size(Pos2::ZERO, frames.size).contains_rect(wheel));
    frames.snapshot("responsive-wheel-small");
    frames.touch(app, TouchPhase::End, start);
    assert!(app.spine_wheel.is_none());
    assert!(app.emoji_pick.is_none());
    assert!(app.shelf_toss.is_empty());
    app.open_note(id);
    app.note.as_mut().unwrap().add_unit_at(1, 0);
    frames.settle(app);
    let dock = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
        .unwrap();
    frames.drag(app, dock.min + vec2(30.0, 29.0), pos2(310.0, 160.0));
    frames.settle(app);
    assert_eq!(app.dock_edge, DockEdge::Right);
    app.enter_pages_mode();
    frames.settle(app);
    assert!(app.sheet_tabs_on);
    frames.snapshot("responsive-page-selection-small");
    let close = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("strip-tray", "Close"))))
        .expect("the tray offers Close");
    frames.touch(app, TouchPhase::Start, close.center());
    frames.touch(app, TouchPhase::End, close.center());
    frames.settle(app);
    assert!(!app.sheet_tabs_on);
}

#[test]
fn early_turn_stays_while_the_hand_keeps_moving_into_the_new_wall() {
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0));
    let bounds = Rect::from_min_max(pos2(8.0, 60.0), pos2(892.0, 592.0));
    let tall = Rect::from_min_size(pos2(832.0, 92.0), vec2(58.0, 526.0));
    let pointer = pos2(861.0, 114.0);
    let down = vec2(0.0, 1700.0);
    assert_eq!(
        carrying_edge(pointer, screen, bounds, DockEdge::Right, tall, down),
        DockEdge::Bottom,
        "the long side turns before it meets the bottom"
    );
    // Next frame the pointer is still on the right wall. Nearest-edge snap
    // would stand the case back up. The hand is still moving down, so it stays.
    let flat = Rect::from_min_size(pos2(832.0, 92.0), vec2(526.0, 58.0));
    assert_eq!(
        carrying_edge(pointer, screen, bounds, DockEdge::Bottom, flat, down * 0.5),
        DockEdge::Bottom
    );
}

#[test]
fn coach_badge_sits_below_the_toolbar_as_a_separate_accessory() {
    if !coach::COACH_UI_VISIBLE {
        return;
    }
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Coach in tools");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    let dock = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
        .unwrap();
    let cap = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("coach-button-rect")))
        .unwrap();
    assert!(!dock.intersects(cap), "badge is detached from dock");
    assert!(
        cap.center().y < dock.top(),
        "cap sits under the toolbar toward the page, cap={cap:?} dock={dock:?}"
    );
    assert!(!app
        .tool_order
        .contains(&DockItem::Chrome(DockChrome::Coach)));
}

#[test]
fn coach_button_sits_next_to_search_in_the_header() {
    if !coach::COACH_UI_VISIBLE {
        return;
    }
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Coach beside top toolbar");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    let cap = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("coach-button-rect")))
        .unwrap();
    let search = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("pdf-search-button-rect")))
        .unwrap();
    assert!((cap.center().y - search.center().y).abs() < 1.0);
    assert!(cap.intersects(search.expand(56.0)));
}

#[test]
fn vertical_dock_keeps_a_color_column() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Vertical colors");
    let app = &mut fixture.app;
    app.open_note(id);
    app.dock_edge = DockEdge::Right;
    app.set_ink_panel(true);
    app.ink_panel_view = InkPanelView::Colors;
    let mut frames = Frames::new(app);
    frames.settle(app);
    let dock = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
        .unwrap();
    let palette = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("tin-rect")))
        .unwrap();
    let first = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("ink-swatch", 0))))
        .unwrap();
    let next = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("ink-swatch", 1))))
        .unwrap();
    assert!(
        (palette.size() - dock.size()).length() < 1.0,
        "vertical color twin matches the toolbar, palette={palette:?} dock={dock:?}"
    );
    assert!(
        (dock.left() - palette.right()).abs() < 11.0,
        "color twin sits against the right toolbar"
    );
    assert!((first.center().x - next.center().x).abs() < 1.0);
    assert!(next.top() >= first.bottom() - 1.0);
}

#[test]
fn horizontal_color_swatches_center_for_top_and_bottom_docks() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Centered colors");
    let app = &mut fixture.app;
    app.open_note(id);
    app.set_ink_panel(true);
    app.ink_panel_view = InkPanelView::Colors;
    let mut frames = Frames::new(app);
    for edge in [DockEdge::Bottom, DockEdge::Top] {
        app.dock_edge = edge;
        frames.settle(app);
        let palette = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("tin-rect")))
            .unwrap();
        let last = app.ink_colors().len() - 1;
        let first_swatch = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new(("ink-swatch", 0))))
            .unwrap();
        let last_swatch = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new(("ink-swatch", last))))
            .unwrap();
        let swatches_center = (first_swatch.left() + last_swatch.right()) * 0.5;
        assert!(
            (swatches_center - palette.center().x).abs() < 48.0,
            "{edge:?}: swatch row is centered in the color strip, row={swatches_center} palette={palette:?}"
        );
    }
}

#[test]
fn top_dock_rests_against_the_window_edge() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Top dock");
    let app = &mut fixture.app;
    app.open_note(id);
    app.dock_edge = DockEdge::Top;
    let mut frames = Frames::new(app);
    frames.size = vec2(900.0, 600.0);
    frames.settle(app);
    let dock = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
        .unwrap();
    assert_eq!(app.dock_edge, DockEdge::Top);
    assert!(
        dock.top() <= 8.0,
        "top dock sits under the transparent header, got {dock:?}"
    );
    assert!(
        dock.width() > dock.height(),
        "top dock stays a horizontal bar, got {dock:?}"
    );
}

#[test]
fn dock_folds_to_the_grip_while_held_and_opens_on_release() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Dock fold");
    let app = &mut fixture.app;
    let mut frames = Frames::new(app);
    frames.size = vec2(900.0, 600.0);
    app.open_note(id);
    frames.settle(app);
    let rest = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
        .unwrap();
    assert!(
        rest.width() > rest.height() * 2.0,
        "resting case is a long bar, got {rest:?}"
    );
    let grip = rest.min + vec2(22.0, 28.0);
    frames.step(app, vec![Event::PointerMoved(grip)]);
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: grip,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
    );
    for _ in 0..24 {
        frames.step(app, vec![]);
    }
    let folded = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
        .unwrap();
    assert!(
        app.dock_held && app.dock_fold > 0.85,
        "pressing the grip folds the case, held={} fold={}",
        app.dock_held,
        app.dock_fold
    );
    assert!(
        folded.width() < rest.width() * 0.45,
        "case shrinks toward the grip, rest {rest:?} folded {folded:?}"
    );
    assert!(
        (folded.width() - folded.height()).abs() < 24.0,
        "folded case is a pill on the grip, got {folded:?}"
    );
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: grip,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    for _ in 0..40 {
        frames.step(app, vec![]);
    }
    let open = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
        .unwrap();
    assert!(
        app.dock_fold < 0.05,
        "release opens the case, fold={}",
        app.dock_fold
    );
    assert!(
        (open.width() - rest.width()).abs() < 12.0,
        "opened case matches rest width, rest {rest:?} open {open:?}"
    );
}

#[test]
fn finger_can_move_toolbar_by_its_grip_after_tools_have_been_reordered() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Move toolbar");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.size = vec2(900.0, 600.0);
    frames.settle(app);
    let from = frames.ctx.data(|d| {
        d.get_temp::<Rect>(Id::new(("tool-well", Tool::Fineliner)))
            .unwrap()
            .center()
    });
    let to = frames.ctx.data(|d| {
        d.get_temp::<Rect>(Id::new(("tool-well", Tool::Highlighter)))
            .unwrap()
            .center()
    });
    frames.drag(app, from, to);
    frames.settle(app);
    let dock = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")).unwrap());
    let grip = dock.min + vec2(30.0, 29.0);
    frames.touch(app, TouchPhase::Start, grip);
    frames.touch(app, TouchPhase::Move, pos2(860.0, 280.0));
    assert!(app.dock_held, "the finger should pick up the grip");
    frames.touch(app, TouchPhase::End, pos2(860.0, 280.0));
    frames.settle(app);
    assert_eq!(app.dock_edge, DockEdge::Right);
}

#[test]
fn dock_turns_onto_the_edge_while_carried_then_springs_home() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Dock slide");
    let app = &mut fixture.app;
    let mut frames = Frames::new(app);
    frames.size = vec2(900.0, 600.0);
    app.open_note(id);
    frames.settle(app);
    assert_eq!(app.dock_edge, DockEdge::Bottom);
    let dock = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
        .unwrap();
    assert!(
        dock.width() > dock.height(),
        "resting case is a horizontal bar, got {dock:?}"
    );
    let rest_w = dock.width();
    let from = dock.min + vec2(30.0, 29.0);
    let to = pos2(860.0, 280.0);
    frames.step(app, vec![Event::PointerMoved(from)]);
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: from,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
    );
    // Lift without heading into a side wall: the case folds onto the grip.
    let lifted = pos2(from.x, (from.y - 180.0).max(120.0));
    frames.step(app, vec![Event::PointerMoved(lifted)]);
    for _ in 0..20 {
        frames.step(app, vec![]);
    }
    let folded = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
        .unwrap();
    assert!(
        app.dock_fold > 0.8,
        "held case folds onto the grip, fold={}",
        app.dock_fold
    );
    assert!(
        folded.width() < rest_w * 0.55,
        "folded case shrinks toward the grip, rest {rest_w} got {folded:?}"
    );
    let mut turned_with_room = false;
    let steps = 16;
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        let p = lifted + (to - lifted) * t;
        frames.step(app, vec![Event::PointerMoved(p)]);
        frames.step(app, vec![]);
        if app.dock_layout_edge() == DockEdge::Right && p.x < frames.size.x - 100.0 {
            turned_with_room = true;
        }
    }
    assert!(
        turned_with_room,
        "case should turn while there is still room before the right edge"
    );
    let mid = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
        .unwrap();
    assert_eq!(app.dock_edge, DockEdge::Bottom);
    assert_eq!(app.dock_layout_edge(), DockEdge::Right);
    assert!(
        mid.width() < rest_w * 0.55,
        "carried case stays folded, got {mid:?}"
    );
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: to,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert_eq!(app.dock_edge, DockEdge::Right);
    assert!(app.dock_float.is_some(), "release starts the slide");
    assert!(app.dock_fold > 0.8, "the case stays folded while traveling");
    assert!(
        app.dock_slide.is_some_and(|slide| !slide.arrived),
        "the destination move starts before the case unfolds"
    );
    for _ in 0..50 {
        frames.step(app, vec![]);
    }
    assert!(
        app.dock_float.is_none(),
        "slide settles: slide={:?}, fold={}, velocity={}",
        app.dock_slide
            .map(|slide| (slide.arrived, slide.shown, slide.vel)),
        app.dock_fold,
        app.dock_fold_vel
    );
    assert_eq!(app.dock_edge, DockEdge::Right);
    let landed = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
        .unwrap();
    assert!(
        landed.height() > landed.width(),
        "landed case is vertical, got {landed:?}"
    );
    assert!(
        landed.right() > frames.size.x - 30.0,
        "landed on the right, got {landed:?}"
    );

    // Right to bottom: the long side turns before it meets the bottom edge.
    let grip = landed.min + vec2(landed.width() * 0.5, 22.0);
    let down = pos2(grip.x, frames.size.y - 40.0);
    frames.step(app, vec![Event::PointerMoved(grip)]);
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: grip,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
    );
    let mut turned_above_bottom = false;
    for i in 1..=16 {
        let t = i as f32 / 16.0;
        let p = grip + (down - grip) * t;
        frames.step(app, vec![Event::PointerMoved(p)]);
        frames.step(app, vec![]);
        if app.dock_layout_edge() == DockEdge::Bottom && p.y < frames.size.y - 100.0 {
            turned_above_bottom = true;
        }
    }
    assert!(
        turned_above_bottom,
        "case should turn while there is still room before the bottom edge"
    );
    assert_eq!(app.dock_layout_edge(), DockEdge::Bottom);
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: down,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
}

#[test]
fn pages_mode_minus_removes_a_sticky() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Sticky minus");
    let app = &mut fixture.app;
    app.open_note(id);
    let tx = TextBox::sticky(pos2(200.0, 180.0), app.look.sticky_yellow());
    let sticky_id = tx.id;
    app.note.as_mut().unwrap().pages[0].texts.push(tx);
    let mut frames = Frames::new(app);
    frames.settle(app);
    app.camera = Camera {
        pan: vec2(40.0, 80.0),
        zoom: 0.8,
        stretch_y: None,
    };
    app.fitted_cell = None;
    app.need_fit = false;
    app.enter_pages_mode();
    frames.step(app, vec![]);
    let hit = {
        let tx = &app.note.as_ref().unwrap().pages[0].texts[0];
        sticky_minus_band(app.sticky_screen_rect(app.canvas_rect, 0, tx)).center()
    };
    frames.drag(app, hit, hit);
    frames.settle(app);
    assert!(app.note.as_ref().unwrap().pages[0]
        .texts
        .iter()
        .all(|t| t.id != sticky_id));
    assert!(app.undo.undo(app.note.as_mut().unwrap()));
    assert!(app.note.as_ref().unwrap().pages[0]
        .texts
        .iter()
        .any(|t| t.id == sticky_id));
}

#[test]
fn pages_mode_minus_removes_an_image_with_undo() {
    for touch in [false, true] {
        let mut fixture = Fixture::new();
        let id = fixture.notebook("Image minus");
        let app = &mut fixture.app;
        app.open_note(id);
        let image_id = Uuid::new_v4();
        app.note.as_mut().unwrap().pages[0].images.push(ImageObj {
            id: image_id,
            pos: [220.0, 180.0],
            size: [120.0, 90.0],
            file: "missing.png".into(),
        });
        let mut frames = Frames::new(app);
        frames.settle(app);
        app.enter_pages_mode();
        frames.settle(app);
        let image = app.image_screen_rect(
            app.canvas_rect,
            0,
            &app.note.as_ref().unwrap().pages[0].images[0],
        );
        let minus = image_minus_band(image).center();
        assert_eq!(
            app.image_minus_at(minus, app.canvas_rect),
            Some((0, image_id))
        );
        if touch {
            frames.touch(app, TouchPhase::Start, minus);
            frames.touch(app, TouchPhase::End, minus);
        } else {
            frames.drag(app, minus, minus);
        }
        assert!(app.note.as_ref().unwrap().pages[0].images.is_empty());
        assert!(app.undo.undo(app.note.as_mut().unwrap()));
        assert_eq!(app.note.as_ref().unwrap().pages[0].images[0].id, image_id);
    }
}

#[test]
fn scrolling_compact_toolbar_does_not_zoom_the_page() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Scroll tools");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.size = vec2(360.0, 640.0);
    frames.settle(app);
    let dock = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
        .unwrap();
    let camera = app.camera;
    frames.step(
        app,
        vec![
            Event::PointerMoved(dock.center()),
            Event::MouseWheel {
                unit: MouseWheelUnit::Point,
                delta: vec2(-150.0, -50.0),
                modifiers: Modifiers::NONE,
            },
        ],
    );
    frames.settle(app);
    assert_eq!(app.camera.zoom, camera.zoom);
    assert_eq!(app.camera.pan, camera.pan);
}

#[test]
fn fit_page_button_is_in_the_header_not_the_notebook_menu() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Fullscreen button");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    let full = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("fullscreen-button-rect")))
        .unwrap();
    assert!(
        full.top() < 52.0,
        "fullscreen control belongs in the top bar"
    );
    let more = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("notebook-menu-button-rect")))
        .unwrap();
    frames.drag(app, more.center(), more.center());
    frames.settle(app);
    assert!(!frames.output.as_ref().unwrap().shapes.iter().any(|shape| {
        matches!(&shape.shape, Shape::Text(text) if text.galley.text() == "Fit page")
    }));
    app.camera.zoom = 0.8;
    app.fitted_cell = None;
    frames.drag(app, full.center(), full.center());
    frames.step(app, vec![]);
    assert_eq!(
        app.fitted_cell,
        Some((0, 0)),
        "button fits the current page"
    );
    assert!(!frames
        .output
        .as_ref()
        .unwrap()
        .viewport_output
        .values()
        .any(|viewport| {
            viewport
                .commands
                .iter()
                .any(|command| matches!(command, ViewportCommand::Fullscreen(_)))
        }));
}

#[test]
fn notebook_menu_keeps_actions_and_keyboard_dismissal() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Notebook menu");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    let more = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("notebook-menu-button-rect")))
        .unwrap()
        .center();
    frames.drag(app, more, more);
    frames.settle(app);
    assert!(Popup::is_any_open(&frames.ctx));
    let last = frames.text_center("Area eraser");
    frames.text_center("Stroke eraser");
    assert!(
        frames.output.as_ref().unwrap().shapes.iter().any(|shape| {
            matches!(&shape.shape, Shape::Text(text) if text.galley.text() == "Area eraser")
                && shape.clip_rect.contains(last)
        }),
        "all menu actions should fit in a normal window"
    );
    frames.snapshot("notebook-menu");
    frames.step_with_modifiers(
        app,
        vec![Event::Key {
            key: Key::S,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        }],
        Modifiers::COMMAND,
    );
    frames.settle(app);
    assert!(
        app.export_picker_open,
        "Ctrl+S still opens the export picker"
    );
    frames.text_center("PNG image");
    frames.text_center("PDF document");
    frames.snapshot("download-canson");
    let cancel = frames.text_center("Cancel");
    frames.drag(app, cancel, cancel);
    frames.settle(app);
    assert!(!app.export_picker_open);
    frames.drag(app, more, more);
    frames.settle(app);
    let finger = frames.text_center("Draw with finger");
    let was_finger = app.lib.index.finger_draw;
    frames.drag(app, finger, finger);
    assert_eq!(
        app.lib.index.finger_draw, !was_finger,
        "the ⋮ menu applies a settings toggle on the first click"
    );
    assert!(
        Popup::is_any_open(&frames.ctx),
        "toggling a setting keeps the ⋮ menu open until an outside click"
    );
    frames.drag(app, finger, finger);
    assert_eq!(
        app.lib.index.finger_draw, was_finger,
        "a second click toggles the setting back"
    );
    assert!(Popup::is_any_open(&frames.ctx));
    frames.key(app, Key::Escape);
    assert!(!Popup::is_any_open(&frames.ctx));
    assert!(matches!(app.scene, Scene::Desk));
    frames.size.y = 400.0;
    frames.settle(app);
    frames.drag(app, more, more);
    frames.settle(app);
    assert!(Popup::is_any_open(&frames.ctx));
    frames.snapshot("notebook-menu-short");
    let save = frames.text_center("Draw with finger");
    assert!(save.y > 0.0 && save.y < frames.size.y);
    let mut reached_last = false;
    for _ in 0..48 {
        frames.key(app, Key::Tab);
        frames.settle(app);
        let last = frames.text_center("Area eraser");
        if frames
            .ctx
            .memory(|m| m.focused())
            .and_then(|id| frames.ctx.read_response(id))
            .is_some_and(|response| response.rect.contains(last))
        {
            reached_last = true;
            assert!(last.y > 52.0 && last.y < frames.size.y);
            break;
        }
    }
    assert!(
        reached_last,
        "keyboard navigation must reveal clipped menu actions"
    );
}

#[test]
fn paper_style_is_available_in_pages_mode_even_with_one_page() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Paper menu");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    assert!(
        frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new(("strip-paper", "Lined"))))
            .is_none(),
        "paper swatches live inside Pages edit, not the writing canvas"
    );
    app.enter_pages_mode();
    frames.settle(app);
    let lined = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("strip-paper", "Lined"))))
        .unwrap();
    frames.drag(app, lined.center(), lined.center());
    assert_eq!(app.note.as_ref().unwrap().paper, PaperKind::Lined);
}

#[test]
fn pages_edit_replaces_the_header_with_a_canvas_tray() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Tray");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    assert!(frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("top-rect")))
        .is_some());
    let more = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("notebook-menu-button-rect")))
        .unwrap()
        .center();
    frames.drag(app, more, more);
    frames.settle(app);
    let edit = frames.text_center("Edit pages");
    frames.drag(app, edit, edit);
    frames.settle(app);
    assert!(app.sheet_tabs_on);
    let header_visible = |frames: &Frames| {
        frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("top-rect")))
            .is_some()
    };
    assert!(
        !header_visible(&frames),
        "the whole header disappears while editing pages"
    );
    frames.snapshot("pages-edit-tray");
    let size = frames.size;
    for (window, artifact) in [
        (size, "pages-edit-tray"),
        (vec2(700.0, 640.0), "pages-edit-tray-two-rows"),
        (vec2(420.0, 640.0), "pages-edit-tray-compact"),
        (vec2(320.0, 640.0), "pages-edit-tray-narrow"),
        (vec2(320.0, 280.0), "pages-edit-tray-short"),
    ] {
        frames.size = window;
        frames.settle(app);
        for material in [false, true] {
            if window.x < 760.0 {
                let tab = frames.text_center(if material { "Paper" } else { "Arrange" });
                frames.touch(app, TouchPhase::Start, tab);
                frames.touch(app, TouchPhase::End, tab);
                frames.settle(app);
            }
            frames.snapshot(artifact);
            let tray = frames
                .ctx
                .data(|d| d.get_temp::<Rect>(Id::new("pages-strip-rect")))
                .unwrap();
            assert!(
                app.canvas_rect.contains_rect(tray),
                "tray fits at {window:?}"
            );
            let mut controls: Vec<Rect> = Vec::new();
            let mut ids = vec![Id::new(("strip-tray", "Close"))];
            if window.x >= 760.0 || !material {
                ids.extend(["Linked", "Separate"].map(|name| Id::new(("strip-tray", name))));
                ids.extend(
                    ["Stack", "Row", "Less", "Count", "More"]
                        .map(|name| Id::new(("strip-flow", name))),
                );
            }
            if window.x >= 760.0 || material {
                ids.extend(PaperKind::ALL.map(|kind| Id::new(("strip-paper", kind.label()))));
                ids.extend(["White", "Sand", "Black"].map(|name| Id::new(("strip-tone", name))));
            }
            for id in ids {
                let target = frames
                    .ctx
                    .data(|d| d.get_temp::<Rect>(id))
                    .expect("visible edit control");
                assert!(
                    tray.contains_rect(target),
                    "control stays inside tray at {window:?}"
                );
                assert!(
                    !controls
                        .iter()
                        .any(|other| other.intersects(target.shrink(0.1))),
                    "controls cannot overlap at {window:?}"
                );
                controls.push(target);
            }
        }
    }
    frames.size = size;
    frames.settle(app);
    let strip = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("pages-strip-rect")))
        .expect("the canvas tray appears");
    assert!(
        app.canvas_rect.contains(strip.min) && strip.bottom() <= app.canvas_rect.bottom() + 1.0
    );
    let separate = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("strip-tray", "Separate"))))
        .expect("the tray offers join controls");
    frames.drag(app, separate.center(), separate.center());
    assert_eq!(app.note.as_ref().unwrap().sheet_join, SheetJoin::Separate);
    let close = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("strip-tray", "Close"))))
        .expect("the tray offers Close");
    frames.touch(app, TouchPhase::Start, close.center());
    frames.touch(app, TouchPhase::End, close.center());
    frames.settle(app);
    assert!(!app.sheet_tabs_on);
    assert!(
        header_visible(&frames),
        "the header returns after leaving pages edit"
    );
}

#[test]
fn pdf_pages_can_be_arranged_horizontally_or_two_per_row() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("PDF arrangement");
    let app = &mut fixture.app;
    app.open_note(id);
    let note = app.note.as_mut().unwrap();
    note.sheet_join = SheetJoin::Separate;
    note.pages = (0..5)
        .map(|index| {
            let mut page = crate::document::Page::at(0, index);
            page.background = Some(PageBackground {
                file: "source.pdf".into(),
                pdf_page: Some(index as u32),
                source_size: None,
            });
            page
        })
        .collect();
    let mut frames = Frames::new(app);
    frames.settle(app);
    app.enter_pages_mode();
    frames.settle(app);
    let cells_now = |app: &InkarchyApp| -> Vec<(i32, i32)> {
        app.note
            .as_ref()
            .unwrap()
            .pages
            .iter()
            .map(|page| (page.col, page.row))
            .collect()
    };
    let keep_pdf_intact = |app: &InkarchyApp| {
        for (index, page) in app.note.as_ref().unwrap().pages.iter().enumerate() {
            assert_eq!(
                page.background.as_ref().unwrap().pdf_page,
                Some(index as u32)
            );
        }
    };
    let tap = |frames: &mut Frames, app: &mut InkarchyApp, well: &str| {
        let target = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new(("strip-flow", well))))
            .expect("tray control");
        frames.drag(app, target.center(), target.center());
        frames.settle(app);
    };
    // One horizontal row holds every page.
    tap(&mut frames, app, "Row");
    assert_eq!(cells_now(app), vec![(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)]);
    keep_pdf_intact(app);
    // The stepper sleeps while one row holds everything.
    tap(&mut frames, app, "More");
    assert_eq!(cells_now(app), vec![(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)]);
    // Coming back to the stack resumes the remembered wrapping.
    tap(&mut frames, app, "Stack");
    assert_eq!(cells_now(app), vec![(0, 0), (0, 1), (0, 2), (0, 3), (0, 4)]);
    keep_pdf_intact(app);
    for (well, expected) in [
        ("More", vec![(0, 0), (1, 0), (0, 1), (1, 1), (0, 2)]),
        ("More", vec![(0, 0), (1, 0), (2, 0), (0, 1), (1, 1)]),
        ("More", vec![(0, 0), (1, 0), (2, 0), (3, 0), (0, 1)]),
        ("More", vec![(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)]),
        ("More", vec![(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)]),
    ] {
        tap(&mut frames, app, well);
        assert_eq!(cells_now(app), expected);
        keep_pdf_intact(app);
    }
    // The chosen density survives a round trip through one row.
    tap(&mut frames, app, "Row");
    assert_eq!(cells_now(app), vec![(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)]);
    tap(&mut frames, app, "Stack");
    assert_eq!(cells_now(app), vec![(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)]);
    keep_pdf_intact(app);
    // And the stepper removes columns again.
    tap(&mut frames, app, "Less");
    assert_eq!(cells_now(app), vec![(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)]);
    keep_pdf_intact(app);
    tap(&mut frames, app, "Less");
    assert_eq!(cells_now(app), vec![(0, 0), (1, 0), (2, 0), (3, 0), (0, 1)]);
    keep_pdf_intact(app);
}

#[test]
fn notebook_menu_stays_clear_of_right_tool_case() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Menu beside tools");
    let app = &mut fixture.app;
    app.open_note(id);
    app.dock_edge = DockEdge::Right;
    let mut frames = Frames::new(app);
    for size in [vec2(1280.0, 860.0), vec2(360.0, 640.0), vec2(320.0, 280.0)] {
        frames.size = size;
        frames.settle(app);
        let more = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("notebook-menu-button-rect")))
            .unwrap();
        frames.drag(app, more.center(), more.center());
        frames.settle(app);
        let menu = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("notebook-menu-rect")))
            .unwrap();
        let dock = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
            .unwrap();
        assert!(
            menu.right() + 4.0 <= dock.left(),
            "{size:?}: menu {menu:?} covers dock {dock:?}"
        );
        assert!(
            menu.left() >= 0.0 && menu.bottom() <= size.y,
            "{size:?}: menu {menu:?} outside window"
        );
        frames.key(app, Key::Escape);
    }
}

#[test]
fn ink_tin_keeps_color_picking_and_fits_after_resize() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Ink tin");
    let app = &mut fixture.app;
    app.open_note(id);
    app.tin_open = true;
    let mut frames = Frames::new(app);
    frames.settle(app);
    assert!(frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("tin-wheel")))
        .is_none());
    let dock = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
        .unwrap();
    let palette = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("tin-rect")))
        .unwrap();
    let first = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("ink-swatch", 0))))
        .unwrap();
    let next = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("ink-swatch", 1))))
        .unwrap();
    assert!(
        (palette.size() - dock.size()).length() < 1.0,
        "color bar mirrors the toolbar size palette={palette:?} dock={dock:?}"
    );
    assert!(
        (dock.top() - palette.bottom()).abs() < 11.0,
        "color bar sits against the toolbar"
    );
    assert!((first.center().y - next.center().y).abs() < 1.0);
    assert_eq!(first.size(), Vec2::splat(40.0));
    assert!(
        next.left() >= first.right() - 1.0,
        "swatches follow one horizontal row"
    );
    frames.snapshot("ink-swatches");
    let add = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("ink-add-color")))
        .unwrap();
    assert_eq!(add.size(), Vec2::splat(48.0));
    frames.drag(app, add.center(), add.center());
    frames.settle(app);
    assert!(app.color_mixer_open);
    let mixer = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("tin-rect")))
        .unwrap();
    let dock = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
        .unwrap();
    assert!(
        (mixer.size() - dock.size()).length() < 1.0,
        "mixer keeps the toolbar twin size"
    );
    assert!((mixer.center().x - palette.center().x).abs() < 4.0);
    assert!(frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("tin-wheel")))
        .is_none());
    let hue = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("ink-slider", 0))))
        .unwrap();
    let sat = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("ink-slider", 1))))
        .unwrap();
    let hue_at = pos2(
        hue.center().x,
        hue.top() + 16.0 + (hue.height() - 21.0) * 0.25,
    );
    frames.drag(app, hue_at, hue_at);
    assert!(
        (app.tin_hue - 0.25).abs() < 0.04,
        "hue slider sets tin_hue, got {}",
        app.tin_hue
    );
    assert!(app.tin_open);
    let saturation = pos2(
        sat.center().x,
        sat.top() + 16.0 + (sat.height() - 21.0) * 0.65,
    );
    frames.touch(app, TouchPhase::Start, saturation);
    frames.touch(app, TouchPhase::End, saturation);
    let expect = hsv_to_rgb(app.tin_hue, 1.0, 0.7);
    assert!((app.ink.r() as i16 - expect.r() as i16).abs() <= 2);
    assert!((app.ink.g() as i16 - expect.g() as i16).abs() <= 2);
    assert!((app.ink.b() as i16 - expect.b() as i16).abs() <= 2);
    assert!(app.tin_open);
    frames.settle(app);
    let tin = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("tin-rect")))
        .unwrap();
    let dock = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
        .unwrap();
    assert!(
        !tin.shrink(0.5).intersects(dock),
        "colour wheel must not cover the tools"
    );
    assert!(
        (tin.center().x - dock.center().x).abs() < 4.0,
        "ink case twin stays aligned with the toolbar"
    );
    frames.snapshot("ink-tin");
    frames.size = vec2(900.0, 500.0);
    frames.settle(app);
    let tin = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("tin-rect")))
        .unwrap();
    assert!(Rect::from_min_size(Pos2::ZERO, frames.size).contains_rect(tin));
    let save = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("ink-save-color")))
        .unwrap();
    assert_eq!(save.size(), Vec2::splat(48.0));
    assert!(
        tin.expand(3.0).contains_rect(save),
        "save stays visible in a short window tin={tin:?} save={save:?}"
    );
    frames.snapshot("ink-tin-short");
    frames.touch(app, TouchPhase::Start, save.center());
    frames.touch(app, TouchPhase::End, save.center());
    frames.size = vec2(1280.0, 860.0);
    frames.settle(app);
    assert!(!app.color_mixer_open);
    let saved_ink = app.ink;
    assert_eq!(
        app.lib.index.custom_inks,
        vec![[saved_ink.r(), saved_ink.g(), saved_ink.b()]]
    );
    app.save_ink_swatch();
    assert_eq!(
        app.lib.index.custom_inks.len(),
        1,
        "duplicate colors are not added"
    );
    let persisted: crate::library::Index =
        serde_json::from_slice(&std::fs::read(app.lib.root.join("library.json")).unwrap()).unwrap();
    assert_eq!(persisted.custom_inks, app.lib.index.custom_inks);
    let custom = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("ink-swatch", app.look.inks.len()))))
        .unwrap();
    app.pick_free_ink(Color32::RED);
    frames.drag(app, custom.center(), custom.center());
    assert_eq!(app.ink, saved_ink, "saved colors can be selected again");
    frames.snapshot("ink-custom-swatch");
    frames.key(app, Key::Escape);
    assert!(!app.tin_open);
    assert!(matches!(app.scene, Scene::Desk));
}

#[test]
fn dragged_toolbar_icon_leaves_its_seat_and_parks_in_a_chosen_slot() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Drag follows finger");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    let source = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("tool-well", Tool::Fineliner))))
        .unwrap();
    frames.step(
        app,
        vec![
            Event::PointerMoved(source.center()),
            Event::PointerButton {
                pos: source.center(),
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
    );
    frames.step(
        app,
        vec![Event::PointerMoved(source.center() - vec2(0.0, 40.0))],
    );
    frames.settle(app);
    assert_eq!(app.dock_tool_drag, Some(DockItem::Tool(Tool::Fineliner)));
    // Source well is hollowed out while the ghost rides the pointer.
    let hollow = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("tool-well", Tool::Fineliner))))
        .unwrap_or(Rect::NOTHING);
    assert!(
        !hollow.is_positive(),
        "dragged icon must leave its toolbar seat"
    );
    let seat = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("storage-slot", 5))))
        .expect("storage seats are addressable");
    frames.step(app, vec![Event::PointerMoved(seat.center())]);
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: seat.center(),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert!(app.tool_is_parked(Tool::Fineliner));
    assert_eq!(app.parked_slots[5], Some(Tool::Fineliner));
    assert!(app.parked_slots[..5].iter().all(Option::is_none));
}

#[test]
fn tools_can_be_stored_and_restored_by_dragging_between_panel_and_dock() {
    for (touch, tool) in [(false, Tool::Fineliner), (true, Tool::Fineliner)] {
        let mut fixture = Fixture::new();
        let id = fixture.notebook("Tool storage");
        let app = &mut fixture.app;
        app.open_note(id);
        let mut frames = Frames::new(app);
        frames.settle(app);
        assert!(!app.ink_panel_open());
        let source = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new(("tool-well", tool))))
            .unwrap()
            .center();
        if touch {
            frames.touch(app, TouchPhase::Start, source);
            frames.touch(app, TouchPhase::Move, source - vec2(0.0, 30.0));
        } else {
            frames.step(
                app,
                vec![
                    Event::PointerMoved(source),
                    Event::PointerButton {
                        pos: source,
                        button: PointerButton::Primary,
                        pressed: true,
                        modifiers: Modifiers::NONE,
                    },
                ],
            );
            frames.step(app, vec![Event::PointerMoved(source - vec2(0.0, 30.0))]);
        }
        frames.settle(app);
        assert!(app.ink_panel_open(), "drag opens storage, touch={touch}");
        let storage = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("dock-tool-storage-rect")))
            .unwrap();
        if touch {
            frames.touch(app, TouchPhase::Move, storage.center());
            frames.touch(app, TouchPhase::End, storage.center());
        } else {
            frames.step(app, vec![Event::PointerMoved(storage.center())]);
            frames.step(
                app,
                vec![Event::PointerButton {
                    pos: storage.center(),
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                }],
            );
        }
        assert!(app.tool_is_parked(tool), "store {tool:?} touch={touch}");
        assert!(
            app.parked_slots.iter().any(|slot| *slot == Some(tool)),
            "tool lands in a storage seat"
        );
        assert!(
            app.note.as_ref().unwrap().pages[0].texts.is_empty(),
            "storing a tool must not place a sticky note"
        );
        frames.settle(app);
        frames.snapshot(if touch {
            "storage-touch"
        } else {
            "storage-mouse"
        });
        frames.key(app, Key::Escape);
        frames.settle(app);
        let storage_button = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("dock-storage-button")))
            .unwrap();
        frames.drag(app, storage_button.center(), storage_button.center());
        frames.settle(app);
        assert_eq!(app.ink_panel_view, InkPanelView::Tools);
        let stored = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new(("tool-well", tool))))
            .unwrap()
            .center();
        let dock = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
            .unwrap();
        if touch {
            frames.touch(app, TouchPhase::Start, stored);
            frames.touch(app, TouchPhase::Move, dock.center());
            frames.touch(app, TouchPhase::End, dock.center());
        } else {
            frames.drag(app, stored, dock.center());
        }
        assert!(!app.tool_is_parked(tool), "restore {tool:?} touch={touch}");
        assert!(app.lib.index.parked_tools.is_empty());
        assert!(app.dock_tool_drag.is_none());
    }
}

#[test]
fn storage_keeps_nine_slots_and_stays_open_after_restoring() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Compact storage");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    for (i, tool) in PARKABLE_TOOLS.into_iter().take(4).enumerate() {
        app.park_tool_at(tool, i);
    }
    app.set_ink_panel(true);
    app.ink_panel_view = InkPanelView::Tools;
    frames.settle(app);
    let dock = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
        .unwrap();
    let twin = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("tin-rect")))
        .unwrap();
    let storage = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-tool-storage-rect")))
        .unwrap();
    assert!(
        (twin.size() - dock.size()).length() < 1.0,
        "storage twin mirrors the toolbar size twin={twin:?} dock={dock:?}"
    );
    assert!(
        twin.contains_rect(storage),
        "storage seats stay inside the twin bay"
    );
    let third = PARKABLE_TOOLS[3];
    let well = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("tool-well", third))))
        .unwrap();
    assert!(
        storage.expand(2.0).intersects(well),
        "last stored tool remains visible"
    );
    frames.drag(app, well.center(), well.center());
    assert!(!app.tool_is_parked(third));
    frames.settle(app);
    assert!(
        app.ink_panel_open() && app.ink_panel_view == InkPanelView::Tools,
        "restoring a tool leaves the tray open"
    );
    let after = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-tool-storage-rect")))
        .unwrap();
    let after_twin = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("tin-rect")))
        .unwrap();
    let after_dock = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
        .unwrap();
    assert!(
        (after_twin.size() - after_dock.size()).length() < 1.0,
        "tray keeps mirroring the toolbar after a restore"
    );
    assert!(after_twin.contains_rect(after));

    for (i, tool) in PARKABLE_TOOLS.into_iter().enumerate() {
        app.park_tool_at(tool, i);
    }
    frames.settle(app);
    let storage = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-tool-storage-rect")))
        .unwrap();
    for tool in PARKABLE_TOOLS {
        let well = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new(("tool-well", tool))))
            .unwrap();
        assert!(
            storage.expand(2.0).intersects(well),
            "{tool:?} fits in the nine slots"
        );
    }
}

#[test]
fn dragging_a_tool_onto_another_inserts_it_at_the_indicated_place() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Swap tools");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    let fineliner = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("tool-well", Tool::Fineliner))))
        .unwrap();
    let brush = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("tool-well", Tool::Brush))))
        .unwrap();
    assert!(fineliner.center().x < brush.center().x);
    frames.drag(app, fineliner.center(), brush.center());
    frames.settle(app);
    assert_eq!(
        app.tool_order[0],
        DockItem::Tool(Tool::Brush),
        "brush shifts into the first slot"
    );
    assert_eq!(
        app.tool_order[1],
        DockItem::Tool(Tool::Fineliner),
        "fineliner moves to the second"
    );
    assert!(!app.tool_is_parked(Tool::Fineliner));
    let saved: crate::library::Index =
        serde_json::from_slice(&std::fs::read(app.lib.root.join("library.json")).unwrap()).unwrap();
    assert_eq!(saved.tool_order, app.tool_order, "order survives a restart");
}

#[test]
fn toolbar_drag_previews_and_inserts_across_multiple_wells() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Move tools");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    let start = frames.ctx.data(|d| {
        d.get_temp::<Rect>(Id::new(("tool-well", Tool::Fineliner)))
            .unwrap()
            .center()
    });
    let end = frames.ctx.data(|d| {
        d.get_temp::<Rect>(Id::new(("tool-well", Tool::Highlighter)))
            .unwrap()
            .center()
    });
    let dock = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")).unwrap());
    frames.step(app, vec![Event::PointerMoved(start)]);
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: start,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
    );
    frames.step(app, vec![Event::PointerMoved(end)]);
    let (_, mark) = app
        .dock_insert_target(DockItem::Tool(Tool::Fineliner), end, dock)
        .expect("the hovered position has an insertion guide");
    assert!(frames.output.as_ref().unwrap().shapes.iter().any(|shape| {
        matches!(&shape.shape, Shape::Rect(rect) if rect.rect == mark && rect.fill == app.look.paper)
    }), "the paper-colored bar is visible before release");
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: end,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert_eq!(
        &app.tool_order[..4],
        &[
            DockItem::Tool(Tool::Brush),
            DockItem::Tool(Tool::Pencil),
            DockItem::Tool(Tool::Highlighter),
            DockItem::Tool(Tool::Fineliner)
        ],
        "intermediate icons shift instead of swapping with the destination"
    );
    assert!(app.dock_tool_drag.is_none());
}

#[test]
fn touch_along_toolbar_reorders_instead_of_scrolling() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Touch reorder");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    let well = |tool| {
        frames.ctx.data(|d| {
            d.get_temp::<Rect>(Id::new(("tool-well", tool)))
                .unwrap()
                .center()
        })
    };
    let from = well(Tool::Fineliner);
    let to = well(Tool::Highlighter);
    frames.touch(app, TouchPhase::Start, from);
    frames.touch(app, TouchPhase::Move, to);
    assert_eq!(app.dock_tool_drag, Some(DockItem::Tool(Tool::Fineliner)));
    frames.touch(app, TouchPhase::End, to);
    assert_eq!(
        &app.tool_order[..4],
        &[
            DockItem::Tool(Tool::Brush),
            DockItem::Tool(Tool::Pencil),
            DockItem::Tool(Tool::Highlighter),
            DockItem::Tool(Tool::Fineliner)
        ]
    );
}

#[test]
fn undo_redo_ink_and_storage_can_each_be_reordered() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("All dock icons");
    let app = &mut fixture.app;
    app.open_note(id);
    app.park_tool_at(Tool::Hand, 0);
    let mut frames = Frames::new(app);
    frames.settle(app);
    for item in [
        DockChrome::Undo,
        DockChrome::Redo,
        DockChrome::Ink,
        DockChrome::Storage,
    ] {
        let source = app
            .dock_tool_wells
            .iter()
            .find(|(key, _)| *key == DockItem::Chrome(item))
            .unwrap()
            .1
            .center();
        let first = app
            .dock_tool_wells
            .iter()
            .find(|(key, _)| *key == DockItem::Tool(Tool::Fineliner))
            .unwrap()
            .1
            .center()
            - vec2(15.0, 0.0);
        frames.step(app, vec![Event::PointerMoved(source)]);
        frames.step(
            app,
            vec![Event::PointerButton {
                pos: source,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }],
        );
        frames.step(app, vec![Event::PointerMoved(first)]);
        assert_eq!(
            app.dock_tool_drag,
            Some(DockItem::Chrome(item)),
            "{item:?} starts dragging"
        );
        let dock = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")).unwrap());
        assert!(
            app.dock_insert_target(DockItem::Chrome(item), first, dock)
                .is_some(),
            "{item:?} can target first well, dock={dock:?}, first={first:?}"
        );
        frames.step(
            app,
            vec![Event::PointerButton {
                pos: first,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
        );
        frames.settle(app);
        assert!(
            app.tool_order
                .iter()
                .position(|key| *key == DockItem::Chrome(item))
                .unwrap()
                < app
                    .tool_order
                    .iter()
                    .position(|key| *key == DockItem::Tool(Tool::Fineliner))
                    .unwrap(),
            "{item:?} moves before the pen"
        );
    }
    assert!(
        app.tool_is_parked(Tool::Hand),
        "moving the storage button never parks another icon"
    );
    let saved: crate::library::Index =
        serde_json::from_slice(&std::fs::read(app.lib.root.join("library.json")).unwrap()).unwrap();
    assert_eq!(saved.tool_order, app.tool_order);
}

#[test]
fn vertical_dock_moves_the_ink_well_with_a_horizontal_guide() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Vertical dock order");
    let app = &mut fixture.app;
    app.open_note(id);
    app.dock_edge = DockEdge::Right;
    let mut frames = Frames::new(app);
    frames.settle(app);
    let from = app
        .dock_tool_wells
        .iter()
        .find(|(item, _)| *item == DockItem::Chrome(DockChrome::Ink))
        .unwrap()
        .1
        .center();
    let to = app
        .dock_tool_wells
        .iter()
        .find(|(item, _)| *item == DockItem::Tool(Tool::Fineliner))
        .unwrap()
        .1
        .center()
        - vec2(0.0, 15.0);
    let dock = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")).unwrap());
    frames.step(app, vec![Event::PointerMoved(from)]);
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: from,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
    );
    frames.step(app, vec![Event::PointerMoved(to)]);
    let (_, guide) = app
        .dock_insert_target(DockItem::Chrome(DockChrome::Ink), to, dock)
        .unwrap();
    assert!(guide.width() > guide.height());
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: to,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert_eq!(app.tool_order[0], DockItem::Chrome(DockChrome::Ink));
}

#[test]
fn holding_an_icon_in_a_compact_dock_reorders_instead_of_swiping() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Compact reorder");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.size = vec2(360.0, 500.0);
    frames.settle(app);
    let from = app
        .dock_tool_wells
        .iter()
        .find(|(key, _)| *key == DockItem::Tool(Tool::Brush))
        .unwrap()
        .1
        .center();
    let to = app
        .dock_tool_wells
        .iter()
        .find(|(key, _)| *key == DockItem::Tool(Tool::Fineliner))
        .unwrap()
        .1
        .center()
        - vec2(15.0, 0.0);
    frames.touch(app, TouchPhase::Start, from);
    for _ in 0..25 {
        frames.step(app, vec![]);
    }
    frames.touch(app, TouchPhase::Move, to);
    assert_eq!(app.dock_tool_drag, Some(DockItem::Tool(Tool::Brush)));
    frames.touch(app, TouchPhase::End, to);
    assert_eq!(app.tool_order[0], DockItem::Tool(Tool::Brush));
}

#[test]
fn dragging_to_compact_dock_edge_reaches_offscreen_icons() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Reach all tools");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.size = vec2(360.0, 500.0);
    frames.settle(app);
    let from = app
        .dock_tool_wells
        .iter()
        .find(|(key, _)| *key == DockItem::Tool(Tool::Fineliner))
        .unwrap()
        .1
        .center();
    let dock = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")).unwrap());
    let edge = pos2(dock.right() - 10.0, from.y);
    frames.step(app, vec![Event::PointerMoved(from)]);
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: from,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
    );
    frames.step(app, vec![Event::PointerMoved(edge)]);
    assert_eq!(app.dock_tool_drag, Some(DockItem::Tool(Tool::Fineliner)));
    for _ in 0..70 {
        frames.step(app, vec![]);
    }
    assert!(
        app.dock_tool_wells
            .iter()
            .any(|(item, _)| *item == DockItem::Chrome(DockChrome::Ink)),
        "holding near the edge scrolls through all the dock icons"
    );
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: edge,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert!(
        app.tool_order
            .iter()
            .position(|item| *item == DockItem::Tool(Tool::Fineliner))
            .unwrap()
            > app
                .tool_order
                .iter()
                .position(|item| *item == DockItem::Chrome(DockChrome::Redo))
                .unwrap()
    );
}

#[test]
fn hand_tool_is_available_and_owns_object_manipulation() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Hand tool");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);

    let hand = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("tool-well", Tool::Hand))))
        .expect("hand is present in the toolbar");
    frames.drag(app, hand.center(), hand.center());
    assert_eq!(app.tool, Tool::Hand);

    let mut text = TextBox::new(pos2(180.0, 180.0), Color32::BLACK);
    text.text = "Move with hand".into();
    text.size = [160.0, 80.0];
    app.note.as_mut().unwrap().pages[0].texts.push(text);
    app.camera = Camera {
        pan: vec2(30.0, 30.0),
        zoom: 1.0,
        stretch_y: None,
    };
    app.fitted_cell = None;
    frames.settle(app);
    let start = app.camera.to_screen(pos2(240.0, 220.0), app.canvas_rect);
    frames.drag(app, start, start + vec2(50.0, 25.0));
    assert_eq!(
        app.note.as_ref().unwrap().pages[0].texts[0].pos,
        [230.0, 205.0]
    );

    app.tool = Tool::Lasso;
    app.sel.clear();
    frames.settle(app);
    let before = app.note.as_ref().unwrap().pages[0].texts[0].pos;
    let start = app.camera.to_screen(pos2(260.0, 230.0), app.canvas_rect);
    frames.drag(app, start, start + vec2(40.0, 20.0));
    assert_eq!(app.note.as_ref().unwrap().pages[0].texts[0].pos, before);
}

#[test]
fn ink_tone_slider_reaches_white_color_and_black() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Precise ink");
    let app = &mut fixture.app;
    app.open_note(id);
    app.set_ink_panel(true);
    app.color_mixer_open = true;
    app.pick_free_ink(hsv_to_rgb(0.42, 0.2, 0.3));
    let mut frames = Frames::new(app);
    frames.settle(app);
    let channel_at = |frames: &Frames, channel: usize, value| {
        let rect = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new(("ink-slider", channel))))
            .unwrap();
        assert!(rect.width() >= 48.0);
        pos2(
            rect.center().x,
            rect.top() + 16.0 + (rect.height() - 21.0) * value,
        )
    };
    let hue = app.tin_hue;
    for (tone, expected) in [
        (0.0, Color32::WHITE),
        (0.5, hsv_to_rgb(hue, 1.0, 1.0)),
        (1.0, Color32::BLACK),
        (0.5, hsv_to_rgb(hue, 1.0, 1.0)),
    ] {
        let point = channel_at(&frames, 1, tone);
        frames.touch(app, TouchPhase::Start, point);
        frames.touch(app, TouchPhase::End, point);
        for (actual, expected) in app.ink.to_array().into_iter().zip(expected.to_array()) {
            assert!((actual as i16 - expected as i16).abs() <= 2);
        }
    }
    assert!(frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("ink-slider", 2_usize))))
        .is_none());
}

#[test]
fn nib_options_remain_available_outside_the_color_palette() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Nib");
    let app = &mut fixture.app;
    app.open_note(id);
    app.width = 2.0;
    app.nib_open = true;
    app.ink_panel_view = InkPanelView::Nib;
    let mut frames = Frames::new(app);
    frames.settle(app);
    frames.snapshot("ink-palette");
    let dock = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
        .unwrap();
    let tin = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("tin-rect")))
        .unwrap();
    assert!(
        frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("dock-tool-storage-rect")))
            .is_none(),
        "storage stays out of the ink palette"
    );
    assert!(
        (dock.size() - tin.size()).length() < 1.0,
        "size bar matches the toolbar, dock {dock:?} tin {tin:?}"
    );
    let preset = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("nib-preset", 0_usize))))
        .expect("size dots fill the bar");
    frames.touch(app, TouchPhase::Start, preset.center());
    frames.touch(app, TouchPhase::End, preset.center());
    assert!(
        app.width > 10.0,
        "the first dot is a broad nib, got {}",
        app.width
    );
    assert_eq!(app.last_ink_width, app.width);
    assert!(frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("ink-add-color")))
        .is_none());
    let ink = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("ink-well-rect")))
        .unwrap();
    frames.touch(app, TouchPhase::Start, ink.center());
    frames.touch(app, TouchPhase::End, ink.center());
    frames.settle(app);
    assert!(frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("ink-swatch", 8))))
        .is_some());
    assert!(frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("nib-track")))
        .is_none());
    let add = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("ink-add-color")))
        .unwrap();
    frames.drag(app, add.center(), add.center());
    frames.settle(app);
    assert!(app.color_mixer_open);
    let back = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("ink-mixer-back")))
        .unwrap();
    frames.drag(app, back.center(), back.center());
    frames.settle(app);
    assert!(!app.color_mixer_open);
    assert_eq!(app.ink_panel_view, InkPanelView::Colors);
    assert!(frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("tin-wheel")))
        .is_none());
    frames.key(app, Key::Escape);
    assert!(!app.nib_open);
}

#[test]
fn area_eraser_covers_more_paper_when_zoomed_out() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Eraser zoom");
    let app = &mut fixture.app;
    app.open_note(id);
    app.width = 10.0;
    app.tool = Tool::EraserArea;
    app.camera.zoom = 2.0;
    let tight = app.eraser_radius_paper(app.camera.zoom, true);
    app.camera.zoom = 0.5;
    let wide = app.eraser_radius_paper(app.camera.zoom, true);
    assert!(
        (wide / tight - 4.0).abs() < 0.05,
        "halving zoom quadruples paper coverage: tight={tight} wide={wide}"
    );
    let screen = app.eraser_radius_screen(true);
    assert!(
        (screen - 18.0).abs() < 0.05,
        "screen size follows nib width"
    );
}

#[test]
fn vector_spine_wheel_preserves_mouse_and_touch_actions() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Wheel");
    let app = &mut fixture.app;
    let mut frames = Frames::new(app);
    frames.settle(app);
    let center = pos2(600.0, 360.0);
    app.open_spine_wheel(id, center, &frames.ctx, true);
    frames.settle(app);
    frames.snapshot("wheel-vector-actions");
    let color = slice_mid(center, 0, 3, 104.0);
    frames.drag(app, color, color);
    assert!(app.spine_wheel.as_ref().unwrap().colors);
    frames.settle(app);
    frames.snapshot("wheel-cover-colors");
    let cover = slice_mid(center, 3, app.look.cloth.len(), 104.0);
    frames.drag(app, cover, cover);
    assert_eq!(app.lib.load_note(id).unwrap().cover, 3);
    frames.touch(app, TouchPhase::Start, center);
    app.open_spine_wheel(id, center, &frames.ctx, false);
    let mark = slice_mid(center, 2, 3, 104.0);
    frames.touch(app, TouchPhase::Move, mark);
    frames.touch(app, TouchPhase::End, mark);
    assert_eq!(app.emoji_pick, Some(id));
    assert!(!app.lib.is_trashed(id));
}

#[test]
fn home_return_button_saves_and_accepts_mouse_touch_and_keyboard() {
    for input in 0..4 {
        let mut fixture = Fixture::new();
        let id = fixture.notebook("Return");
        let app = &mut fixture.app;
        app.open_note(id);
        app.note.as_mut().unwrap().title = "Saved on return".into();
        app.title_buf = "Saved on return".into();
        app.mark_dirty();
        let mut frames = Frames::new(app);
        frames.settle(app);
        if input == 0 {
            frames.snapshot("note-home-return");
        }
        frames.key(app, Key::Tab);
        let focused = frames
            .ctx
            .memory(|m| m.focused())
            .expect("return button is keyboard reachable");
        let button = frames.ctx.read_response(focused).unwrap().rect;
        assert!(button.min.x < 60.0 && button.max.y <= 52.0);
        assert_eq!(button.size(), vec2(44.0, 44.0));
        match input {
            0 => frames.drag(app, button.center(), button.center()),
            1 => {
                frames.touch(app, TouchPhase::Start, button.center());
                assert!(
                    matches!(app.scene, Scene::Shelf { .. }),
                    "a finger press returns home immediately"
                );
            }
            2 => frames.key(app, Key::Enter),
            _ => frames.key(app, Key::Space),
        }
        assert!(matches!(app.scene, Scene::Shelf { .. }));
        assert!(app.note.is_none());
        assert_eq!(app.lib.load_note(id).unwrap().title, "Saved on return");
        assert!(!app.lib.is_trashed(id));
    }
}

#[test]
fn colored_note_tools_preserve_ink_and_support_mouse_and_touch() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Papeterie");
    let app = &mut fixture.app;
    app.open_note(id);
    let ink = app.ink;
    let paper = app.note.as_ref().unwrap().paper;
    let mut frames = Frames::new(app);
    let tool_center = |frames: &Frames, fill| {
        frames
            .output
            .as_ref()
            .unwrap()
            .shapes
            .iter()
            .find_map(|shape| {
                if let Shape::Rect(rect) = &shape.shape {
                    if rect.fill == fill && (rect.rect.width() - 44.0).abs() < 0.1 {
                        return Some(rect.rect.center());
                    }
                }
                None
            })
            .expect("coloured tool face is rendered")
    };
    for (edge, size, touch) in [
        (DockEdge::Bottom, vec2(1280.0, 860.0), false),
        (DockEdge::Right, vec2(900.0, 1050.0), true),
    ] {
        app.dock_edge = edge;
        app.tool = Tool::Fineliner;
        frames.size = size;
        frames.settle(app);
        let brush = tool_center(&frames, app.tool_well_fill(Tool::Brush, false));
        if touch {
            frames.touch(app, TouchPhase::Start, brush);
            frames.touch(app, TouchPhase::End, brush);
        } else {
            frames.drag(app, brush, brush);
        }
        frames.step(app, vec![Event::PointerMoved(Pos2::ZERO)]);
        frames.settle(app);
        assert_eq!(app.tool, Tool::Brush);
        assert_eq!(
            app.ink, ink,
            "control pigments must not change the drawing ink"
        );
        assert_eq!(app.note.as_ref().unwrap().paper, paper);
        tool_center(&frames, app.tool_well_fill(Tool::Brush, true));
        app.tin_open = true;
        frames.settle(app);
        let dock = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
            .unwrap();
        assert!(Rect::from_min_size(Pos2::ZERO, size).contains_rect(dock));
        let tin = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("tin-rect")))
            .unwrap();
        assert!(Rect::from_min_size(Pos2::ZERO, size).contains_rect(tin));
        assert!(
            !tin.shrink(0.5).intersects(dock),
            "colour wheel must not cover the tools"
        );
        let well = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("ink-well-rect")))
            .unwrap();
        assert!(
            (tin.size() - dock.size()).length() < 1.0,
            "{edge:?}: color twin matches toolbar tin={tin:?} dock={dock:?}"
        );
        if edge.vertical() {
            assert!(
                (tin.center().y - dock.center().y).abs() < 4.0,
                "colour twin follows the vertical toolbar"
            );
        } else {
            assert!(
                (tin.center().x - dock.center().x).abs() < 4.0,
                "colour twin follows the horizontal toolbar"
            );
        }
        let _ = well;
        frames.snapshot(if touch {
            "note-colors-vertical"
        } else {
            "note-colors-horizontal"
        });
    }
    let original_case = app.note_case_color();
    app.note.as_mut().unwrap().cover = 3;
    assert_ne!(app.note_case_color(), original_case);
    let theme_pigment = Color32::from_rgb(85, 130, 170);
    app.look.inks[8] = theme_pigment;
    frames.step(app, vec![]);
    tool_center(&frames, theme_pigment);
    assert_eq!(app.ink, ink);
}

#[test]
fn pages_mode_tear_badge_removes_a_page_and_leaves_ink() {
    for touch in [false, true] {
        let mut fixture = Fixture::new();
        let id = fixture.notebook("Delete pages");
        let app = &mut fixture.app;
        app.open_note(id);
        let mut text = TextBox::new(pos2(80.0, 80.0), Color32::BLACK);
        text.text = "Keep".into();
        let text_id = text.id;
        let mut stroke = InkStroke::new(Nib::Fineliner, Color32::BLACK, 3.0);
        stroke.push(InkPoint::new(pos2(420.0, 360.0), 0.6));
        stroke.push(InkPoint::new(pos2(520.0, 420.0), 0.8));
        stroke.tessellate();
        let stroke_id = stroke.id;
        {
            let note = app.note.as_mut().unwrap();
            note.pages[0].texts.push(text);
            note.pages[0].strokes.push(stroke);
            note.pages.push(crate::document::Page::at(1, 0));
        }
        let before = serde_json::to_value(&app.note.as_ref().unwrap().pages).unwrap();
        let mut frames = Frames::new(app);
        frames.settle(app);
        app.camera = Camera {
            pan: vec2(30.0, 80.0),
            zoom: 0.4,
            stretch_y: None,
        };
        app.fitted_cell = None;
        app.need_fit = false;
        let canvas = app.canvas_rect;
        let paper = app.unit_screen_rect(canvas, 1, 0);
        let minus = page_minus_band(paper);
        let top = outward_band(paper, SheetEdge::Top);
        assert!(
            (minus.width() - top.width()).abs() < 0.5
                && (minus.height() - top.height()).abs() < 0.5,
            "the red minus matches a green + tab"
        );
        let hit = minus.center();
        assert!(canvas.contains(hit), "center minus sits on the canvas");
        app.enter_pages_mode();
        // The press already down when the marks appear must not delete.
        if touch {
            frames.touch(app, TouchPhase::Start, hit);
        } else {
            frames.step(
                app,
                vec![Event::PointerButton {
                    pos: hit,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                }],
            );
        }
        frames.settle(app);
        assert_eq!(app.note.as_ref().unwrap().pages.len(), 2);
        if touch {
            frames.touch(app, TouchPhase::End, hit);
        } else {
            frames.step(
                app,
                vec![Event::PointerButton {
                    pos: hit,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                }],
            );
        }
        assert_eq!(app.note.as_ref().unwrap().pages.len(), 2);
        frames.step(app, vec![]);
        // The canvas grows when the header folds away: recompute the band.
        let paper = app.unit_screen_rect(app.canvas_rect, 1, 0);
        let minus = page_minus_band(paper);
        let hit = minus.center();
        // A stylus tap often leaves the press point. That still deletes.
        let wobble = hit + vec2(18.0, 6.0);
        assert!(minus.contains(wobble));
        if touch {
            frames.touch(app, TouchPhase::Start, hit);
            frames.touch(app, TouchPhase::Move, wobble);
            frames.touch(app, TouchPhase::End, wobble);
        } else {
            frames.drag(app, hit, wobble);
        }
        frames.settle(app);
        let note = app.note.as_ref().unwrap();
        assert_eq!(note.pages.len(), 1);
        assert!(note.unit_occupied(0, 0));
        assert!(!note.unit_occupied(1, 0));
        assert_eq!(note.pages[0].texts[0].id, text_id);
        assert_eq!(note.pages[0].strokes[0].id, stroke_id);
        assert!(app.undo.undo(app.note.as_mut().unwrap()));
        assert_eq!(
            serde_json::to_value(&app.note.as_ref().unwrap().pages).unwrap(),
            before
        );

        app.camera = Camera {
            pan: vec2(30.0, 80.0),
            zoom: 0.4,
            stretch_y: None,
        };
        app.fitted_cell = None;
        app.need_fit = false;
        app.enter_pages_mode();
        frames.settle(app);
        let paper = app.unit_screen_rect(app.canvas_rect, 1, 0);
        let off = paper.left_top() + vec2(paper.width() * 0.08, paper.height() * 0.08);
        assert!(!page_minus_band(paper).contains(off));
        frames.step(app, vec![Event::PointerMoved(off)]);
        frames.step(
            app,
            vec![Event::PointerButton {
                pos: off,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }],
        );
        frames.step(
            app,
            vec![Event::PointerButton {
                pos: off,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
        );
        assert_eq!(app.note.as_ref().unwrap().pages.len(), 2);
        frames.key(app, Key::Escape);
        assert!(!app.sheet_tabs_on);
        assert_eq!(app.note.as_ref().unwrap().pages.len(), 2);
        assert_eq!(app.note.as_ref().unwrap().pages[0].texts[0].id, text_id);
    }
}

#[test]
fn double_tap_on_page_leaves_pages_mode_without_moving_it() {
    for touch in [false, true] {
        let mut fixture = Fixture::new();
        let id = fixture.notebook("Double tap pages");
        let app = &mut fixture.app;
        app.open_note(id);
        let mut frames = Frames::new(app);
        frames.settle(app);
        app.camera = Camera {
            pan: vec2(200.0, 100.0),
            zoom: 0.3,
            stretch_y: None,
        };
        app.fitted_cell = None;
        app.need_fit = false;
        let before = serde_json::to_value(&app.note.as_ref().unwrap().pages).unwrap();
        let canvas = app.canvas_rect;
        let page = app.unit_screen_rect(canvas, 0, 0).center();
        let desk = canvas.left_top() + vec2(25.0, 25.0);
        assert!(app.page_unit_at(desk, canvas).is_none());
        for pos in [page, desk] {
            app.enter_pages_mode();
            frames.settle(app);
            for tap in 0..2 {
                if touch {
                    frames.touch(app, TouchPhase::Start, pos);
                    frames.touch(app, TouchPhase::End, pos);
                } else {
                    for pressed in [true, false] {
                        frames.step(
                            app,
                            vec![Event::PointerButton {
                                pos,
                                button: PointerButton::Primary,
                                pressed,
                                modifiers: Modifiers::NONE,
                            }],
                        );
                    }
                }
                assert_eq!(
                    app.sheet_tabs_on,
                    tap == 0,
                    "exit after second tap at {pos:?} (touch={touch})"
                );
            }
            frames.settle(app);
        }
        assert_eq!(
            serde_json::to_value(&app.note.as_ref().unwrap().pages).unwrap(),
            before
        );
    }
}

#[test]
fn long_press_on_page_enters_pages_mode() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Long press pages");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    assert!(!app.sheet_tabs_on);
    let screen = app
        .camera
        .to_screen(pos2(PAGE_W * 0.5, PAGE_H * 0.5), app.canvas_rect);
    frames.step(
        app,
        vec![
            Event::PointerMoved(screen),
            Event::PointerButton {
                pos: screen,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
    );
    for _ in 0..30 {
        frames.step(app, vec![Event::PointerMoved(screen)]);
    }
    assert!(
        app.sheet_tabs_on,
        "long press on empty paper opens Pages manage mode"
    );
    assert_eq!(app.pages_jiggle_cell, Some((0, 0)));
    assert!(!app.sheet_tabs(app.canvas_rect).is_empty());
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: screen,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
}

#[test]
fn pen_long_press_on_page_enters_pages_mode() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Pen hold pages");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    assert!(!app.sheet_tabs_on);
    let screen = app
        .camera
        .to_screen(pos2(PAGE_W * 0.5, PAGE_H * 0.5), app.canvas_rect);
    // A stylus press on lectern paper never injects Pointer events
    // (inject_pen_pointer only forwards chrome presses), so egui's pointer
    // stays idle; the tablet snapshot alone must drive the pages long-hold.
    let pen = |pressed: bool, down: bool, released: bool| PenSnapshot {
        pos: Some(screen),
        samples: Vec::new(),
        down,
        pressed,
        released,
        eraser: false,
        lasso_btn: false,
        air_toggle: false,
        pressure: Some(0.5),
        in_proximity: true,
        pinch_zoom: 1.0,
        pinch_pan: Vec2::ZERO,
        pinching: false,
    };
    app.tablet.set_test_pen(Some(pen(true, true, false)));
    frames.step(app, vec![]);
    app.tablet.set_test_pen(Some(pen(false, true, false)));
    for _ in 0..30 {
        frames.step(app, vec![]);
    }
    assert!(
        app.sheet_tabs_on,
        "stylus hold on empty paper opens Pages manage mode"
    );
    assert_eq!(app.pages_jiggle_cell, Some((0, 0)));
    assert!(!app.sheet_tabs(app.canvas_rect).is_empty());
    app.tablet.set_test_pen(Some(pen(false, false, true)));
    frames.step(app, vec![]);
    app.tablet.set_test_pen(None);
    frames.settle(app);
    assert!(app.sheet_tabs_on, "released hold keeps Pages manage mode");
}

#[test]
fn finger_long_press_on_page_shows_progress_ring() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Finger hold progress");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    let screen = app
        .camera
        .to_screen(pos2(PAGE_W * 0.5, PAGE_H * 0.5), app.canvas_rect);

    frames.touch(app, TouchPhase::Start, screen);

    assert!(
        !frames.output.as_ref().unwrap().shapes.iter().any(|shape| {
            matches!(
                &shape.shape,
                Shape::Circle(circle)
                    if circle.center.distance(screen) < 0.5
                        && (circle.radius - 44.0).abs() < 0.5
            )
        }),
        "a normal touch does not immediately draw the hold indicator"
    );
    frames.time += HOLD_RING_DELAY + 0.02;
    frames.step(app, vec![]);
    assert!(
        frames.output.as_ref().unwrap().shapes.iter().any(|shape| {
            matches!(
                &shape.shape,
                Shape::Circle(circle)
                    if circle.center.distance(screen) < 0.5
                        && (circle.radius - 44.0).abs() < 0.5
                        && (circle.stroke.width - 2.5).abs() < 0.1
            )
        }),
        "the hold indicator appears only after the finger has paused"
    );
    assert!(
        !app.sheet_tabs_on,
        "the hold must complete before Pages mode"
    );
}

#[test]
fn pages_mode_long_press_drags_page_into_neighbour_slot() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Reorder pages");
    let app = &mut fixture.app;
    app.open_note(id);
    {
        let note = app.note.as_mut().unwrap();
        note.sheet_join = SheetJoin::Separate;
        note.pages.push(crate::document::Page::at(1, 0));
        note.pages[1].texts.push({
            let mut t = TextBox::new(pos2(40.0, 40.0), Color32::BLACK);
            t.text = "Right".into();
            t
        });
    }
    let mut frames = Frames::new(app);
    frames.settle(app);
    app.camera = Camera {
        pan: vec2(40.0, 80.0),
        zoom: 0.35,
        stretch_y: None,
    };
    app.fitted_cell = None;
    app.enter_pages_mode();
    frames.settle(app);
    assert!(app.pages_marks_ready);
    let canvas = app.canvas_rect;
    let paper = app.unit_screen_rect(canvas, 1, 0);
    // Stay off the center − badge so the press arms a page drag, not a tear.
    let from = paper.left_top() + vec2(paper.width() * 0.22, paper.height() * 0.22);
    assert!(!page_minus_band(paper).contains(from));
    frames.step(
        app,
        vec![
            Event::PointerMoved(from),
            Event::PointerButton {
                pos: from,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
    );
    assert_eq!(
        app.pages_jiggle_cell,
        Some((1, 0)),
        "pressing a page makes it the only animated sheet"
    );
    for _ in 0..30 {
        frames.step(app, vec![Event::PointerMoved(from)]);
    }
    assert!(app.page_drag.is_some(), "long press lifts the page");
    let dest = app.unit_screen_rect(canvas, 2, 0).center();
    frames.step(app, vec![Event::PointerMoved(dest)]);
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: dest,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    frames.settle(app);
    let note = app.note.as_ref().unwrap();
    assert!(note.unit_occupied(0, 0));
    assert!(!note.unit_occupied(1, 0));
    assert!(note.unit_occupied(2, 0));
    let moved = note.pages.iter().find(|p| p.col == 2).unwrap();
    assert!(moved.texts.iter().any(|t| t.text.contains("Right")));
    assert!(app.page_drag.is_none());
}

#[test]
fn pages_mode_keeps_selected_sheet_still_and_stops_highlighting_when_left() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Jiggle");
    let app = &mut fixture.app;
    app.open_note(id);
    {
        let note = app.note.as_mut().unwrap();
        note.sheet_join = SheetJoin::Separate;
        note.pages.push(crate::document::Page::at(1, 0));
    }
    let t = 0.42;
    assert_eq!(app.page_jiggle_offset(0, 0, t), Vec2::ZERO);
    app.enter_pages_mode();
    assert_eq!(app.page_jiggle_offset(0, 0, t), Vec2::ZERO);
    assert_eq!(app.page_jiggle_offset(1, 0, t), Vec2::ZERO);
    app.pages_jiggle_cell = Some((1, 0));
    assert_eq!(app.page_jiggle_offset(0, 0, t), Vec2::ZERO);
    assert_eq!(app.page_jiggle_offset(1, 0, t), Vec2::ZERO);
    app.page_drag = Some(PageDrag {
        col: 1,
        row: 0,
        grab: Vec2::ZERO,
        screen: Pos2::ZERO,
    });
    assert_eq!(
        app.page_jiggle_offset(1, 0, t),
        Vec2::ZERO,
        "the lifted sheet follows the pointer"
    );
    assert_eq!(app.page_jiggle_offset(0, 0, t), Vec2::ZERO);
    app.page_drag = None;
    app.leave_pages_mode();
    assert_eq!(app.page_jiggle_offset(0, 0, t), Vec2::ZERO);
}

#[test]
fn pages_mode_finger_drag_pans_without_tearing() {
    for touch in [false, true] {
        let mut fixture = Fixture::new();
        let id = fixture.notebook("Delete zone");
        let app = &mut fixture.app;
        app.open_note(id);
        let mut text = TextBox::new(pos2(80.0, 80.0), Color32::BLACK);
        text.text = "Keep".into();
        let text_id = text.id;
        {
            let note = app.note.as_mut().unwrap();
            note.pages[0].texts.push(text);
            note.pages.push(crate::document::Page::at(1, 0));
        }
        let mut frames = Frames::new(app);
        frames.settle(app);
        app.camera = Camera {
            pan: vec2(30.0, 80.0),
            zoom: 0.4,
            stretch_y: None,
        };
        app.fitted_cell = None;
        app.enter_pages_mode();
        frames.settle(app);
        let canvas = app.canvas_rect;
        let from = app
            .camera
            .to_screen(pos2(PAGE_W * 0.5, PAGE_H * 0.5), canvas);
        let to = from + vec2(80.0, 40.0);
        let pan_before = app.camera.pan;
        if touch {
            frames.touch(app, TouchPhase::Start, from);
            frames.touch(app, TouchPhase::Move, to);
            frames.touch(app, TouchPhase::End, to);
        } else {
            frames.drag(app, from, to);
        }
        frames.settle(app);
        assert!(app.sheet_tabs_on);
        assert_eq!(app.note.as_ref().unwrap().pages.len(), 2);
        assert_eq!(app.note.as_ref().unwrap().pages[0].texts[0].id, text_id);
        assert!(
            (app.camera.pan - pan_before).length() > 1.0,
            "a drag on the paper pans instead of selecting pages"
        );
    }
}

#[test]
fn corner_drop_reopens_smoothly_and_targets_bottom_right() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Drop");
    let app = &mut fixture.app;
    let mut frames = Frames::new(app);
    frames.settle(app);
    for _ in 0..2 {
        app.shelf_haul_ids = vec![id];
        app.shelf_haul_armed = true;
        frames.step(app, vec![]);
        assert!(app.trash_reveal > 0.0 && app.trash_reveal < 1.0);
        frames.settle(app);
        assert_eq!(app.trash_reveal, 1.0);
        frames.snapshot("corner-drop");
        let screen = Rect::from_min_size(Pos2::ZERO, frames.size);
        assert!(app.trash_rect.min.x >= screen.right() - BIN_DROP_RADIUS);
        assert!(app.over_trash_drop_zone(screen.right_bottom() - vec2(75.0, 75.0), screen, 0.0));
        assert!(!app.over_trash_drop_zone(screen.left_bottom() + vec2(75.0, -75.0), screen, 0.0));
        assert!(!app.over_trash_drop_zone(screen.right_bottom() - vec2(170.0, 170.0), screen, 0.0));
        app.shelf_haul_armed = false;
        frames.step(app, vec![]);
        assert!(app.trash_reveal > 0.0 && app.trash_reveal < 1.0);
        frames.settle(app);
        assert_eq!(app.trash_reveal, 0.0);
    }
}

#[test]
fn emoji_picker_reopens_above_backdrop_and_accepts_mouse_and_touch() {
    let mut fixture = Fixture::new();
    let first = fixture.notebook("First");
    let second = fixture.notebook("Second");
    let app = &mut fixture.app;
    app.emoji_group = crate::emoji::EmojiGroup::SmileysPeople;
    let expected = app
        .emoji
        .group_chars(crate::emoji::EmojiGroup::SmileysPeople)
        .first()
        .expect("emoji font")
        .to_string();
    let mut frames = Frames::new(app);
    frames.settle(app);
    for id in [first, second, first] {
        app.emoji_pick = Some(id);
        // Progressive decode fills the first cell within a few frames.
        for _ in 0..30 {
            frames.step(app, vec![]);
            if app
                .emoji_tex
                .contains_key(&expected.chars().next().unwrap())
            {
                break;
            }
        }
        let outside = pos2(4.0, 4.0);
        frames.drag(app, outside, outside);
        assert!(app.emoji_pick.is_none());
        frames.settle(app);

        app.emoji_pick = Some(id);
        for _ in 0..30 {
            frames.step(app, vec![]);
        }
        let cell = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("emoji-first-cell")))
            .expect("first emoji cell")
            .center();
        assert_eq!(
            frames.ctx.layer_id_at(cell),
            Some(LayerId::new(Order::Foreground, Id::new("emoji-pick")))
        );
        if id == second {
            frames.touch(app, TouchPhase::Start, cell);
            frames.touch(app, TouchPhase::End, cell);
        } else {
            frames.drag(app, cell, cell);
        }
        assert!(
            app.emoji_pick.is_none(),
            "emoji cell must receive the click"
        );
        assert_eq!(app.lib.load_note(id).unwrap().emoji, expected);
        frames.settle(app);
    }
    assert!(
        app.emoji_tex.len() < app.emoji.catalog_len() / 2,
        "opening must not decode the whole catalog"
    );
}

#[test]
fn emoji_picker_fits_small_window_and_escape_closes_it() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Small window");
    let app = &mut fixture.app;
    let mut frames = Frames::new(app);
    frames.size = vec2(360.0, 320.0);
    app.emoji_pick = Some(id);
    frames.settle(app);
    let sheet = frames
        .ctx
        .memory(|m| m.area_rect(Id::new("emoji-pick")))
        .unwrap();
    assert!(Rect::from_min_size(Pos2::ZERO, frames.size).contains_rect(sheet));
    frames.key(app, Key::Escape);
    assert!(app.emoji_pick.is_none());
}

#[test]
fn drawer_close_button_closes_without_deleting_and_can_reopen() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Keep in trash");
    let app = &mut fixture.app;
    app.lib.trash_note(id).expect("trash notebook");
    app.shelf_trash = true;
    let mut frames = Frames::new(app);
    frames.settle(app);
    frames.snapshot("bin-drawer-actions");
    let height = app.bin_panel_height;
    let close = app.bin_close_rect.center();
    assert!(app.bin_close_rect.height() >= 44.0);
    assert!(app.bin_close_rect.min.y > app.trash_rect.center().y);
    frames.drag(app, close, close);
    assert!(!app.shelf_trash);
    assert!(!app.shelf_band_armed);
    assert!(app.lib.is_trashed(id));
    frames.step(app, vec![]);
    assert!(app.bin_panel_height > 0.0 && app.bin_panel_height < height);
    frames.snapshot("bin-drawer-closing");
    frames.settle(app);
    assert_eq!(app.bin_panel_height, 0.0);
    assert!(app.trash_grid.is_empty());
    let basket = app.trash_rect.center();
    frames.drag(app, basket, basket);
    assert!(app.shelf_trash);
    frames.settle(app);
    assert_eq!(app.bin_panel_height, height);
    assert!(app.lib.is_trashed(id));
}

#[test]
fn drawer_close_button_accepts_touch() {
    let mut fixture = Fixture::new();
    let app = &mut fixture.app;
    app.shelf_trash = true;
    let mut frames = Frames::new(app);
    frames.settle(app);
    let close = app.bin_close_rect.center();
    frames.touch(app, TouchPhase::Start, close);
    frames.touch(app, TouchPhase::End, close);
    assert!(!app.shelf_trash);
    frames.settle(app);
    assert_eq!(app.bin_panel_height, 0.0);
}

#[test]
fn drawer_keyboard_close_respects_modal_priority_and_selected_notebooks() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Selected");
    let app = &mut fixture.app;
    let mut frames = Frames::new(app);
    app.shelf_trash = true;
    frames.settle(app);
    app.emoji_pick = Some(id);
    frames.key(app, Key::Escape);
    assert!(app.emoji_pick.is_none());
    assert!(app.shelf_trash);
    for key in [Key::Escape, Key::Enter, Key::Space] {
        app.shelf_trash = true;
        app.shelf_sel = vec![id];
        frames.settle(app);
        if key == Key::Enter {
            for _ in 0..32 {
                frames.key(app, Key::Tab);
                if frames.ctx.memory(|m| m.has_focus(Id::new("close-trash"))) {
                    break;
                }
            }
            assert!(frames.ctx.memory(|m| m.has_focus(Id::new("close-trash"))));
        } else if key == Key::Space {
            frames
                .ctx
                .memory_mut(|m| m.request_focus(Id::new("close-trash")));
        }
        frames.key(app, key);
        assert!(!app.shelf_trash, "{key:?} should close the drawer");
        assert!(matches!(app.scene, Scene::Shelf { .. }));
        assert!(!app.lib.is_trashed(id));
        frames.settle(app);
        assert_eq!(app.bin_panel_height, 0.0);
    }
}

#[test]
fn notebook_drag_over_close_button_never_dismisses_drawer() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Move past grip");
    let app = &mut fixture.app;
    app.shelf_trash = true;
    let mut frames = Frames::new(app);
    frames.settle(app);
    let source = app
        .shelf_slots
        .iter()
        .find(|(note, _)| *note == id)
        .unwrap()
        .1
        .center();
    let close = app.bin_close_rect.center();
    frames.drag(app, source, close);
    assert!(app.shelf_trash);
    assert!(!app.lib.is_trashed(id));
    let target = app.trash_grid[0].center();
    frames.drag(app, source, target);
    assert!(app.shelf_trash);
    assert!(app.lib.is_trashed(id));
}

#[test]
fn management_drag_moves_between_home_and_bin_slots() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Move");
    let app = &mut fixture.app;
    app.shelf_trash = true;
    let mut frames = Frames::new(app);
    frames.settle(app);
    frames.snapshot("bin-management-empty");
    let source = app
        .shelf_slots
        .iter()
        .find(|(note, _)| *note == id)
        .unwrap()
        .1
        .center();
    let bin = app.trash_grid[1].center();
    frames.drag(app, source, bin);
    assert_eq!(
        app.lib
            .index
            .trash
            .iter()
            .find(|n| n.id == id)
            .unwrap()
            .slot,
        1
    );
    assert_eq!(app.trash_grid.len(), TRASH_SLOTS as usize);
    assert_eq!(app.trash_reveal, 0.0);
    frames.snapshot("bin-management-full");
    let source = app
        .shelf_slots
        .iter()
        .find(|(note, _)| *note == id)
        .unwrap()
        .1
        .center();
    let bin = app.trash_grid[2].center();
    frames.drag(app, source, bin);
    assert_eq!(
        app.lib
            .index
            .trash
            .iter()
            .find(|n| n.id == id)
            .unwrap()
            .slot,
        2
    );
    let source = app
        .shelf_slots
        .iter()
        .find(|(note, _)| *note == id)
        .unwrap()
        .1
        .center();
    let home = app.shelf_grid[1].center();
    frames.drag(app, source, home);
    assert!(!app.lib.is_trashed(id));
    assert_eq!(
        app.lib
            .index
            .notes
            .iter()
            .find(|n| n.id == id)
            .unwrap()
            .slot,
        1
    );
}

#[test]
fn home_drop_uses_corner_until_toss_completes() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Throw");
    let app = &mut fixture.app;
    let mut frames = Frames::new(app);
    frames.settle(app);
    let source = app
        .shelf_slots
        .iter()
        .find(|(note, _)| *note == id)
        .unwrap()
        .1
        .center();
    let target = frames.size.to_pos2() - vec2(75.0, 75.0);
    frames.drag(app, source, target);
    assert_eq!(app.shelf_toss.len(), 1);
    assert!(!app.lib.is_trashed(id));
    assert_eq!(app.trash_reveal, 1.0);
    for _ in 0..10 {
        frames.step(app, vec![]);
    }
    assert_eq!(app.shelf_toss.len(), 1);
    assert_eq!(app.trash_reveal, 1.0);
    frames.snapshot("corner-toss");
    for _ in 0..40 {
        frames.step(app, vec![]);
    }
    assert!(app.shelf_toss.is_empty());
    assert!(app.lib.is_trashed(id));
    assert_eq!(app.trash_reveal, 0.0);
}

#[test]
fn linked_paper_layers_share_pixel_aligned_edges() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Continuous paper");
    let app = &mut fixture.app;
    app.open_note(id);
    let note = app.note.as_mut().unwrap();
    note.pages
        .extend([(1, 0), (0, 1), (1, 1)].map(|(x, y)| crate::document::Page::at(x, y)));
    note.paper = PaperKind::Grid;
    let ctx = Context::default();
    let canvas = Rect::from_min_size(Pos2::ZERO, vec2(2400.0, 2400.0));
    for density in [1.0, 1.25, 2.0] {
        ctx.set_pixels_per_point(density);
        for zoom in [0.125, 0.333, 0.625] {
            app.camera = Camera {
                pan: vec2(33.37, 47.19),
                zoom,
                stretch_y: None,
            };
            let output = ctx.run(
                RawInput {
                    screen_rect: Some(canvas),
                    ..Default::default()
                },
                |ctx| {
                    let painter = ctx
                        .layer_painter(LayerId::background())
                        .with_clip_rect(canvas);
                    app.paint_world(&painter, canvas);
                },
            );
            let mut surfaces = Vec::new();
            let mut started_grain = false;
            for shape in &output.shapes {
                if let egui::epaint::Shape::Rect(rect) = &shape.shape {
                    if rect.brush.is_some() {
                        started_grain = true;
                        surfaces.push(shape.clip_rect);
                    } else if rect.fill == app.look.paper {
                        assert!(
                            !started_grain,
                            "opaque paper must never cover grain or ruling"
                        );
                    }
                }
            }
            assert_eq!(surfaces.len(), 4);
            assert_eq!(surfaces[0].max.x, surfaces[1].min.x);
            assert_eq!(surfaces[0].max.y, surfaces[2].min.y);
            assert_eq!(surfaces[1].max.y, surfaces[3].min.y);
            assert_eq!(surfaces[2].max.x, surfaces[3].min.x);
            for edge in surfaces
                .iter()
                .flat_map(|r| [r.min.x, r.min.y, r.max.x, r.max.y])
            {
                let pixel = edge * output.pixels_per_point;
                assert!(
                    (pixel - pixel.round()).abs() < 0.001,
                    "fractional scissor edge: {pixel}"
                );
            }
        }
    }
}

#[test]
fn linked_canvas_keeps_selected_page_when_resized() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Linked");
    let app = &mut fixture.app;
    app.open_note(id);
    let note = app.note.as_mut().unwrap();
    note.pages.push(crate::document::Page::at(1, 0));
    note.paper = PaperKind::Grid;
    let mut frames = Frames::new(app);
    frames.settle(app);
    app.camera
        .fit_page(app.canvas_rect, app.origin_of(1), app.pw(), app.ph());
    app.fit_to_screen();
    for (name, size) in [
        ("linked-wide", vec2(1400.0, 740.0)),
        ("linked-tall", vec2(820.0, 1050.0)),
    ] {
        frames.size = size;
        frames.settle(app);
        assert_eq!(app.fitted_cell, Some((1, 0)));
        assert_eq!(app.page_in_view(app.canvas_rect), 1);
        assert_uniform_page_fit(app, app.origin_of(1), app.pw(), app.ph());
        frames.snapshot(name);
    }
}

fn assert_uniform_page_fit(app: &InkarchyApp, origin: Vec2, page_w: f32, page_h: f32) {
    let rect = app.canvas_rect;
    assert!(app.camera.stretch_y.is_none());
    let page = Rect::from_min_max(
        app.camera.to_screen(origin.to_pos2(), rect),
        app.camera
            .to_screen((origin + vec2(page_w, page_h)).to_pos2(), rect),
    );
    assert!((page.width() / page.height() - page_w / page_h).abs() < 0.002);
    assert!(page.min.x <= rect.min.x + 0.05 && page.max.x >= rect.max.x - 0.05);
    assert!(page.min.y <= rect.min.y + 0.05 && page.max.y >= rect.max.y - 0.05);
    let fills_x = (page.width() - rect.width()).abs() < 0.05;
    let fills_y = (page.height() - rect.height()).abs() < 0.05;
    assert!(fills_x || fills_y, "{page:?} vs {rect:?}");
    if !fills_x {
        assert!(page.min.x < rect.min.x && page.max.x > rect.max.x);
        assert!((page.center().x - rect.center().x).abs() < 0.05);
    }
    if !fills_y {
        assert!(page.min.y < rect.min.y && page.max.y > rect.max.y);
        assert!((page.center().y - rect.center().y).abs() < 0.05);
    }
}

#[test]
fn dock_expands_around_its_destination_without_a_second_slide() {
    for edge in [
        DockEdge::Top,
        DockEdge::Bottom,
        DockEdge::Left,
        DockEdge::Right,
    ] {
        let mut fixture = Fixture::new();
        let id = fixture.notebook("Dock expansion");
        let app = &mut fixture.app;
        app.open_note(id);
        app.dock_edge = edge;
        let mut frames = Frames::new(app);
        frames.settle(app);
        app.dock_fold = 1.0;
        app.dock_float = Some(dock_rest_min(
            edge,
            app.dock_pill_size(edge.vertical()),
            dock_bounds(&frames.ctx, edge),
        ));
        app.dock_slide = Some(DockSlide {
            vel: Vec2::ZERO,
            shown: true,
            arrived: true,
        });
        for _ in 0..40 {
            frames.step(app, vec![]);
            let rect = frames
                .ctx
                .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
                .unwrap();
            let expected = dock_rest_min(edge, rect.size(), dock_bounds(&frames.ctx, edge));
            assert!(
                rect.min.distance(expected) < 1.5,
                "{edge:?}: expansion lost its anchor: {rect:?}, expected {expected:?}, fold {}",
                app.dock_fold
            );
        }
        assert_eq!(app.dock_fold, 0.0);
        assert!(app.dock_slide.is_none());
    }
}

#[test]
fn tool_storage_handles_small_screens_cancel_and_tap_to_restore() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Small tool case");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.size = vec2(360.0, 400.0);
    frames.settle(app);
    let source = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("tool-well", Tool::Fineliner))))
        .unwrap()
        .center();
    frames.touch(app, TouchPhase::Start, source);
    frames.touch(app, TouchPhase::Move, source - vec2(0.0, 40.0));
    frames.settle(app);
    let storage = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("dock-tool-storage-rect")))
        .unwrap();
    assert!(
        storage.width() >= 48.0 && storage.height() >= 22.0,
        "tray scrolls into view while carrying a tool: {storage:?}"
    );
    frames.touch(app, TouchPhase::Move, storage.center());
    frames.touch(app, TouchPhase::Cancel, storage.center());
    assert_eq!(app.parked_count(), 0, "cancel never parks a tool");
    assert!(app.dock_tool_drag.is_none());
    frames.settle(app);
    let source = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("tool-well", Tool::Fineliner))))
        .unwrap()
        .center();
    frames.touch(app, TouchPhase::Start, source);
    frames.touch(app, TouchPhase::Move, storage.center());
    frames.touch(app, TouchPhase::End, storage.center());
    frames.settle(app);
    assert!(app.tool_is_parked(Tool::Fineliner));
    let stored = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new(("tool-well", Tool::Fineliner))))
        .unwrap()
        .center();
    frames.touch(app, TouchPhase::Start, stored);
    frames.touch(app, TouchPhase::End, stored);
    assert_eq!(app.parked_count(), 0, "tap restores the stored tool");
}
