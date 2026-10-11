//! Local, versioned onboarding preferences. No analytics or render settings live here.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const FORMAT: &str = "sono-renderer-onboarding";
const SCHEMA_VERSION: u32 = 2;
pub(crate) const WELCOME_CONTENT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct OnboardingPreferences {
    format: String,
    schema_version: u32,
    pub welcome_completed: bool,
    /// True when the user explicitly skipped the welcome rather than entering a workflow.
    pub welcome_skipped: bool,
    /// Content version completed or skipped; retained so future optional guidance can evolve
    /// without resetting the first-run experience on every application update.
    pub welcome_version_completed: u32,
    pub gameplay_presentation_tour_completed: bool,
    pub first_successful_job_id: Option<String>,
    pub first_successful_output: Option<PathBuf>,
}

impl Default for OnboardingPreferences {
    fn default() -> Self {
        Self {
            format: FORMAT.into(),
            schema_version: SCHEMA_VERSION,
            welcome_completed: false,
            welcome_skipped: false,
            welcome_version_completed: 0,
            gameplay_presentation_tour_completed: false,
            first_successful_job_id: None,
            first_successful_output: None,
        }
    }
}

impl OnboardingPreferences {
    pub fn should_show_welcome(&self) -> bool {
        !self.welcome_completed
    }

    pub fn load(path: &Path) -> Result<Self> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
        };
        let mut preferences: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing onboarding preferences {}", path.display()))?;
        if preferences.format != FORMAT {
            bail!(
                "{} is not a Sono-Renderer onboarding preference file",
                path.display()
            );
        }
        if preferences.schema_version > SCHEMA_VERSION {
            bail!(
                "onboarding preferences use unsupported schema version {}",
                preferences.schema_version
            );
        }
        if preferences.schema_version == 0 {
            bail!("onboarding preference schema version must be positive");
        }
        match preferences.schema_version {
            1 => {
                // Existing users who completed the old welcome have already opted out
                // of first-run guidance. Migration must not show it again.
                if preferences.welcome_completed {
                    preferences.welcome_version_completed = WELCOME_CONTENT_VERSION;
                }
                preferences.schema_version = 2;
            }
            SCHEMA_VERSION => {}
            version => bail!("unsupported onboarding preference schema version {version}"),
        }
        Ok(preferences)
    }

    pub fn complete_welcome(&mut self) {
        self.welcome_completed = true;
        self.welcome_skipped = false;
        self.welcome_version_completed = WELCOME_CONTENT_VERSION;
    }

    pub fn skip_welcome(&mut self) {
        self.welcome_completed = true;
        self.welcome_skipped = true;
        self.welcome_version_completed = WELCOME_CONTENT_VERSION;
    }

    pub fn reset_welcome(&mut self) {
        self.welcome_completed = false;
        self.welcome_skipped = false;
        self.welcome_version_completed = 0;
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if self.format != FORMAT || self.schema_version != SCHEMA_VERSION {
            bail!("cannot save onboarding preferences with an unsupported format or schema");
        }
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        let bytes =
            serde_json::to_vec_pretty(self).context("serializing onboarding preferences")?;
        let mut temp = tempfile::NamedTempFile::new_in(parent).with_context(|| {
            format!("creating temporary preference file in {}", parent.display())
        })?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        temp.persist(path)
            .map_err(|error| error.error)
            .with_context(|| format!("saving onboarding preferences {}", path.display()))?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    }
}

#[cfg(not(test))]
pub(crate) fn default_path() -> PathBuf {
    if let Some(path) = std::env::var_os("SONO_RENDERER_ONBOARDING_FILE") {
        return PathBuf::from(path);
    }
    crate::store_paths::config_root().join("onboarding.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_preferences_show_welcome_then_round_trip_completion() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("onboarding.json");
        let mut state = OnboardingPreferences::load(&path).unwrap();
        assert!(!state.welcome_completed);
        state.complete_welcome();
        state.gameplay_presentation_tour_completed = true;
        state.first_successful_job_id = Some("job-17".into());
        state.first_successful_output = Some(PathBuf::from("first.mp4"));
        state.save(&path).unwrap();
        assert_eq!(OnboardingPreferences::load(&path).unwrap(), state);
    }

    #[test]
    fn schema_one_completed_state_migrates_without_showing_welcome_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("onboarding.json");
        fs::write(
            &path,
            r#"{"format":"sono-renderer-onboarding","schema_version":1,"welcome_completed":true,"gameplay_presentation_tour_completed":true,"first_successful_job_id":"job-4","first_successful_output":"out.mp4"}"#,
        )
        .unwrap();

        let migrated = OnboardingPreferences::load(&path).unwrap();

        assert!(migrated.welcome_completed);
        assert!(!migrated.should_show_welcome());
        assert!(!migrated.welcome_skipped);
        assert_eq!(migrated.welcome_version_completed, WELCOME_CONTENT_VERSION);
        assert!(migrated.gameplay_presentation_tour_completed);
        assert_eq!(migrated.first_successful_job_id.as_deref(), Some("job-4"));
        assert_eq!(migrated.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn incomplete_schema_one_state_remains_first_run_after_migration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("onboarding.json");
        fs::write(
            &path,
            r#"{"format":"sono-renderer-onboarding","schema_version":1,"welcome_completed":false}"#,
        )
        .unwrap();

        let migrated = OnboardingPreferences::load(&path).unwrap();

        assert!(migrated.should_show_welcome());
        assert_eq!(migrated.welcome_version_completed, 0);
    }

    #[test]
    fn explicit_skip_persists_and_reset_allows_manual_replay() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("onboarding.json");
        let mut state = OnboardingPreferences::default();
        state.skip_welcome();
        state.save(&path).unwrap();

        let mut reopened = OnboardingPreferences::load(&path).unwrap();
        assert!(reopened.welcome_completed);
        assert!(reopened.welcome_skipped);
        reopened.reset_welcome();
        assert!(!reopened.welcome_completed);
        assert!(!reopened.welcome_skipped);
        assert_eq!(reopened.welcome_version_completed, 0);
    }

    #[test]
    fn completing_welcome_keeps_it_dismissed_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("onboarding.json");
        let mut state = OnboardingPreferences::default();
        assert!(state.should_show_welcome());
        state.complete_welcome();
        state.save(&path).unwrap();

        let reopened = OnboardingPreferences::load(&path).unwrap();
        assert!(!reopened.should_show_welcome());
        assert!(!reopened.welcome_skipped);
        assert_eq!(reopened.welcome_version_completed, WELCOME_CONTENT_VERSION);
    }

    #[test]
    fn unsupported_onboarding_schema_is_reported_instead_of_reset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("onboarding.json");
        fs::write(
            &path,
            r#"{"format":"sono-renderer-onboarding","schema_version":99}"#,
        )
        .unwrap();
        assert!(OnboardingPreferences::load(&path)
            .unwrap_err()
            .to_string()
            .contains("unsupported schema"));
    }
}
