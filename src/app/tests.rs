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
    app: CahierApp,
    root: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("cahier-ui-{}", Uuid::new_v4()));
        let lib = Library::fresh(root.clone());
        Self {
            app: CahierApp::with_library(Look::load(), lib),
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
    let started = std::time::Instant::now();
    let mut frames = Frames::new(&fixture.app);
    frames.settle(&mut fixture.app);
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_millis(1500),
        "opening 40 PDF pages blocked the UI for {elapsed:?}"
    );
    assert!(matches!(fixture.app.scene, Scene::Desk));
}

struct Frames {
    ctx: Context,
    time: f64,
    textures: HashMap<TextureId, ColorImage>,
    output: Option<FullOutput>,
    size: Vec2,
}

impl Frames {
    fn new(app: &CahierApp) -> Self {
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

    fn step(&mut self, app: &mut CahierApp, events: Vec<Event>) {
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

    fn settle(&mut self, app: &mut CahierApp) {
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

    fn drag(&mut self, app: &mut CahierApp, from: Pos2, to: Pos2) {
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

    fn key(&mut self, app: &mut CahierApp, key: Key) {
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

    fn touch(&mut self, app: &mut CahierApp, phase: TouchPhase, pos: Pos2) {
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
        let Some(directory) = std::env::var_os("CAHIER_TEST_ARTIFACTS") else {
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
            for accessory in 0..3 {
                app.palette_open = accessory > 0;
                app.tin_open = accessory == 2;
                frames.settle(app);
                let screen = Rect::from_min_size(Pos2::ZERO, size);
                let controls: Vec<_> = ["top-rect", "dock-rect", "strip-rect", "tin-rect"]
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
                if edge == DockEdge::Bottom && size.x == 320.0 && accessory == 2 {
                    frames.snapshot("responsive-note-small");
                }
            }
        }
    }
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
    app.start_page_delete();
    frames.settle(app);
    let cancel = frames.text_center("Cancel");
    let delete = frames.text_center("Delete (0)");
    assert!(cancel.x < delete.x && cancel.y < 78.0 && delete.y < 78.0);
    frames.snapshot("responsive-page-selection-small");
    frames.touch(app, TouchPhase::Start, cancel);
    frames.touch(app, TouchPhase::End, cancel);
    assert!(app.page_delete.is_none());
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
    let more = pos2(frames.size.x - 30.0, 26.0);
    frames.drag(app, more, more);
    frames.settle(app);
    assert!(Popup::is_any_open(&frames.ctx));
    let last = frames.text_center("Move to trash");
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
    app.palette_open = true;
    app.tin_open = true;
    let mut frames = Frames::new(app);
    frames.settle(app);
    for (input, col) in [(0, 2), (1, 6), (2, 9)] {
        let id = Id::new(("tin-pan", 0, col));
        let hit = frames.ctx.read_response(id).unwrap().rect;
        match input {
            0 => frames.drag(app, hit.center(), hit.center()),
            1 => {
                frames.touch(app, TouchPhase::Start, hit.center());
                frames.touch(app, TouchPhase::End, hit.center());
            }
            _ => {
                frames.ctx.memory_mut(|m| m.request_focus(id));
                frames.key(app, Key::Enter);
            }
        }
        assert_eq!(app.ink, hsv_to_rgb(col as f32 / 12.0, 0.88, 0.92));
        assert!(app.tin_open);
    }
    frames.settle(app);
    let tin = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("tin-rect")))
        .unwrap();
    let strip = frames
        .ctx
        .data(|d| d.get_temp::<Rect>(Id::new("strip-rect")))
        .unwrap();
    assert!(
        !tin.intersects(strip),
        "advanced colours must not cover the ink strip"
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
    assert!(!app.tin_open && app.palette_open);
    assert!(matches!(app.scene, Scene::Desk));
}

#[test]
fn ink_fields_use_continuous_meshes() {
    let ctx = Context::default();
    let output = ctx.run(RawInput::default(), |ctx| {
        let painter = ctx.layer_painter(LayerId::background());
        paint_sv_field(
            &painter,
            Rect::from_min_size(Pos2::ZERO, vec2(204.0, 132.0)),
            0.5,
        );
        paint_hue_bar(
            &painter,
            Rect::from_min_size(pos2(0.0, 144.0), vec2(204.0, 16.0)),
        );
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
        app.palette_open = true;
        frames.settle(app);
        let dock = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
            .unwrap();
        assert!(Rect::from_min_size(Pos2::ZERO, size).contains_rect(dock));
        let strip = frames
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("strip-rect")))
            .unwrap();
        assert!(Rect::from_min_size(Pos2::ZERO, size).contains_rect(strip));
        if touch {
            assert!(
                strip.width() <= 60.0,
                "vertical palette must remain a narrow strip"
            );
            assert!(
                strip.max.x <= dock.min.x,
                "palette must not cover the tools"
            );
        }
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
fn page_trash_selection_is_explicit_and_batch_undo_restores_pages() {
    for (join, touch) in [(SheetJoin::Linked, false), (SheetJoin::Separate, true)] {
        let mut fixture = Fixture::new();
        let id = fixture.notebook("Pages");
        let app = &mut fixture.app;
        app.open_note(id);
        let note = app.note.as_mut().unwrap();
        note.add_unit_at(1, 0);
        note.add_unit_at(2, 0);
        note.sheet_join = join;
        let before = serde_json::to_value(&note.pages).unwrap();
        let mut frames = Frames::new(app);
        frames.settle(app);
        app.fit_to_screen();
        frames.settle(app);
        frames.snapshot("note-toolbar-trash");
        app.start_page_delete();
        frames.settle(app);
        let hit = app
            .unit_screen_rect(app.canvas_rect, 0, 0)
            .intersect(app.canvas_rect);
        if touch {
            frames.touch(app, TouchPhase::Start, hit.center());
            assert_eq!(app.note.as_ref().unwrap().pages.len(), 3);
            frames.touch(app, TouchPhase::End, hit.center());
        } else {
            frames.drag(app, hit.center(), hit.center());
        }
        assert_eq!(app.note.as_ref().unwrap().pages.len(), 3);
        assert_eq!(app.page_delete.as_ref().unwrap(), &vec![(0, 0)]);
        frames.settle(app);
        frames.snapshot("note-page-selection");
        frames.key(app, Key::Escape);
        assert!(app.page_delete.is_none());
        assert_eq!(app.note.as_ref().unwrap().pages.len(), 3);
        app.start_page_delete();
        app.page_delete = Some(vec![(0, 0), (1, 0), (2, 0)]);
        app.delete_selected_pages();
        assert_eq!(app.note.as_ref().unwrap().pages.len(), 3);
        app.page_delete = Some(vec![(0, 0), (1, 0)]);
        app.delete_selected_pages();
        assert_eq!(app.note.as_ref().unwrap().pages.len(), 1);
        assert!(app.note.as_ref().unwrap().unit_occupied(2, 0));
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
        frames.snapshot(name);
    }
}
