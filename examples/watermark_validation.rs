//! Development-only screenshots of the production watermark compositor over
//! actual Watch-rendered frames. This is not a public CLI or desktop feature.
//!
//! Run from the repository root with:
//! `cargo run --offline --example watermark_validation`

use anyhow::{bail, Context, Result};
use renderer::{
    formats,
    offline::{FrameSession, RenderedFrame},
    render::{RenderBackend, ResourceOverrides},
    render_ui::{GameplayUiMode, RendererUiConfig},
    watermark::WatermarkConfig,
};
use std::{collections::BTreeMap, fs, path::Path};

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
        .context("engine has no skin selection")?;
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

    let output = root.join("artifacts/watermark-validation");
    fs::create_dir_all(&output)?;
    let main_time = 298.0 / 12.0;
    let image_profile =
        WatermarkConfig::load_profile(root.join("watermark-profiles/validation-image.json"))?;
    let mut image_session = make_session(
        &package.watch,
        &package.rom,
        &package.configuration,
        &level,
        &resources,
        skin,
        background,
        effects,
        particles,
        1280,
        720,
        image_profile.clone(),
    )?;
    let image_main = image_session.render_global_frame(298)?;
    save(
        &image_main,
        &output,
        "image_random_waypoint_16x9_1280x720_t24.833.png",
        1280,
        720,
    )?;
    let image_later = image_session.render_global_frame(308)?;
    save(
        &image_later,
        &output,
        "image_random_waypoint_16x9_1280x720_t25.667.png",
        1280,
        720,
    )?;

    let mut image_4_3_session = make_session(
        &package.watch,
        &package.rom,
        &package.configuration,
        &level,
        &resources,
        skin,
        background,
        effects,
        particles,
        1024,
        768,
        image_profile,
    )?;
    let image_4_3 = image_4_3_session.render_global_frame(298)?;
    save(
        &image_4_3,
        &output,
        "image_random_waypoint_4x3_1024x768_t24.833.png",
        1024,
        768,
    )?;

    let text_profile =
        WatermarkConfig::load_profile(root.join("watermark-profiles/validation-text.json"))?;
    let mut text_session = make_session(
        &package.watch,
        &package.rom,
        &package.configuration,
        &level,
        &resources,
        skin,
        background,
        effects,
        particles,
        1280,
        720,
        text_profile,
    )?;
    let text_frame = text_session.render_global_frame(298)?;
    save(
        &text_frame,
        &output,
        "text_waypoints_16x9_1280x720_t24.833.png",
        1280,
        720,
    )?;

    let manifest = format!(
        "# Follow-up #7 watermark validation\n\nGenerated with `cargo run --offline --example watermark_validation`. These PNGs are actual Watch scene frames from the CPU renderer with the production `FrameSession` watermark stage enabled. No scene or watermark pixels are mocked.\n\n- Engine: `{}`\n- Level: `{}`\n- Skin resources: `{}`\n- Skin/background/effects/particles: `{skin}` / `{background:?}` / `{effects:?}` / `{particles:?}`\n- Backend: CPU; the exact same compositor runs after WGPU's existing framebuffer readback.\n- Watch time: {main_time:.3}s (frame 298) and 25.667s (frame 308)\n- Main profile: `watermark-profiles/validation-image.json`; image asset is `assets/judge/1.png`, seed 20261006, bounded deterministic random waypoints, smooth-step interpolation, repeated 10s visibility, animated scale/rotation/opacity.\n- Text profile: `watermark-profiles/validation-text.json`; `WATERMARK • SEED 42`, looping waypoints, smooth-step interpolation, animation.\n- Both profiles retain the exact fixed attribution `※Made with「Sono-Renderer」by Nexint.` at the safe bottom-left.\n- The 4:3 image repeats the same timestamp and seed using the same relative bounds.\n\n| File | Profile / state | Time | Resolution |\n|---|---|---:|---:|\n| `image_random_waypoint_16x9_1280x720_t24.833.png` | Image + seeded random waypoints | 24.833s | 1280x720 |\n| `image_random_waypoint_16x9_1280x720_t25.667.png` | Same profile, later deterministic pose | 25.667s | 1280x720 |\n| `image_random_waypoint_4x3_1024x768_t24.833.png` | Same seed and state at 4:3 | 24.833s | 1024x768 |\n| `text_waypoints_16x9_1280x720_t24.833.png` | Custom text + configured waypoints | 24.833s | 1280x720 |\n",
        engine_path.display(),
        level_path.display(),
        resources.display(),
    );
    fs::write(output.join("MANIFEST.md"), manifest)?;
    println!(
        "Wrote production-renderer watermark PNGs to {}",
        output.display()
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn make_session<'a>(
    watch: &'a renderer::watch::WatchData,
    rom: &[u8],
    configuration: &serde_json::Value,
    level: &formats::LevelData,
    resources: &Path,
    skin: &str,
    background: Option<&str>,
    effects: Option<&str>,
    particles: Option<&str>,
    width: u32,
    height: u32,
    watermark: WatermarkConfig,
) -> Result<FrameSession<'a>> {
    let mut ui = RendererUiConfig::default();
    ui.enabled = false;
    ui.mode = GameplayUiMode::Off;
    FrameSession::new_configured_with_sources_and_watermark(
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
        60.0,
        RenderBackend::Cpu,
        false,
        false,
        &ResourceOverrides::default(),
        watermark,
    )
}

fn save(frame: &RenderedFrame, output: &Path, name: &str, width: u32, height: u32) -> Result<()> {
    image::save_buffer_with_format(
        output.join(name),
        &frame.rgb,
        width,
        height,
        image::ColorType::Rgb8,
        image::ImageFormat::Png,
    )?;
    Ok(())
}
