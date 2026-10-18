use super::*;

fn evaluation() -> Analysis {
    Analysis {
        exercise_found: true,
        readable: true,
        complete_step: true,
        verdict: Verdict::OnTrack,
        statement: "Solve 2x + 3 = 11".into(),
        recognized_step: "2x = 8".into(),
        explanation: "Subtracting 3 from both sides is valid.".into(),
        region: Some([0.05, 0.1, 0.85, 0.4]),
    }
}

fn completion(ticket: Ticket) -> Completed {
    Completed {
        ticket,
        result: Ok(evaluation()),
        timings: Timings::default(),
    }
}

#[test]
fn analysis_never_grades_ambiguous_or_incomplete_writing() {
    for i in 0..5 {
        let mut result = evaluation();
        result.verdict = Verdict::LikelyError;
        match i {
            0 => result.readable = false,
            1 => result.complete_step = false,
            2 => result.exercise_found = false,
            3 => result.statement.clear(),
            _ => result.recognized_step.clear(),
        }
        assert_eq!(result.validate().unwrap().verdict, Verdict::Insufficient);
    }
    let mut result = evaluation();
    result.region = Some([0.9, 0.0, 0.5, 0.1]);
    assert!(result.validate().is_err());
}

#[test]
fn scheduler_debounces_coalesces_and_never_repeats_unchanged_content() {
    let mut s = Session::default();
    s.reset(Some(Uuid::new_v4()));
    let t = Instant::now();
    s.invalidate(t);
    assert!(!s.ready(t + Duration::from_millis(349)));
    assert!(s.ready(t + QUIET));
    s.sent(t + QUIET);
    let ticket = s.busy.unwrap();
    for i in 1..100 {
        s.changed(true, t + Duration::from_millis(350 + i));
    }
    assert!(
        !s.ready(t + Duration::from_secs(10)),
        "one analysis at a time"
    );
    s.receive(completion(ticket));
    assert!(
        s.result.is_some(),
        "appending must not starve the last completed step"
    );
    assert!(!s.ready(t + Duration::from_millis(1549)));
    assert!(s.ready(t + Duration::from_millis(1550)));
    s.sent(t + Duration::from_millis(1550));
    s.receive(completion(s.busy.unwrap()));
    assert!(!s.ready(t + Duration::from_secs(60)));
}

#[test]
fn continuous_writing_gets_a_first_snapshot_without_waiting_forever() {
    let mut s = Session::default();
    s.reset(Some(Uuid::new_v4()));
    let t = Instant::now();
    s.invalidate(t);
    for i in 1..12 {
        s.changed(true, t + Duration::from_millis(i * 100));
    }
    assert!(s.ready(t + INTERVAL));
}

#[test]
fn erasing_switching_notebooks_and_disabling_discard_old_results() {
    for operation in 0..3 {
        let mut s = Session::default();
        s.reset(Some(Uuid::new_v4()));
        let t = Instant::now();
        s.sent(t);
        let ticket = s.busy.unwrap();
        match operation {
            0 => s.changed(false, t),
            1 => s.reset(Some(Uuid::new_v4())),
            _ => s.reset(None),
        }
        s.receive(completion(ticket));
        assert!(s.result.is_none());
        assert!(s.busy.is_none());
    }
}

#[test]
fn errors_suspend_requests_until_explicit_retry() {
    let mut s = Session::default();
    s.reset(Some(Uuid::new_v4()));
    let t = Instant::now();
    s.sent(t);
    s.receive(Completed {
        ticket: s.busy.unwrap(),
        result: Err("Quota reached".into()),
        timings: Timings::default(),
    });
    s.changed(true, t);
    assert!(!s.ready(t + Duration::from_secs(60)));
    s.invalidate(t);
    assert!(s.ready(t + Duration::from_secs(2)));
}

