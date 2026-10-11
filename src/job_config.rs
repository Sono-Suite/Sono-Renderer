//! Shared queue scheduling settings.
//!
//! RenderConfig carries these settings for compatibility with the common GUI,
//! CLI, profile, and project configuration model. Keeping their data types
//! outside the manager lets the renderer depend on settings without depending
//! on queue execution code.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

pub(crate) const MAX_WORKERS: usize = 8;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum JobPriority {
    High,
    #[default]
    Normal,
    Low,
}

impl JobPriority {
    pub(crate) fn age_bias_seconds(self) -> i64 {
        match self {
            Self::High => 7_200,
            Self::Normal => 3_600,
            Self::Low => 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct JobManagerConfig {
    /// Conservative by default because each render can own a GPU context,
    /// decoder state, audio buffers, and an FFmpeg process.
    pub max_concurrent_jobs: usize,
    pub default_priority: JobPriority,
}

impl Default for JobManagerConfig {
    fn default() -> Self {
        Self {
            max_concurrent_jobs: 1,
            default_priority: JobPriority::Normal,
        }
    }
}

impl JobManagerConfig {
    pub fn validate(&self) -> Result<()> {
        if !(1..=MAX_WORKERS).contains(&self.max_concurrent_jobs) {
            bail!("maximum concurrent render jobs must be in 1..={MAX_WORKERS}");
        }
        Ok(())
    }
}
