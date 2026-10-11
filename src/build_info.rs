//! Identity embedded in this executable by Cargo's single package version.

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COMMIT: &str = env!("SONO_RENDERER_GIT_COMMIT");
pub const DIRTY: &str = env!("SONO_RENDERER_BUILD_DIRTY");
pub const CHANNEL: &str = env!("SONO_RENDERER_BUILD_CHANNEL");
pub const TARGET: &str = env!("SONO_RENDERER_BUILD_TARGET");
pub const PROFILE: &str = env!("SONO_RENDERER_BUILD_PROFILE");
pub const BUILD_ID: &str = env!("SONO_RENDERER_BUILD_ID");

pub fn is_dirty() -> bool {
    DIRTY == "true"
}

pub fn about_text() -> String {
    format!(
        "Sono-Renderer {VERSION}\nBuild: {BUILD_ID} ({PROFILE})\nLicense: MIT; see LICENSE and THIRD_PARTY_NOTICES.md"
    )
}