#[test]
fn old_library_files_default_to_disabled_and_preferences_roundtrip() {
    // Use the actual existing representation for legacy fields.
    let mut value = serde_json::to_value(crate::library::Index::default()).unwrap();
    value.as_object_mut().unwrap().remove("ai");
    let mut index: crate::library::Index = serde_json::from_value(value).unwrap();
    assert!(index.ai.enabled_notebooks.is_empty());
    let id = Uuid::new_v4();
    index.ai.enabled_notebooks.insert(id);
    index.ai.provider = Provider::Gemini;
    let serialized = serde_json::to_string(&index).unwrap();
    assert!(!serialized.contains("api_key"));
    let read: crate::library::Index = serde_json::from_str(&serialized).unwrap();
    assert_eq!(read.ai, index.ai);
}

#[test]
fn snapshot_only_copies_the_requested_page_without_meshes() {
    let mut note = Note::blank("Test", 0);
    note.pages.push(crate::document::Page::at(1, 0));
    let mut stroke =
        crate::ink::InkStroke::new(crate::ink::Nib::Fineliner, egui::Color32::BLACK, 2.0);
    stroke.push(crate::ink::InkPoint::new(egui::pos2(80.0, 90.0), 0.7));
    stroke.tessellate();
    note.pages[1].strokes.push(stroke);
    let copy = snapshot(&note, 1).unwrap();
    assert_eq!(copy.pages.len(), 1);
    assert_eq!(copy.pages[0].strokes.len(), 1);
    assert!(copy.pages[0].strokes[0].mesh.is_none());
    assert!(snapshot(&note, 2).is_none());
}

/// A reproducible synthetic page, never a user's notebook.
fn sample_note() -> Note {
    use crate::ink::{InkPoint, InkStroke, Nib};
    let mut note = Note::blank("Coach smoke test", 0);
    note.paper = crate::document::PaperKind::Blank;
    let mut text = crate::document::TextBox::new(egui::pos2(80.0, 60.0), egui::Color32::BLACK);
    text.text = "Solve 2x + 3 = 11. Find x.".into();
    text.size = [850.0, 70.0];
    text.size_pt = 32.0;
    note.pages[0].texts.push(text);
    // Pen paths spelling "2x = 8"; genuinely vector ink rather than printed answers.
    let paths: &[&[[f32; 2]]] = &[
        &[
            [0., 7.],
            [4., 2.],
            [10., 0.],
            [16., 3.],
            [18., 8.],
            [15., 14.],
            [2., 28.],
            [20., 28.],
        ],
        &[[0., 5.], [18., 27.]],
        &[[18., 4.], [0., 28.]],
        &[[0., 9.], [23., 9.]],
        &[[0., 20.], [23., 20.]],
        &[
            [10., 14.],
            [2., 8.],
            [3., 2.],
            [11., 0.],
            [18., 4.],
            [17., 10.],
            [10., 14.],
            [2., 20.],
            [2., 26.],
            [10., 30.],
            [19., 26.],
            [18., 20.],
            [10., 14.],
        ],
    ];
    let offsets = [0., 35., 35., 80., 80., 125.];
    for (path, x) in paths.iter().zip(offsets) {
        let mut s = InkStroke::new(Nib::Fineliner, egui::Color32::BLACK, 3.0);
        for [px, py] in *path {
            s.push(InkPoint::new(
                egui::pos2(90.0 + (x + px) * 2.0, 180.0 + py * 2.0),
                0.8,
            ));
        }
        note.pages[0].strokes.push(s);
    }
    note
}

#[test]
fn renders_current_page_crop_and_reference_with_explicit_image_roles() {
    let note = sample_note();
    let request = Request {
        ticket: Ticket {
            epoch: 0,
            revision: 1,
        },
        note: note.clone(),
        reference: Some(note),
        note_dir: std::env::temp_dir(),
        look: Look::load(),
        context: Some(evaluation()),
        settings: AiSettings {
            provider: Provider::Gemini,
            ..Default::default()
        },
        key: String::new(),
        created: Instant::now(),
    };
    let prepared = prepare_images(&request).unwrap();
    let (images, detail) = (&prepared.images, &prepared.detail);
    assert_eq!(images.len(), 3);
    assert!(detail.contains("Image 2 is a close-up"));
    assert!(detail.contains("Image 3 is the user-selected"));
    let overview = image::load_from_memory(&images[0]).unwrap();
    assert_eq!(overview.width().max(overview.height()), 768);
}

