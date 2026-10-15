//! Opt-in real-engine export acceptance for the pack-to-profile workflow.
//! Run with `cargo test --test gameplay_ui_asset_import_export -- --ignored`.

use renderer::{
    render::{RenderBackend, RenderConfig},
    render_ui::{RendererUiConfig, UiComboChange, UiGameplayEvent, UiScoreRule, UiScoringConfig},
};
use std::{
    fs,
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
#[ignore = "real Next Sekai engine export; run explicitly for media/export certification"]
fn imported_asset_pack_art_reaches_a_real_gameplay_mp4() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    let inputs = repository.join("TestingSuite/Next Sekai Engine/levels/larp 64x");
    let engine = inputs.join("Next RUSH.zip");
    let resources = inputs.join("ProSeka Faithful 0.8.4.scp");
    let level = inputs.join("larp 64x.json.gz");
    for input in [&engine, &resources, &level] {
        assert!(
            input.is_file(),
            "required real render fixture is missing: {}",
            input.display()
        );
    }

    let acceptance_dir = repository.join("target/gameplay-ui-import-acceptance");
    fs::create_dir_all(&acceptance_dir).unwrap();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let profile_path = acceptance_dir.join(format!("imported-ui-{unique}.json"));
    let event_path = acceptance_dir.join(format!("events-{unique}.json"));
    let config_path = acceptance_dir.join(format!("render-config-{unique}.json"));
    let output_path = acceptance_dir.join(format!("sono-overlay-reference-{unique}.mp4"));
    let frame_path = acceptance_dir.join(format!("sono-overlay-reference-{unique}-frame.png"));
    let default_config_path = acceptance_dir.join(format!("fresh-default-{unique}.json"));
    let default_output_path = acceptance_dir.join(format!("fresh-default-{unique}.mp4"));
    let default_frame_path = acceptance_dir.join(format!("fresh-default-{unique}-frame.png"));
    let before_path = acceptance_dir.join(format!("sono-overlay-before-event-{unique}.png"));
    let feedback_path = acceptance_dir.join(format!("sono-overlay-judgment-feedback-{unique}.png"));
    let miss_path = acceptance_dir.join(format!("sono-overlay-after-miss-{unique}.png"));

    let manifest = repository.join("assets/presentation-pack.json");
    let (ui, report) = RendererUiConfig::import_asset_pack(&manifest, None).unwrap();
    assert!(report.loaded_assets.contains(&"judge.1".to_owned()));
    assert_eq!(report.configured_elements, 29);
    assert!(!report
        .unconfigured_element_families
        .contains(&"combo.glyph-family.n".to_owned()));
    assert_eq!(
        ui.assets[&ui.fonts["combo"].glyph_assets["8"]],
        Path::new("pack://combo.p8")
    );
    assert_eq!(
        ui.assets[&ui.fonts["combo-burst"].glyph_assets["8"]],
        Path::new("pack://combo.b8")
    );
    assert_eq!(
        ui.fonts["combo.glyph-family.n"].glyph_assets["8"],
        "combo.n8"
    );
    assert_eq!(
        ui.fonts["combo"].center_spacing,
        Some(108.0),
        "the imported AP layout preserves the established spacing"
    );
    assert!(ui
        .elements
        .iter()
        .any(|element| element.id == "score-panel"));
    assert!(ui.elements.iter().any(|element| element.id == "life-panel"));
    assert!(ui
        .elements
        .iter()
        .any(|element| element.id == "combo-value"));
    assert!(ui
        .elements
        .iter()
        .any(|element| element.id == "judgement-perfect"));
    ui.save_profile(&profile_path).unwrap();

    let events = [
        UiGameplayEvent {
            time: 26.0,
            event: "fixture-hit".into(),
            payload: [
                ("rank_code".into(), 2.0),
                ("score_bonus".into(), 46_823.0),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        },
        UiGameplayEvent {
            time: 27.0,
            event: "fixture-miss".into(),
            ..Default::default()
        },
    ];
    fs::write(&event_path, serde_json::to_vec_pretty(&events).unwrap()).unwrap();

    let mut config = RenderConfig::default();
    config.engine = engine;
    config.resources = resources;
    config.level = level;
    config.output = Some(output_path.clone());
    // The source reference frame is at 26s. The imported provider events are
    // deliberately labeled fixtures: the renderer does not claim Watch
    // generated player judgments. They exercise the authoritative event
    // adapter through the actual Watch/render/compositor/export pipeline.
    config.start_time = 25.5;
    config.duration = 1.0 + 31.0 / 60.0;
    config.fps = 60;
    config.width = 1920;
    config.height = 1080;
    config.backend = RenderBackend::Cpu;
    config.layers.bgm = false;
    config.layers.particles = false;
    config.layers.sfx = false;
    config.set_ui_profile(&profile_path).unwrap();
    config.ui.load_external_data().unwrap();
    config.ui.events_file = Some(event_path.clone());
    let mut scoring = UiScoringConfig {
        // Values are external fixture state, not Watch outputs. The hit at
        // 26s matches the source increment reference (655525 / +46823 / combo 47).
        initial_score: 608_702.0,
        initial_combo: 46,
        initial_life: 1_000.0,
        maximum_life: 1_000.0,
        ..UiScoringConfig::default()
    };
    scoring.rules.push(UiScoreRule {
        event: "fixture-hit".into(),
        score: 46_823.0,
        combo: UiComboChange::Increment,
        life: -120.0,
        judgement: "PERFECT".into(),
        ..UiScoreRule::default()
    });
    scoring.rules.push(UiScoreRule {
        event: "fixture-miss".into(),
        combo: UiComboChange::Reset,
        life: -300.0,
        judgement: "MISS".into(),
        ..UiScoreRule::default()
    });
    config.ui.scoring = Some(scoring);
    config.ui.load_external_data().unwrap();
    assert_eq!(
        config.ui.gameplay_state_at(25.5).unwrap().map(|state| (
            state.score,
            state.combo,
            state.life,
            state.judgment
        )),
        Some((608_702.0, 46, 1_000.0, None))
    );
    assert_eq!(
        config.ui.gameplay_state_at(26.0).unwrap().map(|state| (
            state.score,
            state.combo,
            state.life,
            state.judgment
        )),
        Some((655_525.0, 47, 880.0, Some("PERFECT".into())))
    );
    assert_eq!(
        config
            .ui
            .gameplay_state_at(26.0)
            .unwrap()
            .unwrap()
            .variables["score_bonus"],
        46_823.0
    );
    assert_eq!(
        config.ui.gameplay_state_at(27.0).unwrap().map(|state| (
            state.score,
            state.combo,
            state.life,
            state.judgment
        )),
        Some((655_525.0, 0, 580.0, Some("MISS".into())))
    );
    config.save_config(&config_path).unwrap();

    let executable = std::env::var_os("SONO_ACCEPTANCE_RENDERER")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_renderer").into());
    let rendered = Command::new(&executable)
        .arg("render-config")
        .arg(&config_path)
        .current_dir(repository)
        .output()
        .expect("starting the production render-config command");
    assert!(
        rendered.status.success(),
        "render-config failed: {}\n{}",
        String::from_utf8_lossy(&rendered.stdout),
        String::from_utf8_lossy(&rendered.stderr)
    );
    assert!(output_path.is_file() && fs::metadata(&output_path).unwrap().len() > 10_000);

    let addons = repository.parent().unwrap().join("addons");
    let ffprobe = addons.join("ffprobe.exe");
    let ffmpeg = addons.join("ffmpeg.exe");
    assert!(
        ffprobe.is_file(),
        "FFprobe fixture tool is missing: {}",
        ffprobe.display()
    );
    assert!(
        ffmpeg.is_file(),
        "FFmpeg fixture tool is missing: {}",
        ffmpeg.display()
    );
    let probe = Command::new(&ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,codec_name,width,height,r_frame_rate:format=duration",
            "-of",
            "json",
        ])
        .arg(&output_path)
        .output()
        .unwrap();
    assert!(
        probe.status.success(),
        "FFprobe failed: {}",
        String::from_utf8_lossy(&probe.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
    let video = metadata["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stream| stream["codec_type"] == "video")
        .unwrap();
    assert_eq!(video["codec_name"], "h264");
    assert_eq!(video["width"], 1920);
    assert_eq!(video["height"], 1080);
    assert_eq!(video["r_frame_rate"], "60/1");
    let duration = metadata["format"]["duration"]
        .as_str()
        .unwrap()
        .parse::<f64>()
        .unwrap();
    assert!((duration - 1.5).abs() < 0.03, "unexpected duration {duration}");

    let extracted = Command::new(&ffmpeg)
        .args(["-y", "-v", "error", "-ss", "0.5", "-i"])
        .arg(&output_path)
        .args(["-frames:v", "1", "-update", "1"])
        .arg(&frame_path)
        .output()
        .unwrap();
    assert!(
        extracted.status.success(),
        "frame extraction failed: {}",
        String::from_utf8_lossy(&extracted.stderr)
    );
    let frame = image::open(&frame_path).unwrap().to_rgb8();
    assert_eq!(frame.dimensions(), (1920, 1080));
    for (time, path) in [
        ("0", &before_path),
        ("0.75", &feedback_path),
        ("1.5", &miss_path),
    ] {
        let extracted = Command::new(&ffmpeg)
            .args(["-y", "-v", "error", "-ss", time, "-i"])
            .arg(&output_path)
            .args(["-frames:v", "1", "-update", "1"])
            .arg(path)
            .output()
            .unwrap();
        assert!(
            extracted.status.success(),
            "event-state frame extraction failed: {}",
            String::from_utf8_lossy(&extracted.stderr)
        );
        assert_eq!(image::image_dimensions(path).unwrap(), (1920, 1080));
    }
    let before_frame = image::open(&before_path).unwrap().to_rgb8();
    let feedback_frame = image::open(&feedback_path).unwrap().to_rgb8();
    let miss_frame = image::open(&miss_path).unwrap().to_rgb8();
    let changed_pixels = |left: &image::RgbImage,
                          right: &image::RgbImage,
                          x_start: u32,
                          x_end: u32,
                          y_start: u32,
                          y_end: u32| {
        let mut changed = 0;
        for y in y_start..y_end {
            for x in x_start..x_end {
                if left.get_pixel(x, y) != right.get_pixel(x, y) {
                    changed += 1;
                }
            }
        }
        changed
    };
    assert!(
        changed_pixels(&before_frame, &frame, 0, 720, 0, 180) > 0,
        "score-panel pixels did not change after the fixture hit"
    );
    assert!(
        changed_pixels(&before_frame, &frame, 1410, 1920, 300, 690) > 0,
        "combo pixels did not change after the fixture hit"
    );
    assert!(
        changed_pixels(&frame, &feedback_frame, 810, 1110, 630, 720) > 200,
        "judgment artwork did not appear after its configured fade-in interval"
    );
    assert!(
        changed_pixels(&frame, &miss_frame, 1410, 1920, 0, 180) > 0,
        "life gauge pixels did not change after the fixture miss"
    );
    assert!(
        changed_pixels(&frame, &miss_frame, 1410, 1920, 300, 690) > 0,
        "combo pixels did not change after the fixture miss/reset"
    );
    let digit = image::open(repository.join("assets/score/digit/1.png"))
        .unwrap()
        .to_rgba8();
    let opaque = digit
        .pixels()
        .find(|pixel| pixel[3] > 240)
        .expect("score glyph fixture has opaque pixels");
    let target = [opaque[0], opaque[1], opaque[2]];
    let has_glyph_color = frame.pixels().any(|pixel| {
        pixel[0].abs_diff(target[0]) < 36
            && pixel[1].abs_diff(target[1]) < 36
            && pixel[2].abs_diff(target[2]) < 36
    });
    assert!(
        has_glyph_color,
        "exported frame did not retain a color from the imported score glyph artwork"
    );

    // Render the same chart, range, events, and timestamps through a truly
    // fresh RenderConfig::default() with no UI profile selected. This catches
    // cases where compositor tests pass only because they explicitly load the
    // imported reference profile.
    let mut default_config = config.clone();
    default_config.output = Some(default_output_path.clone());
    default_config.ui = RenderConfig::default().ui;
    default_config.ui.events_file = Some(event_path.clone());
    default_config.ui.scoring = config.ui.scoring.clone();
    default_config.ui.load_external_data().unwrap();
    assert_eq!(default_config.ui.profile_name, "Asset-backed Gameplay HUD");
    assert_eq!(default_config.ui.fonts["combo"].center_spacing, Some(108.0));
    assert_eq!(
        default_config.ui.fonts["combo-burst"].center_spacing,
        Some(108.0)
    );
    assert_eq!(default_config.ui.elements, config.ui.elements);
    default_config.save_config(&default_config_path).unwrap();
    let default_rendered = Command::new(&executable)
        .arg("render-config")
        .arg(&default_config_path)
        .current_dir(repository)
        .output()
        .expect("starting the production render-config command with no UI profile");
    assert!(
        default_rendered.status.success(),
        "fresh-default render-config failed:\n{}\n{}",
        String::from_utf8_lossy(&default_rendered.stdout),
        String::from_utf8_lossy(&default_rendered.stderr)
    );
    assert!(
        default_output_path.is_file() && fs::metadata(&default_output_path).unwrap().len() > 10_000,
        "fresh-default profile did not produce a valid MP4"
    );
    let default_probe = Command::new(&ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,codec_name,width,height,r_frame_rate:format=duration",
            "-of",
            "json",
        ])
        .arg(&default_output_path)
        .output()
        .unwrap();
    assert!(
        default_probe.status.success(),
        "FFprobe rejected fresh-default output: {}",
        String::from_utf8_lossy(&default_probe.stderr)
    );
    let default_metadata: serde_json::Value = serde_json::from_slice(&default_probe.stdout).unwrap();
    let default_video = default_metadata["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stream| stream["codec_type"] == "video")
        .unwrap();
    assert_eq!(default_video["codec_name"], "h264");
    assert_eq!(default_video["width"], 1920);
    assert_eq!(default_video["height"], 1080);
    assert_eq!(default_video["r_frame_rate"], "60/1");
    let default_duration = default_metadata["format"]["duration"]
        .as_str()
        .unwrap()
        .parse::<f64>()
        .unwrap();
    assert!(
        (default_duration - 1.5).abs() < 0.03,
        "unexpected default duration {default_duration}"
    );
    let extracted = Command::new(&ffmpeg)
        .args(["-y", "-v", "error", "-ss", "0.5", "-i"])
        .arg(&default_output_path)
        .args(["-frames:v", "1", "-update", "1"])
        .arg(&default_frame_path)
        .output()
        .unwrap();
    assert!(
        extracted.status.success(),
        "fresh-default frame extraction failed: {}",
        String::from_utf8_lossy(&extracted.stderr)
    );
    let default_frame = image::open(&default_frame_path).unwrap().to_rgb8();
    assert_eq!(default_frame.dimensions(), (1920, 1080));
    assert_eq!(
        frame, default_frame,
        "fresh default and working acceptance profile must produce the same frame at chart time 26.0"
    );
    for (time, imported_path) in [
        ("0", &before_path),
        ("0.75", &feedback_path),
        ("1.5", &miss_path),
    ] {
        let path = acceptance_dir.join(format!("fresh-default-{unique}-{time}.png"));
        let extracted = Command::new(&ffmpeg)
            .args(["-y", "-v", "error", "-ss", time, "-i"])
            .arg(&default_output_path)
            .args(["-frames:v", "1", "-update", "1"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            extracted.status.success(),
            "fresh-default frame extraction at {time}s failed: {}",
            String::from_utf8_lossy(&extracted.stderr)
        );
        let imported = image::open(imported_path).unwrap().to_rgb8();
        let fresh = image::open(&path).unwrap().to_rgb8();
        assert_eq!(
            imported, fresh,
            "fresh default and imported acceptance frames differ at output {time}s"
        );
    }
    let bonus_frames = [
        ("age-00", "0.5"),
        ("age-01", "0.516666667"),
        ("age-12", "0.7"),
        ("age-30", "1.0"),
        ("age-31", "1.016666667"),
    ];
    for (age, time) in bonus_frames {
        let imported_frame_path = acceptance_dir.join(format!("score-bonus-{age}-{unique}.png"));
        let default_bonus_frame_path =
            acceptance_dir.join(format!("score-bonus-default-{age}-{unique}.png"));
        for (video, path) in [
            (&output_path, &imported_frame_path),
            (&default_output_path, &default_bonus_frame_path),
        ] {
            let extracted = Command::new(&ffmpeg)
                .args(["-y", "-v", "error", "-ss", time, "-i"])
                .arg(video)
                .args(["-frames:v", "1", "-update", "1"])
                .arg(path)
                .output()
                .unwrap();
            assert!(
                extracted.status.success(),
                "score-bonus frame {age} extraction failed: {}",
                String::from_utf8_lossy(&extracted.stderr)
            );
        }
        let imported = image::open(&imported_frame_path).unwrap().to_rgb8();
        let fresh = image::open(&default_bonus_frame_path).unwrap().to_rgb8();
        assert_eq!(
            imported, fresh,
            "default and acceptance MP4 differ at {age}"
        );
        let crop = image::imageops::crop_imm(&imported, 350, 80, 280, 90).to_image();
        crop.save(acceptance_dir.join(format!("score-bonus-crop-{age}-{unique}.png")))
            .unwrap();
    }
    println!("DEFAULT MP4: {}", default_output_path.display());
    println!("DEFAULT FRAME: {}", default_frame_path.display());

    println!("MP4: {}", output_path.display());
    println!("FRAME: {}", frame_path.display());
    println!("PROFILE: {}", profile_path.display());
}

#[test]
#[ignore = "real Next Sekai engine export; run explicitly for default static HUD certification"]
fn bundled_default_hud_exports_static_art_without_gameplay_events() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    let inputs = repository.join("TestingSuite/Next Sekai Engine/levels/larp 64x");
    let engine = inputs.join("Next RUSH.zip");
    let resources = inputs.join("ProSeka Faithful 0.8.4.scp");
    let level = inputs.join("larp 64x.json.gz");
    for input in [&engine, &resources, &level] {
        assert!(input.is_file(), "missing real fixture: {}", input.display());
    }

    let output_dir = repository.join("target/gameplay-ui-import-acceptance");
    fs::create_dir_all(&output_dir).unwrap();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let config_path = output_dir.join(format!("fresh-default-{unique}.json"));
    let output_path = output_dir.join(format!("fresh-default-no-events-{unique}.mp4"));
    let frame_path = output_dir.join(format!("fresh-default-no-events-{unique}.png"));

    let mut config = RenderConfig::default();
    config.engine = engine;
    config.resources = resources;
    config.level = level;
    config.output = Some(output_path.clone());
    config.duration = 1.0;
    config.fps = 2;
    config.width = 640;
    config.height = 360;
    config.backend = RenderBackend::Cpu;
    config.layers.bgm = false;
    config.layers.particles = false;
    config.layers.sfx = false;
    config.ui.enabled = true;
    assert!(config.ui.events_file.is_none());
    assert!(config.ui.scoring.is_none());
    config.ui.load_external_data().unwrap();
    config.save_config(&config_path).unwrap();

    let rendered = Command::new(env!("CARGO_BIN_EXE_renderer"))
        .arg("render-config")
        .arg(&config_path)
        .current_dir(repository)
        .output()
        .expect("starting the production render-config command");
    assert!(
        rendered.status.success(),
        "default render-config failed:\n{}\n{}",
        String::from_utf8_lossy(&rendered.stdout),
        String::from_utf8_lossy(&rendered.stderr)
    );
    assert!(output_path.is_file() && fs::metadata(&output_path).unwrap().len() > 10_000);

    let addons = repository.parent().unwrap().join("addons");
    let ffprobe = addons.join("ffprobe.exe");
    let ffmpeg = addons.join("ffmpeg.exe");
    let probe = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,codec_name,width,height",
            "-of",
            "json",
        ])
        .arg(&output_path)
        .output()
        .unwrap();
    assert!(
        probe.status.success(),
        "FFprobe rejected default HUD output: {}",
        String::from_utf8_lossy(&probe.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
    assert!(metadata["streams"]
        .as_array()
        .unwrap()
        .iter()
        .any(|stream| {
            stream["codec_type"] == "video"
                && stream["codec_name"] == "h264"
                && stream["width"] == 640
                && stream["height"] == 360
        }));

    let extracted = Command::new(ffmpeg)
        .args(["-y", "-v", "error", "-ss", "0.25", "-i"])
        .arg(&output_path)
        .args(["-frames:v", "1", "-update", "1"])
        .arg(&frame_path)
        .output()
        .unwrap();
    assert!(
        extracted.status.success(),
        "frame extraction failed: {}",
        String::from_utf8_lossy(&extracted.stderr)
    );
    let frame = image::open(&frame_path).unwrap().to_rgb8();
    assert_eq!(frame.dimensions(), (640, 360));
    let score_panel = image::open(repository.join("assets/score/bg.png"))
        .unwrap()
        .to_rgba8();
    let reference_colors = score_panel
        .pixels()
        .filter(|pixel| {
            pixel[3] > 240 && pixel[0] > 40 && pixel[0] < 150 && pixel[1] > 40 && pixel[1] < 150
        })
        .map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .take(2048)
        .collect::<Vec<_>>();
    let static_art_visible = frame.enumerate_pixels().any(|(x, y, pixel)| {
        x < 240
            && y < 70
            && reference_colors.iter().any(|color| {
                pixel[0].abs_diff(color[0]) < 30
                    && pixel[1].abs_diff(color[1]) < 30
                    && pixel[2].abs_diff(color[2]) < 30
            })
    });
    assert!(
        static_art_visible,
        "the reference score-panel artwork was not visible in the event-free export"
    );

    println!("MP4: {}", output_path.display());
    println!("FRAME: {}", frame_path.display());
}
