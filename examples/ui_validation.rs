//! Development-only real-renderer screenshots for the gameplay UI profiles.
//!
//! Run from the repository root with `cargo run --offline --example ui_validation`.
//! Each chart/Watch scene is rendered by the real CPU renderer; the production
//! UI compositor is then applied to copies of those real frame pixels. This is
//! not included in the installed renderer CLI or desktop application.

use anyhow::{bail, ensure, Context, Result};
use renderer::{
    formats,
    offline::{FrameSession, RenderedFrame},
    render::RenderBackend,
    render_ui::{
        render_gameplay_ui, GameplayUiMode, RendererUiConfig, UiBitmapFont, UiElement,
        UiElementKind, UiFrameState, UiGameplayState, UiLayout, UiTextAlign,
    },
};
use std::{collections::BTreeMap, fs, path::Path, time::Instant};

fn main() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture = root.join("TestingSuite/Next Sekai Engine/levels/larp 64x");
    let engine_path = fixture.join("Next RUSH.zip");
    let resources = fixture.join("ProSeka Faithful 0.8.4.scp");
    let level_path = fixture.join("larp 64x.json.gz");
    for path in [&engine_path, &level_path, &resources] {
        if !path.exists() {
            bail!("visual-validation fixture is missing: {}", path.display());
        }
    }

    let package = formats::load_engine(&engine_path)?;
    let level = formats::load_level(&level_path)?;
    let defaults: BTreeMap<_, _> = package.metadata.resource_defaults().into_iter().collect();
    let skin = defaults
        .get("skins")
        .map(String::as_str)
        .or(package.metadata.skin_name.as_deref())
        .context("visual-validation engine has no default skin")?;
    let background = defaults
        .get("backgrounds")
        .map(String::as_str)
        .or(package.metadata.background_name.as_deref());
    let effects = defaults
        .get("effects")
        .map(String::as_str)
        .or(package.metadata.effect_name.as_deref());
    let particles = defaults
        .get("particles")
        .map(String::as_str)
        .or(package.metadata.particle_name.as_deref());

    let output = root.join("artifacts/ui-validation");
    fs::create_dir_all(&output)?;
    let time = 298.0 / 12.0;
    let landscape = render_scene(
        &package.watch,
        &package.rom,
        &package.configuration,
        &level,
        &resources,
        skin,
        background,
        effects,
        particles,
        time,
        1280,
        720,
    )?;
    let landscape_state = frame_state(&landscape, time, 20.0);
    let landscape_1080p = render_scene(
        &package.watch,
        &package.rom,
        &package.configuration,
        &level,
        &resources,
        skin,
        background,
        effects,
        particles,
        time,
        1920,
        1080,
    )?;
    let landscape_state_1080p = frame_state(&landscape_1080p, time, 20.0);
    let landscape_4_3 = render_scene(
        &package.watch,
        &package.rom,
        &package.configuration,
        &level,
        &resources,
        skin,
        background,
        effects,
        particles,
        time,
        1024,
        768,
    )?;
    let landscape_state_4_3 = frame_state(&landscape_4_3, time, 20.0);
    let event_window_frames = render_scene_frames(
        &package.watch,
        &package.rom,
        &package.configuration,
        &level,
        &resources,
        skin,
        background,
        effects,
        particles,
        &[296, 297, 298, 305],
        1280,
        720,
    )?;

    let mut default = RendererUiConfig::load_profile(root.join("ui-profiles/default.json"))?;
    default.prepare_external_data()?;
    let mut outputs = Vec::new();
    let mut normal_state = landscape_state.clone();
    normal_state.gameplay = default.scoring.as_ref().map(UiGameplayState::from_config);
    ensure!(
        normal_state
            .gameplay
            .as_ref()
            .is_some_and(|state| { state.score == 0.0 && state.combo == 0 && state.life == 850.0 }),
        "default visual fixture initial state changed"
    );
    outputs.push(render_ui(
        &landscape,
        &default,
        &normal_state,
        1280,
        720,
        "default_assets_normal_16_9_1280x720_t24.833.png",
        &output,
    )?);

    // Evaluate the authored LIFE curve on real Watch frames at the same times.
    // This exercises both the profile timeline and production compositor.
    let life_times = [0.0, 5.0, 10.0, 100.0];
    let life_frames = render_scene_frames(
        &package.watch,
        &package.rom,
        &package.configuration,
        &level,
        &resources,
        skin,
        background,
        effects,
        particles,
        &[0, 60, 120, 1200],
        1280,
        720,
    )?;
    let life_demo =
        RendererUiConfig::load_profile(root.join("ui-profiles/life-timeline-demo.json"))?;
    for ((frame, time), filename) in life_frames.iter().zip(life_times).zip([
        "life_timeline_1000_at_0s_16_9_1280x720.png",
        "life_timeline_eased_midpoint_at_5s_16_9_1280x720.png",
        "life_timeline_zero_at_10s_16_9_1280x720.png",
        "life_timeline_negative_100_at_100s_16_9_1280x720.png",
    ]) {
        let mut state = frame_state(frame, time, 100.0);
        state.time = time;
        let resolved = life_demo.resolve_presentation_state(&state)?;
        let expected = match time {
            0.0 => 1000.0,
            5.0 => 250.0,
            10.0 => 0.0,
            100.0 => -100.0,
            _ => unreachable!("only fixed validation sample times are used"),
        };
        ensure!(
            resolved.presentation_values.get("life") == Some(&expected),
            "timeline LIFE sample at {time}s did not match {expected}"
        );
        outputs.push(render_ui(
            frame, &life_demo, &state, 1280, 720, filename, &output,
        )?);
    }

    // This fixture is intentionally synthetic and is only a visual oracle for
    // event-specific score, combo, life, judgement, rank, bonus, and note-type
    // styling. Its fixture scoring values are presentation inputs, not game
    // results or scoring advice.
    let mut event_profile = default.clone();
    let scoring = event_profile
        .scoring
        .as_mut()
        .context("asset-backed visual fixture has no external-event scoring adapter")?;
    scoring.initial_life = 1000.0;
    scoring.maximum_life = 1200.0;
    for rule in &mut scoring.rules {
        if rule.event == "hit" && rule.note_type.as_deref() == Some("normal") {
            rule.score = 11948.0;
            rule.life = 0.0;
        } else if rule.event == "hit" && rule.note_type.as_deref() == Some("special") {
            rule.score = 11954.0;
            rule.life = 0.0;
        }
    }
    event_profile.events_file = Some("visual-validation-events.json".into());
    event_profile.load_external_data()?;
    let mut event_state = landscape_state.clone();
    event_state.gameplay = event_profile.gameplay_state_at(time)?;
    ensure!(
        event_state.gameplay.as_ref().is_some_and(|state| {
            state.score == 489874.0
                && state.combo == 41
                && state.life == 1000.0
                && state.judgment.as_deref() == Some("PERFECT")
                && state.note_type.as_deref() == Some("special")
                && state.variables.get("rank_code") == Some(&2.0)
                && state.variables.get("score_bonus") == Some(&931.0)
        }),
        "asset-backed reference fixture did not reduce to the documented synthetic state"
    );
    outputs.push(render_ui(
        &landscape,
        &event_profile,
        &event_state,
        1280,
        720,
        "reference_assets_special_hit_16_9_1280x720_t24.833.png",
        &output,
    )?);
    let mut event_state_1080p = landscape_state_1080p.clone();
    event_state_1080p.gameplay = event_profile.gameplay_state_at(time)?;
    outputs.push(render_ui(
        &landscape_1080p,
        &event_profile,
        &event_state_1080p,
        1920,
        1080,
        "reference_assets_special_hit_16_9_1920x1080_t24.833.png",
        &output,
    )?);
    let mut event_state_4_3 = landscape_state_4_3.clone();
    event_state_4_3.gameplay = event_profile.gameplay_state_at(time)?;
    outputs.push(render_ui(
        &landscape_4_3,
        &event_profile,
        &event_state_4_3,
        1024,
        768,
        "reference_assets_special_hit_4_3_1024x768_t24.833.png",
        &output,
    )?);
    let event_window_names = [
        "reference_before_special_t24.667_f296_16_9_1280x720.png",
        "reference_judgement_rise_t24.750_f297_16_9_1280x720.png",
        "reference_judgement_settled_t24.833_f298_16_9_1280x720.png",
        "reference_judgement_expired_t25.417_f305_16_9_1280x720.png",
    ];
    for (frame, filename) in event_window_frames.iter().zip(event_window_names) {
        let captured_time = frame.report.timeline.unwrap_or(time);
        let mut state = frame_state(frame, captured_time, 20.0);
        state.gameplay = event_profile.gameplay_state_at(captured_time)?;
        outputs.push(render_ui(
            frame,
            &event_profile,
            &state,
            1280,
            720,
            filename,
            &output,
        )?);
    }

    let mut miss_profile = default.clone();
    miss_profile.events_file = Some("visual-validation-miss-events.json".into());
    miss_profile.load_external_data()?;
    let mut miss_state = landscape_state.clone();
    miss_state.gameplay = miss_profile.gameplay_state_at(time)?;
    ensure!(
        miss_state.gameplay.as_ref().is_some_and(|state| {
            state.score == 0.0
                && state.combo == 0
                && state.life == 750.0
                && state.judgment.as_deref() == Some("MISS")
        }),
        "miss visual fixture did not reduce to the documented state"
    );
    outputs.push(render_ui(
        &landscape,
        &miss_profile,
        &miss_state,
        1280,
        720,
        "miss_assets_16_9_1280x720_t24.833.png",
        &output,
    )?);

    let mut minimal = RendererUiConfig::load_profile(root.join("ui-profiles/minimal.json"))?;
    // Exercise custom bitmap-font loading from a repository PNG atlas. This
    // single-glyph atlas intentionally only renders the digit zero.
    minimal.assets.insert(
        "validation-zero".into(),
        "../assets/life/digit/0.png".into(),
    );
    minimal.load_external_data()?;
    let (atlas_width, atlas_height) = minimal
        .loaded_asset_dimensions("validation-zero")
        .context("loaded zero glyph atlas has no decoded dimensions")?;
    minimal.fonts.insert(
        "validation-zero".into(),
        UiBitmapFont {
            asset: "validation-zero".into(),
            glyph_width: atlas_width,
            glyph_height: atlas_height,
            columns: 1,
            first_codepoint: '0' as u32,
            advance: 1.0,
            glyph_map: BTreeMap::new(),
            glyph_assets: BTreeMap::new(),
            glyph_scales: BTreeMap::new(),
            glyph_advances: BTreeMap::new(),
            center_spacing: None,
            glyph_max_dimension: None,
        },
    );
    minimal.elements.push(UiElement {
        id: "bitmap-font-validation".into(),
        layout: UiLayout {
            x: 0.78,
            y: 0.07,
            width: 0.2,
            height: 0.13,
            ..UiLayout::default()
        },
        kind: UiElementKind::Text {
            text: "0000".into(),
            value: None,
            color: [255; 4],
            font_size: 52.0,
            font: Some("validation-zero".into()),
            align: UiTextAlign::Center,
        },
        ..UiElement::default()
    });
    minimal.load_external_data()?;
    outputs.push(render_ui(
        &landscape,
        &minimal,
        &landscape_state,
        1280,
        720,
        "minimal_animation_phase_0.667_bitmap_font_16_9_t24.833.png",
        &output,
    )?);

    let mut terminal = RendererUiConfig::load_profile(root.join("ui-profiles/terminal.json"))?;
    terminal.prepare_external_data()?;
    outputs.push(render_ui(
        &landscape_4_3,
        &terminal,
        &landscape_state_4_3,
        1024,
        768,
        "terminal_field_monitor_4_3_1024x768_t24.833.png",
        &output,
    )?);

    let mut legacy = RendererUiConfig::default();
    legacy.mode = GameplayUiMode::Legacy;
    legacy.enabled = true;
    outputs.push(render_ui(
        &landscape,
        &legacy,
        &landscape_state,
        1280,
        720,
        "legacy_comparison_16_9_1280x720_t24.833.png",
        &output,
    )?);

    let mut manifest = format!(
        "# UI validation renders\n\nGenerated by `cargo run --offline --example ui_validation`. Every PNG begins with a real Watch frame rendered by the CPU renderer with the engine's default background/effect/particle resources selected, then applies the production gameplay UI compositor to those pixels. The level fixture does not provide the music-video/character layer visible in the reference movie.\n\n- Engine: `{}`\n- Level: `{}`\n- Skin resource directory: `{}`\n- Skin: `{skin}`\n- Background resource: `{background:?}`\n- Effects resource: `{effects:?}`\n- Particle resource: `{particles:?}`\n- Renderer backend: CPU\n- Watch FPS: 12\n- Main captured scene/UI time: {time:.3}s (global frame 298)\n- UI progress denominator: 20s\n- Particle rendering: enabled\n- Audio playback: not captured in PNGs\n- Behavioral reference movie: `TestingSuite/ui_ref/Larp 64x Preview.mp4` (30 fps, 1920x1080). Full-resolution frames at 24.800, 24.900, 25.000, 25.100, and 25.500 seconds were inspected. The temporal sequence samples real Watch frames 296, 297, 298, and 305 (24.667s, 24.750s, 24.833s, 25.417s) around the synthetic event at 24.700s.\n- The reference-state screenshots use an explicitly synthetic adapter fixture to demonstrate score 00489874, combo 41, life 1000, rank code B, and bonus 931. Those values are presentation inputs, not authoritative gameplay results. The state/time was selected to match the supplied reference around 24.8s; the movie advances to score 00517813/combo 42 shortly after 25.0s.\n- The 4:3 and 1920x1080 images repeat the same state and time as the 1280x720 landscape image.\n- The default-state image uses configured initial values (score 0, combo 0, life 850), not a player result.\n- The minimal profile adds a validation-only one-glyph PNG atlas for `0`; its looping 0.5s opacity animation is sampled at phase 0.333s (0.667 of the cycle).\n- Video reference analysis and asset-role inventory: `ui-profiles/ASSET_USAGE.md` and `SONO_RENDERER_UI_FOLLOWUP_6_REPORT.md`.\n\n| Image | Profile / state | Time | Resolution | Event data and reduced UI state | Watch state from rendered frame | UI CPU render |\n|---|---|---:|---:|---|---|---:|\n{}\n",
        engine_path.display(),
        level_path.display(),
        resources.display(),
        outputs.join("\n")
    );
    manifest.push_str(
        "\nThe `life_timeline_*` entries use one generic numeric timeline track and the same image/gauge/glyph compositor as the reference template. The 0→10 segment uses Ease Out; the 10→100 segment uses a configured cubic Bezier. The authored presentation value is held separately from Watch gameplay state. SHA-256 identities are in `SHA256SUMS.txt`.\n",
    );
    fs::write(output.join("MANIFEST.md"), manifest)?;
    let mut hashes = String::from("# SHA-256 hashes for generated UI validation PNGs\n\n");
    let mut images: Vec<_> = fs::read_dir(&output)?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "png"))
        .collect();
    images.sort();
    for image in images {
        let (sha256, bytes) = renderer::sha256_file(&image)?;
        let filename = image.file_name().unwrap_or_default().to_string_lossy();
        hashes.push_str(&format!("{sha256}  {filename}  ({bytes} bytes)\n"));
    }
    fs::write(output.join("SHA256SUMS.txt"), hashes)?;
    println!(
        "Wrote visual-validation PNGs and MANIFEST.md to {}",
        output.display()
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_scene(
    watch: &renderer::watch::WatchData,
    rom: &[u8],
    configuration: &serde_json::Value,
    level: &formats::LevelData,
    resources: &Path,
    skin: &str,
    background: Option<&str>,
    effects: Option<&str>,
    particles: Option<&str>,
    time: f64,
    width: u32,
    height: u32,
) -> Result<RenderedFrame> {
    let mut ui = RendererUiConfig::default();
    ui.enabled = false;
    ui.mode = GameplayUiMode::Off;
    let mut session = FrameSession::new_configured(
        watch,
        rom,
        configuration,
        level,
        resources,
        skin,
        background,
        effects,
        particles,
        width,
        height,
        12,
        &[],
        true,
        ui,
        0.0,
        20.0,
        RenderBackend::Cpu,
        false,
        false,
    )?;
    let frame = session.render_global_frame((time * 12.0).ceil() as u64)?;
    if frame.report.callbacks.is_empty() {
        bail!("real Watch render at {time}s produced no callbacks");
    }
    Ok(frame)
}

#[allow(clippy::too_many_arguments)]
fn render_scene_frames(
    watch: &renderer::watch::WatchData,
    rom: &[u8],
    configuration: &serde_json::Value,
    level: &formats::LevelData,
    resources: &Path,
    skin: &str,
    background: Option<&str>,
    effects: Option<&str>,
    particles: Option<&str>,
    global_frames: &[u64],
    width: u32,
    height: u32,
) -> Result<Vec<RenderedFrame>> {
    let mut ui = RendererUiConfig::default();
    ui.enabled = false;
    ui.mode = GameplayUiMode::Off;
    let mut session = FrameSession::new_configured(
        watch,
        rom,
        configuration,
        level,
        resources,
        skin,
        background,
        effects,
        particles,
        width,
        height,
        12,
        &[],
        true,
        ui,
        0.0,
        20.0,
        RenderBackend::Cpu,
        false,
        false,
    )?;
    let mut frames = Vec::with_capacity(global_frames.len());
    for global_frame in global_frames {
        let frame = session.render_global_frame(*global_frame)?;
        if frame.report.callbacks.is_empty() {
            bail!("real Watch render at global frame {global_frame} produced no callbacks");
        }
        frames.push(frame);
    }
    Ok(frames)
}

fn frame_state(frame: &RenderedFrame, time: f64, duration: f64) -> UiFrameState {
    UiFrameState {
        time: frame.report.timeline.unwrap_or(time),
        progress: (time / duration).clamp(0.0, 1.0),
        runtime_entities: frame.report.runtime_entity_count,
        active_entities: frame.report.active_entity_count,
        callbacks: frame.report.callbacks.len(),
        draws: frame.report.display_list.sprites.len(),
        scheduled_events: frame.report.audio_events.len()
            + frame.report.scheduled_effects.len()
            + frame.report.particle_events.len(),
        ..UiFrameState::default()
    }
}

#[allow(clippy::too_many_arguments)]
fn render_ui(
    frame: &RenderedFrame,
    config: &RendererUiConfig,
    state: &UiFrameState,
    width: u32,
    height: u32,
    filename: &str,
    output: &Path,
) -> Result<String> {
    let mut rgb = frame.rgb.clone();
    let start = Instant::now();
    render_gameplay_ui(&mut rgb, width, height, config, state)?;
    let ui_cpu_micros = start.elapsed().as_micros();
    if filename.contains("bitmap_font") {
        let changed_region_pixels = rgb
            .chunks_exact(3)
            .zip(frame.rgb.chunks_exact(3))
            .enumerate()
            .filter(|(index, (rendered, base))| {
                let x = *index as u32 % width;
                let y = *index as u32 / width;
                (950..width).contains(&x) && y < 150 && rendered != base
            })
            .count();
        if changed_region_pixels < 20 {
            bail!("bitmap-font validation text did not draw in its expected top-right region");
        }
    }
    image::save_buffer_with_format(
        output.join(filename),
        &rgb,
        width,
        height,
        image::ColorType::Rgb8,
        image::ImageFormat::Png,
    )?;
    let event_input = if filename.starts_with("reference_before_special") {
        Some("synthetic log: 40 normal hits applied; special hit at 24.700s is pending")
    } else if filename.starts_with("reference_") {
        Some("synthetic log: 40 normal hits plus special hit at 24.700s")
    } else if filename.starts_with("miss_assets") {
        Some("synthetic miss at 24.700s")
    } else if filename.starts_with("default_assets_normal") {
        Some("configured initial values only; no gameplay result")
    } else {
        None
    };
    let resolved_state = config.resolve_presentation_state(state)?;
    let event_data = if filename.starts_with("life_timeline_") {
        format!(
            "authored presentation LIFE {:?}; Watch/gameplay state is not modified",
            resolved_state.presentation_values.get("life")
        )
    } else {
        match (event_input, state.gameplay.as_ref()) {
        (Some(input), Some(gameplay)) => format!(
            "{input}; score {:.0}, combo {}, life {:.0}, judgement {:?}, note type {:?}, rank code {:?}, bonus {:?}, latest event {:?}, judgement time {:?}",
            gameplay.score,
            gameplay.combo,
            gameplay.life,
            gameplay.judgment,
            gameplay.note_type,
            gameplay.variables.get("rank_code"),
            gameplay.variables.get("score_bonus"),
            gameplay.latest_event_time,
            gameplay.judgment_time
        ),
        (Some(input), None) => input.to_string(),
        (None, _) => "none".to_string(),
        }
    };
    Ok(format!(
        "| `{filename}` | {} | {:.3} | {width}x{height} | {} | {} active / {} runtime entities, {} callbacks, {} skin draws | {} us |",
        ui_profile_label(filename),
        frame.report.timeline.unwrap_or(state.time),
        event_data,
        state.active_entities,
        state.runtime_entities,
        state.callbacks,
        state.draws,
        ui_cpu_micros
    ))
}

fn ui_profile_label(filename: &str) -> &'static str {
    if filename.starts_with("default_assets_")
        || filename.starts_with("reference_assets_")
        || filename.starts_with("miss_assets_")
    {
        "`default.json`"
    } else if filename.starts_with("life_timeline_") {
        "`life-timeline-demo.json` (authored LIFE track)"
    } else if filename.starts_with("minimal_") {
        "`minimal.json` plus validation-only zero glyph atlas"
    } else if filename.starts_with("terminal_") {
        "`terminal.json`"
    } else if filename.starts_with("reference_") {
        "`default.json` + timestamped synthetic event fixture"
    } else {
        "legacy renderer overlay"
    }
}
