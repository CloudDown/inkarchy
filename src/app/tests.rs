use super::*;
mod gestures;

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
        app.tool = Tool::Lasso;
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
        let duplicate = frames.text_center("Duplicate");
        frames.drag(app, duplicate, duplicate);
        frames.settle(app);
        assert_eq!(app.note.as_ref().unwrap().pages[0].texts.len(), 2);
        let delete = frames.text_center("Delete");
        frames.drag(app, delete, delete);
        frames.settle(app);
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
        self.time += 1.0 / 60.0;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
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
        assert_eq!(app.zoom_percent(), 100);
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
    assert_eq!(app.zoom_percent(), 100);
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
        for size in [
            vec2(1280.0, 860.0),
            vec2(320.0, 280.0),
            vec2(360.0, 640.0),
            vec2(1024.0, 320.0),
            vec2(760.0, 560.0),
            vec2(600.0, 400.0),
        ] {
            frames.size = size;
            for accessory in 0..2 {
                app.tin_open = accessory == 1;
                frames.settle(app);
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
                if edge == DockEdge::Bottom && size.x == 320.0 && accessory == 1 {
                    frames.snapshot("responsive-note-small");
                }
            }
        }
    }
}

/// Header wells, rightmost first. Index 0 is Pages, 1 is the notebook menu.
fn header_wells(frames: &Frames) -> Vec<Pos2> {
    let mut wells: Vec<Rect> = frames
        .output
        .as_ref()
        .unwrap()
        .shapes
        .iter()
        .filter_map(|shape| {
            let Shape::Rect(rect) = &shape.shape else {
                return None;
            };
            if rect.fill.a() == 255
                && (rect.rect.height() - 40.0).abs() < 0.1
                && rect.rect.top() < 52.0
            {
                Some(rect.rect)
            } else {
                None
            }
        })
        .collect();
    wells.sort_by(|a, b| b.right().total_cmp(&a.right()));
    wells.into_iter().map(|rect| rect.center()).collect()
}

#[test]
fn pages_mode_adds_a_sheet_and_cancel_hides_the_pluses() {
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
    let manage = header_wells(&frames)[0];
    frames.drag(app, manage, manage);
    frames.settle(app);
    assert!(app.object_delete.is_some());
    let canvas = app.canvas_rect;
    let tabs = app.sheet_tabs(canvas);
    assert!(!tabs.is_empty(), "Pages mode shows the + tabs");
    let hit = tabs
        .iter()
        .map(|(_, _, rect)| rect.center())
        .find(|pos| canvas.contains(*pos))
        .expect("a + tab sits on the canvas");
    frames.drag(app, hit, hit);
    frames.settle(app);
    assert_eq!(
        app.note.as_ref().unwrap().pages.len(),
        pages + 1,
        "a + tab adds a page while managing"
    );
    let cancel = frames.text_center("Cancel");
    frames.drag(app, cancel, cancel);
    frames.settle(app);
    assert!(app.object_delete.is_none());
    assert!(
        app.sheet_tabs(app.canvas_rect).is_empty(),
        "leaving Pages mode hides the + tabs"
    );
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
    assert!(!app.lib.load_note(id).unwrap().pinned);
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
    app.start_object_delete();
    frames.settle(app);
    let cancel = frames.text_center("Cancel");
    let delete = frames.text_center("Delete (0)");
    assert!(cancel.x < delete.x && cancel.y < 78.0 && delete.y < 78.0);
    frames.snapshot("responsive-page-selection-small");
    frames.touch(app, TouchPhase::Start, cancel);
    frames.touch(app, TouchPhase::End, cancel);
    assert!(app.object_delete.is_none());
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
    for _ in 0..50 {
        frames.step(app, vec![]);
    }
    assert!(app.dock_float.is_none(), "slide settles");
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
fn sticky_pad_drags_a_yellow_note_onto_the_page() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Stickies");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    let pad = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("sticky-pad-rect")))
        .expect("sticky pad");
    let canvas = app.canvas_rect;
    let drop = canvas.center();
    frames.drag(app, pad.center(), drop);
    frames.settle(app);
    let texts = &app.note.as_ref().unwrap().pages[0].texts;
    assert_eq!(texts.len(), 1);
    assert!(texts[0].sticky);
    assert_eq!(texts[0].color32(), app.look.yellow());
    assert_eq!(app.editing_text.map(|(p, _)| p), Some(0));
}