#[test]
fn local_crop_keeps_all_initial_content_and_maps_regions_back_to_the_page() {
    let request = Request {
        ticket: Ticket {
            epoch: 0,
            revision: 1,
        },
        note: sample_note(),
        reference: None,
        note_dir: std::env::temp_dir(),
        look: Look::load(),
        context: None,
        settings: AiSettings::default(),
        key: String::new(),
        created: Instant::now(),
    };
    let prepared = prepare_images(&request).unwrap();
    assert_eq!(prepared.images.len(), 1);
    assert!(
        prepared.viewport[3] < 0.5,
        "empty margins should be removed"
    );
    let mut result = evaluation();
    result.region = Some([0.0, 0.0, 1.0, 1.0]);
    assert_eq!(
        prepared.restore_region(result).region,
        Some(prepared.viewport)
    );
}

#[test]
fn corrupt_exercise_background_is_not_silently_replaced_by_blank_paper() {
    let root = std::env::temp_dir().join(format!("inkarchy-coach-corrupt-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("exercise.pdf"), b"not a PDF").unwrap();
    let mut note = sample_note();
    note.pages[0].background = Some(crate::document::PageBackground {
        file: "exercise.pdf".into(),
        pdf_page: Some(0),
        source_size: None,
    });
    let request = Request {
        ticket: Ticket {
            epoch: 0,
            revision: 1,
        },
        note,
        reference: None,
        note_dir: root.clone(),
        look: Look::load(),
        context: None,
        settings: AiSettings::default(),
        key: String::new(),
        created: Instant::now(),
    };
    assert!(prepare_images(&request).is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
#[ignore = "Requires an installed local vision model and an Ollama server"]
fn live_local_coach_smoke() {
    let mut settings = AiSettings::default();
    settings.local_model = std::env::var("INKARCHY_AI_TEST_MODEL")
        .unwrap_or_else(|_| AiSettings::default().local_model);
    check_connection(&settings, "").expect("local vision model available");
    let client = provider::Transport::new().unwrap();
    let epoch = AtomicU64::new(0);
    eprintln!("Loading the local model (up to 180 seconds)…");
    let load = Instant::now();
    client
        .warm(&settings, &epoch, 0)
        .expect("local model loading");
    eprintln!("load_ms={}", load.elapsed().as_millis());
    let mut request = Request {
        ticket: Ticket {
            epoch: 0,
            revision: 1,
        },
        note: sample_note(),
        reference: None,
        note_dir: std::env::temp_dir(),
        look: Look::load(),
        context: None,
        settings: settings.clone(),
        key: String::new(),
        created: Instant::now(),
    };
    for run in 0..2 {
        let start = Instant::now();
        let prepared = prepare_images(&request).unwrap();
        let prep = start.elapsed();
        let infer = Instant::now();
        let result = client
            .analyze(
                &settings,
                "",
                &prepared.images,
                request.context.as_ref(),
                &prepared.detail,
                &epoch,
                0,
            )
            .map(|r| prepared.restore_region(r))
            .expect("structured local evaluation");
        eprintln!(
            "run={run} image_ms={} model_ms={} verdict={:?} read={}",
            prep.as_millis(),
            infer.elapsed().as_millis(),
            result.verdict,
            result.recognized_step
        );
        assert!(result.exercise_found);
        assert!(result.readable);
        assert_eq!(
            result.verdict,
            Verdict::OnTrack,
            "simple valid algebra step"
        );
        request.context = Some(result);
    }
}