#[test]
fn long_press_on_a_sticky_uses_the_folder_cloth_wheel() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Sticky color");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    let pad = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("sticky-pad-rect")))
        .expect("sticky pad");
    frames.step(
        app,
        vec![
            Event::PointerMoved(pad.center()),
            Event::PointerButton {
                pos: pad.center(),
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
    );
    for _ in 0..30 {
        frames.step(app, vec![]);
    }
    assert!(
        app.sticky_wheel.is_none(),
        "the pad icon does not open color"
    );
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: pad.center(),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );

    let canvas = app.canvas_rect;
    let drop = canvas.center();
    frames.drag(app, pad.center(), drop);
    frames.settle(app);
    app.finish_text_edit();
    let screen = {
        let tx = &app.note.as_ref().unwrap().pages[0].texts[0];
        let paper = tx.rect().center() + app.origin_of(0);
        app.camera.to_screen(paper, app.canvas_rect)
    };
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
    let wheel = app
        .sticky_wheel
        .as_ref()
        .expect("long press opens the wheel");
    assert!(!wheel.secondary);
    let cloth = (0..app.look.cloth.len())
        .find(|&i| app.look.cloth_at(i as u8) != app.look.yellow())
        .expect("a cloth color besides yellow") as u8;
    let n = app.look.cloth.len().max(1);
    let (origin, scale) = fitted_wheel(frames.ctx.screen_rect(), wheel.origin);
    let aim = slice_mid(origin, cloth as usize, n, (WHEEL_IN + WHEEL_OUT) * 0.5);
    let aim = origin + (aim - origin) * scale;
    frames.step(app, vec![Event::PointerMoved(aim)]);
    frames.step(
        app,
        vec![
            Event::PointerMoved(aim),
            Event::PointerButton {
                pos: aim,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            },
        ],
    );
    assert!(app.sticky_wheel.is_none());
    let tx = &app.note.as_ref().unwrap().pages[0].texts[0];
    assert_eq!(tx.color32(), app.look.cloth_at(cloth));
    assert_eq!(app.sticky_color, app.look.cloth_at(cloth));
}

#[test]
fn finger_hold_on_a_sticky_opens_the_cloth_wheel() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Sticky finger");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    let tx = TextBox::sticky(pos2(400.0, 300.0), app.look.yellow());
    app.note.as_mut().unwrap().pages[0].texts.push(tx);
    frames.settle(app);
    let screen = {
        let tx = &app.note.as_ref().unwrap().pages[0].texts[0];
        app.camera
            .to_screen(tx.rect().center() + app.origin_of(0), app.canvas_rect)
    };
    frames.touch(app, TouchPhase::Start, screen);
    for _ in 0..30 {
        frames.step(app, vec![]);
    }
    assert!(
        app.sticky_wheel.is_some(),
        "a still finger on the sticky opens the cloth wheel"
    );
    frames.touch(app, TouchPhase::End, screen);
    assert!(
        app.sticky_wheel.is_some(),
        "lifting in the hole leaves the wheel up for a tap"
    );
    let cloth = (0..app.look.cloth.len())
        .find(|&i| app.look.cloth_at(i as u8) != app.look.yellow())
        .expect("a cloth color besides yellow") as u8;
    let n = app.look.cloth.len().max(1);
    let (origin, scale) = fitted_wheel(
        frames.ctx.screen_rect(),
        app.sticky_wheel.as_ref().unwrap().origin,
    );
    let aim = slice_mid(origin, cloth as usize, n, (WHEEL_IN + WHEEL_OUT) * 0.5);
    let aim = origin + (aim - origin) * scale;
    frames.touch(app, TouchPhase::Start, aim);
    frames.touch(app, TouchPhase::End, aim);
    assert!(app.sticky_wheel.is_none());
    assert_eq!(
        app.note.as_ref().unwrap().pages[0].texts[0].color32(),
        app.look.cloth_at(cloth)
    );
}

#[test]
fn sticky_tap_fills_the_sheet_and_a_corner_drag_resizes_it() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Sticky edit");
    let app = &mut fixture.app;
    app.open_note(id);
    let tx = TextBox::sticky(pos2(180.0, 160.0), app.look.yellow());
    let sticky_id = tx.id;
    app.note.as_mut().unwrap().pages[0].texts.push(tx);
    let mut frames = Frames::new(app);
    frames.settle(app);
    let screen = {
        let tx = &app.note.as_ref().unwrap().pages[0].texts[0];
        app.camera
            .to_screen(tx.rect().center() + app.origin_of(0), app.canvas_rect)
    };
    frames.step(app, vec![Event::PointerMoved(screen)]);
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: screen,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
    );
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: screen,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert_eq!(app.sticky_sheet, Some((0, sticky_id)));
    frames.settle(app);
    frames.step(app, vec![Event::Text("Hello".into())]);
    frames.settle(app);
    assert!(
        app.note.as_ref().unwrap().pages[0].texts[0]
            .text
            .contains("Hello"),
        "the sheet accepts typing"
    );
    let done = frames.text_center("Done");
    frames.step(app, vec![Event::PointerMoved(done)]);
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: done,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
    );
    frames.step(
        app,
        vec![Event::PointerButton {
            pos: done,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert!(app.sticky_sheet.is_none());
    assert!(app.note.as_ref().unwrap().pages[0].texts[0]
        .text
        .contains("Hello"));

    let before = app.note.as_ref().unwrap().pages[0].texts[0].size;
    let corner = {
        let tx = &app.note.as_ref().unwrap().pages[0].texts[0];
        app.camera
            .to_screen(tx.rect().right_bottom() + app.origin_of(0), app.canvas_rect)
    };
    frames.drag(app, corner - vec2(6.0, 6.0), corner + vec2(70.0, 36.0));
    let after = app.note.as_ref().unwrap().pages[0].texts[0].size;
    assert!(after[0] > before[0] + 20.0, "wider {before:?} -> {after:?}");
    assert!(
        after[1] > before[1] + 10.0,
        "taller {before:?} -> {after:?}"
    );

    let corner = {
        let tx = &app.note.as_ref().unwrap().pages[0].texts[0];
        app.camera
            .to_screen(tx.rect().right_bottom() + app.origin_of(0), app.canvas_rect)
    };
    let finger = corner - vec2(8.0, 8.0);
    let pull = finger + vec2(40.0, 24.0);
    let mid = app.note.as_ref().unwrap().pages[0].texts[0].size;
    frames.touch(app, TouchPhase::Start, finger);
    frames.touch(app, TouchPhase::Move, pull);
    frames.touch(app, TouchPhase::End, pull);
    let end = app.note.as_ref().unwrap().pages[0].texts[0].size;
    assert!(end[0] > mid[0] + 10.0, "finger widens {mid:?} -> {end:?}");
    assert!(end[1] > mid[1] + 8.0, "finger heightens {mid:?} -> {end:?}");
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
fn notebook_menu_keeps_actions_and_keyboard_dismissal() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Notebook menu");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    let more = header_wells(&frames)[1];
    frames.drag(app, more, more);
    frames.settle(app);
    assert!(Popup::is_any_open(&frames.ctx));
    let last = frames.text_center("Move to trash");
    frames.text_center("Stroke eraser");
    frames.text_center("Area eraser");
    assert!(
        frames.output.as_ref().unwrap().shapes.iter().any(|shape| {
            matches!(&shape.shape, Shape::Text(text) if text.galley.text() == "Move to trash")
                && shape.clip_rect.contains(last)
        }),
        "all menu actions should fit in a normal window"
    );
    frames.snapshot("notebook-menu");
    let download = frames.text_center("Download…");
    frames.touch(app, TouchPhase::Start, download);
    frames.touch(app, TouchPhase::End, download);
    frames.settle(app);
    assert!(app.export_picker_open);
    frames.text_center("PNG image");
    frames.text_center("PDF document");
    frames.snapshot("download-canson");
    let cancel = frames.text_center("Cancel");
    frames.drag(app, cancel, cancel);
    frames.settle(app);
    assert!(!app.export_picker_open);
    frames.drag(app, more, more);
    frames.settle(app);
    let separate = frames.text_center("Separate pages");
    frames.drag(app, separate, separate);
    assert_eq!(app.note.as_ref().unwrap().sheet_join, SheetJoin::Separate);
    frames.drag(app, more, more);
    frames.settle(app);
    frames.key(app, Key::Escape);
    assert!(!Popup::is_any_open(&frames.ctx));
    assert!(matches!(app.scene, Scene::Desk));
    frames.size.y = 400.0;
    frames.settle(app);
    frames.drag(app, more, more);
    frames.settle(app);
    assert!(Popup::is_any_open(&frames.ctx));
    frames.snapshot("notebook-menu-short");
    let save = frames.text_center("Download…");
    assert!(save.y > 0.0 && save.y < frames.size.y);
    let mut reached_last = false;
    for _ in 0..48 {
        frames.key(app, Key::Tab);
        frames.settle(app);
        let last = frames.text_center("Move to trash");
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
fn ink_tin_keeps_color_picking_and_fits_after_resize() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Ink tin");
    let app = &mut fixture.app;
    app.open_note(id);
    app.tin_open = true;
    let mut frames = Frames::new(app);
    frames.settle(app);
    let wheel = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("tin-wheel")))
        .unwrap();
    let geom = ink_wheel_geom(wheel);
    let hue_at = geom.hue_pos(0.25);
    frames.drag(app, hue_at, hue_at);
    assert!(
        (app.tin_hue - 0.25).abs() < 0.04,
        "hue ring sets tin_hue, got {}",
        app.tin_hue
    );
    assert!(app.tin_open);
    let sv = geom.sv_pos(0.65, 0.8);
    frames.touch(app, TouchPhase::Start, sv);
    frames.touch(app, TouchPhase::End, sv);
    let expect = hsv_to_rgb(app.tin_hue, 0.65, 0.8);
    assert_eq!(app.ink, expect);
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
    frames.snapshot("ink-tin");
    frames.size = vec2(900.0, 500.0);
    frames.settle(app);
    let tin = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("tin-rect")))
        .unwrap();
    assert!(Rect::from_min_size(Pos2::ZERO, frames.size).contains_rect(tin));
    frames.snapshot("ink-tin-short");
    frames.key(app, Key::Escape);
    assert!(!app.tin_open);
    assert!(matches!(app.scene, Scene::Desk));
}

#[test]
fn ink_fields_use_continuous_meshes() {
    let ctx = Context::default();
    let output = ctx.run(RawInput::default(), |ctx| {
        let painter = ctx.layer_painter(LayerId::background());
        paint_sv_disk(&painter, pos2(102.0, 102.0), 96.0, 0.5);
        paint_hue_ring(&painter, pos2(102.0, 102.0), 108.0, 136.0);
    });
    let meshes: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let Shape::Mesh(mesh) = &shape.shape {
                Some(mesh)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(meshes.len(), 2, "one continuous mesh per colour field");
    for mesh in &meshes {
        assert!(mesh.is_valid());
    }
    assert_eq!(meshes[0].vertices.first().unwrap().color, Color32::WHITE);
    assert_eq!(meshes[0].vertices.last().unwrap().color, Color32::BLACK);
    assert_eq!(
        meshes[1].vertices.first().unwrap().color,
        meshes[1].vertices.last().unwrap().color
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
    let pin = slice_mid(center, 0, 4, 104.0);
    frames.drag(app, pin, pin);
    assert!(app.lib.load_note(id).unwrap().pinned);
    app.open_spine_wheel(id, center, &frames.ctx, true);
    frames.settle(app);
    let color = slice_mid(center, 1, 4, 104.0);
    frames.drag(app, color, color);
    assert!(app.spine_wheel.as_ref().unwrap().colors);
    frames.settle(app);
    frames.snapshot("wheel-cover-colors");
    let cover = slice_mid(center, 3, app.look.cloth.len(), 104.0);
    frames.drag(app, cover, cover);
    assert_eq!(app.lib.load_note(id).unwrap().cover, 3);
    frames.touch(app, TouchPhase::Start, center);
    app.open_spine_wheel(id, center, &frames.ctx, false);
    let mark = slice_mid(center, 3, 4, 104.0);
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
                    if rect.fill == fill && (rect.rect.width() - 40.0).abs() < 0.1 {
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
fn fold_tears_the_outer_page() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Tear ear");
    let app = &mut fixture.app;
    app.open_note(id);
    app.note.as_mut().unwrap().add_unit_at(1, 0);
    let mut frames = Frames::new(app);
    frames.settle(app);
    app.camera = Camera {
        pan: vec2(8.0, 8.0),
        zoom: 0.5,
        stretch_y: None,
    };
    app.fitted_cell = None;
    app.need_fit = false;
    let paper = app.unit_screen_rect(app.canvas_rect, 1, 0);
    let ear = paper.right_top() + vec2(-4.0, 4.0);
    frames.drag(app, ear, ear);
    assert_eq!(app.note.as_ref().unwrap().pages.len(), 1);
    assert!(app.note.as_ref().unwrap().unit_occupied(0, 0));
    assert!(app.undo.undo(app.note.as_mut().unwrap()));
    assert_eq!(app.note.as_ref().unwrap().pages.len(), 2);
}

fn delete_column_cloth(frames: &Frames) -> Rect {
    fn consider(shape: &Shape, screen_w: f32, best: &mut Option<Rect>) {
        match shape {
            Shape::Rect(rect) if rect.corner_radius.nw == 22 => {
                let r = rect.rect;
                // The column is the cloth flush with the right margin. Its
                // height changes with the rows, so the tallest rect is not it.
                let on_the_right = (r.right() - (screen_w - 12.0)).abs() < 3.0 && r.width() > 150.0;
                let higher = best.is_none_or(|have| r.top() < have.top());
                if on_the_right && higher {
                    *best = Some(r);
                }
            }
            Shape::Vec(list) => {
                for shape in list {
                    consider(shape, screen_w, best);
                }
            }
            _ => {}
        }
    }
    let mut best = None;
    for shape in &frames.output.as_ref().unwrap().shapes {
        consider(&shape.shape, frames.size.x, &mut best);
    }
    best.expect("delete column cloth")
}

#[test]
fn delete_column_height_follows_the_row_count() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Delete height");
    let app = &mut fixture.app;
    app.open_note(id);
    let mut frames = Frames::new(app);
    frames.settle(app);
    app.start_object_delete();
    frames.step(app, vec![]);
    frames.step(app, vec![]);
    let empty = delete_column_cloth(&frames);
    assert!(
        empty.height() < 160.0 && empty.bottom() < frames.size.y - 40.0,
        "no rows, the column is the header, got {empty:?}"
    );

    app.note.as_mut().unwrap().add_unit_at(1, 0);
    app.object_delete = Some(vec![
        DeletePick {
            target: DeleteTarget::Page(0, 0),
            on: true,
        },
        DeletePick {
            target: DeleteTarget::Page(1, 0),
            on: true,
        },
    ]);
    frames.step(app, vec![]);
    let two = delete_column_cloth(&frames);
    assert!(
        two.height() > empty.height() + 60.0 && two.bottom() < frames.size.y - 40.0,
        "two rows set the height on that frame, empty {} two {two:?}",
        empty.height()
    );
    frames.step(app, vec![]);
    let two_again = delete_column_cloth(&frames);
    assert!(
        (two_again.height() - two.height()).abs() < 1.0,
        "the column must not keep growing after the rows are in"
    );

    let note = app.note.as_mut().unwrap();
    for col in 2..40 {
        note.add_unit_at(col, 0);
    }
    app.object_delete = Some(
        (0..40)
            .map(|col| DeletePick {
                target: DeleteTarget::Page(col, 0),
                on: true,
            })
            .collect(),
    );
    frames.step(app, vec![]);
    let many = delete_column_cloth(&frames);
    assert!(
        many.bottom() >= frames.size.y - 2.0,
        "a long list fills the column and then scrolls, got {many:?}"
    );
    frames.step(app, vec![]);
    let many_again = delete_column_cloth(&frames);
    assert!((many_again.height() - many.height()).abs() < 1.0);
}

#[test]
fn pages_mode_checks_pages_and_leaves_ink() {
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
        app.start_object_delete();
        frames.step(app, vec![]);
        frames.step(app, vec![]);
        assert!(frames.output.as_ref().unwrap().shapes.iter().any(
            |shape| matches!(&shape.shape, Shape::Text(text) if text.galley.text() == "Tap a page")
        ));
        let empty_cloth = delete_column_cloth(&frames);
        assert!(
            empty_cloth.height() < 160.0 && empty_cloth.bottom() < frames.size.y * 0.5,
            "empty delete column should hug the header, got {empty_cloth:?}"
        );
        let canvas = app.canvas_rect;
        let page_two = app
            .camera
            .to_screen(pos2(PAGE_W * 1.5, PAGE_H * 0.5), canvas);
        if touch {
            frames.touch(app, TouchPhase::Start, page_two);
            frames.touch(app, TouchPhase::End, page_two);
        } else {
            frames.drag(app, page_two, page_two);
        }
        let picks = app.object_delete.as_ref().unwrap();
        assert!(picks
            .iter()
            .all(|pick| { matches!(pick.target, DeleteTarget::Page(_, _)) && pick.on }));
        assert_eq!(picks.len(), 1);
        assert!(picks.iter().any(|pick| {
            matches!(pick.target, DeleteTarget::Page(col, row) if col == 1 && row == 0)
        }));
        frames.step(app, vec![]);
        let filled_cloth = delete_column_cloth(&frames);
        assert!(
            filled_cloth.height() > empty_cloth.height() + 20.0
                && filled_cloth.bottom() < frames.size.y - 40.0,
            "column should fit the page row, empty {} filled {}",
            empty_cloth.height(),
            filled_cloth.height()
        );
        frames.settle(app);
        frames.snapshot(if touch {
            "delete-picks-touch"
        } else {
            "delete-picks-mouse"
        });
        let commit = frames.text_center("Delete (1)");
        frames.text_center("Page 2");
        if touch {
            frames.touch(app, TouchPhase::Start, commit);
            frames.touch(app, TouchPhase::End, commit);
        } else {
            frames.drag(app, commit, commit);
        }
        frames.settle(app);
        assert!(app.object_delete.is_none());
        let note = app.note.as_ref().unwrap();
        assert_eq!(note.pages.len(), 1);
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
        app.start_object_delete();
        frames.settle(app);
        let page_two = app
            .camera
            .to_screen(pos2(PAGE_W * 1.5, PAGE_H * 0.5), app.canvas_rect);
        frames.drag(app, page_two, page_two);
        assert_eq!(app.object_delete.as_ref().unwrap().len(), 1);
        frames.key(app, Key::Escape);
        assert!(app.object_delete.is_none());
        assert_eq!(app.note.as_ref().unwrap().pages.len(), 2);
        assert_eq!(app.note.as_ref().unwrap().pages[0].texts[0].id, text_id);
    }
}

#[test]
fn delete_band_checks_a_zone_and_a_page() {
    for touch in [false, true] {
        let mut fixture = Fixture::new();
        let id = fixture.notebook("Delete zone");
        let app = &mut fixture.app;
        app.open_note(id);
        let mut text = TextBox::new(pos2(80.0, 80.0), Color32::BLACK);
        text.text = "Gone".into();
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
        app.start_object_delete();
        frames.settle(app);
        let canvas = app.canvas_rect;
        let text_from = app.camera.to_screen(pos2(70.0, 70.0), canvas);
        let text_to = app.camera.to_screen(pos2(370.0, 170.0), canvas);
        let page_center = app
            .camera
            .to_screen(pos2(PAGE_W * 1.5, PAGE_H * 0.5), canvas);
        let page_from = page_center - vec2(70.0, 50.0);
        let page_to = page_center + vec2(70.0, 50.0);
        let page_tap = app.camera.to_screen(pos2(40.0, 400.0), canvas);
        if touch {
            frames.touch(app, TouchPhase::Start, text_from);
            frames.touch(app, TouchPhase::Move, text_to);
            frames.touch(app, TouchPhase::End, text_to);
            frames.touch(app, TouchPhase::Start, page_from);
            frames.touch(app, TouchPhase::Move, page_to);
            frames.touch(app, TouchPhase::End, page_to);
            frames.touch(app, TouchPhase::Start, page_tap);
            frames.touch(app, TouchPhase::End, page_tap);
        } else {
            frames.drag(app, text_from, text_to);
            frames.drag(app, page_from, page_to);
            frames.drag(app, page_tap, page_tap);
        }
        let picks = app.object_delete.as_ref().unwrap();
        assert!(picks.iter().any(|pick| {
            matches!(pick.target, DeleteTarget::Page(col, row) if col == 1 && row == 0) && pick.on
        }));
        assert!(picks.iter().any(|pick| {
            matches!(pick.target, DeleteTarget::Page(col, row) if col == 0 && row == 0) && pick.on
        }));
        frames.settle(app);
        let commit = frames.text_center("Delete (1)");
        frames.text_center("Page 2");
        if touch {
            frames.touch(app, TouchPhase::Start, commit);
            frames.touch(app, TouchPhase::End, commit);
        } else {
            frames.drag(app, commit, commit);
        }
        frames.settle(app);
        assert!(app.object_delete.is_none());
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
    let expected = app.emoji.catalog().first().expect("emoji font").to_string();
    let mut frames = Frames::new(app);
    frames.settle(app);
    for id in [first, second, first] {
        app.emoji_pick = Some(id);
        frames.settle(app);
        let outside = pos2(4.0, 4.0);
        frames.drag(app, outside, outside);
        assert!(app.emoji_pick.is_none());
        frames.settle(app);

        app.emoji_pick = Some(id);
        frames.settle(app);
        let sheet = frames
            .ctx
            .memory(|m| m.area_rect(Id::new("emoji-pick")))
            .unwrap();
        let cell = sheet.min + vec2(32.0, 32.0);
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
        app.emoji_tex.len() < app.emoji.catalog().len() / 2,
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
        assert_eq!(app.zoom_percent(), 100);
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
