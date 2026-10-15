//! Deterministic chart-score forecast matching Sono-Overlay's `ped.go` rules.
//!
//! This adapter consumes Sonolus LevelData only; it does not inspect Watch
//! memory, render calls, audio, or gameplay pixels. It intentionally models
//! the reference forecast's perfect-judge score path. It is not a player
//! replay, a miss simulator, or a universal Sonolus scoring contract.

use crate::formats::LevelData;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const WEIGHTS: &[(&str, f64)] = &[
    // Nanashi family.
    ("#BPM_CHANGE", 0.0),
    ("Initialization", 0.0),
    ("InputManager", 0.0),
    ("Stage", 0.0),
    ("NormalTapNote", 1.0),
    ("CriticalTapNote", 2.0),
    ("NormalFlickNote", 1.0),
    ("CriticalFlickNote", 3.0),
    ("NormalSlideStartNote", 1.0),
    ("CriticalSlideStartNote", 2.0),
    ("NormalSlideEndNote", 1.0),
    ("CriticalSlideEndNote", 2.0),
    ("NormalSlideEndFlickNote", 1.0),
    ("CriticalSlideEndFlickNote", 3.0),
    ("HiddenSlideTickNote", 0.0),
    ("NormalSlideTickNote", 0.1),
    ("CriticalSlideTickNote", 0.2),
    ("IgnoredSlideTickNote", 0.1),
    ("NormalAttachedSlideTickNote", 0.1),
    ("CriticalAttachedSlideTickNote", 0.2),
    ("NormalSlideConnector", 0.0),
    ("CriticalSlideConnector", 0.0),
    ("SimLine", 0.0),
    ("NormalSlotEffect", 0.0),
    ("SlideSlotEffect", 0.0),
    ("FlickSlotEffect", 0.0),
    ("CriticalSlotEffect", 0.0),
    ("NormalSlotGlowEffect", 0.0),
    ("SlideSlotGlowEffect", 0.0),
    ("FlickSlotGlowEffect", 0.0),
    ("CriticalSlotGlowEffect", 0.0),
    ("NormalTraceNote", 0.1),
    ("CriticalTraceNote", 0.2),
    ("NormalTraceSlotEffect", 0.0),
    ("NormalTraceSlotGlowEffect", 0.0),
    ("DamageNote", 0.1),
    ("DamageSlotEffect", 0.0),
    ("DamageSlotGlowEffect", 0.0),
    ("NormalTraceFlickNote", 1.0),
    ("CriticalTraceFlickNote", 3.0),
    ("NonDirectionalTraceFlickNote", 1.0),
    ("NormalTraceSlideStartNote", 0.1),
    ("NormalTraceSlideEndNote", 0.1),
    ("CriticalTraceSlideStartNote", 0.2),
    ("CriticalTraceSlideEndNote", 0.2),
    // These source names occur in ALL_FLICK but have no base WEIGHT_MAP
    // entry in ped.go; its Go map lookup therefore gives them zero.
    ("NormalTraceSlideEndFlickNote", 0.0),
    ("CriticalTraceSlideEndFlickNote", 0.0),
    ("TimeScaleGroup", 0.0),
    ("TimeScaleChange", 0.0),
    // Next SEKAI family.
    ("#TIMESCALE_CHANGE", 0.0),
    ("#TIMESCALE_GROUP", 0.0),
    // Next Sekai presentation/camera entities verified as non-scoring.
    ("CameraChange", 0.0),
    ("StageMaskChange", 0.0),
    ("StagePivotChange", 0.0),
    ("StageStyleChange", 0.0),
    ("StageTransformChange", 0.0),
    ("_InputManager", 0.0),
    ("SlideManager", 0.0),
    ("Connector", 0.0),
    ("SlotGlowEffect", 0.0),
    ("SlotEffect", 0.0),
    ("NormalHeadTapNote", 1.0),
    ("CriticalHeadTapNote", 2.0),
    ("NormalHeadFlickNote", 1.0),
    ("CriticalHeadFlickNote", 3.0),
    ("NormalHeadTraceNote", 0.1),
    ("CriticalHeadTraceNote", 0.2),
    ("NormalHeadTraceFlickNote", 1.0),
    ("CriticalHeadTraceFlickNote", 3.0),
    ("NormalHeadReleaseNote", 1.0),
    ("CriticalHeadReleaseNote", 2.0),
    ("NormalTailTapNote", 1.0),
    ("CriticalTailTapNote", 2.0),
    ("NormalTailFlickNote", 1.0),
    ("CriticalTailFlickNote", 3.0),
    ("NormalTailTraceNote", 0.1),
    ("CriticalTailTraceNote", 0.2),
    ("NormalTailTraceFlickNote", 1.0),
    ("CriticalTailTraceFlickNote", 3.0),
    ("NormalTailReleaseNote", 1.0),
    ("CriticalTailReleaseNote", 2.0),
    ("TransientHiddenTickNote", 0.1),
    ("NormalTickNote", 0.1),
    ("CriticalTickNote", 0.2),
    ("AnchorNote", 0.0),
    ("FakeNormalTapNote", 0.0),
    ("FakeCriticalTapNote", 0.0),
    ("FakeNormalFlickNote", 0.0),
    ("FakeCriticalFlickNote", 0.0),
    ("FakeNormalTraceNote", 0.0),
    ("FakeCriticalTraceNote", 0.0),
    ("FakeNormalTraceFlickNote", 0.0),
    ("FakeCriticalTraceFlickNote", 0.0),
    ("FakeNormalReleaseNote", 0.0),
    ("FakeCriticalReleaseNote", 0.0),
    ("FakeNormalHeadTapNote", 0.0),
    ("FakeCriticalHeadTapNote", 0.0),
    ("FakeNormalHeadFlickNote", 0.0),
    ("FakeCriticalHeadFlickNote", 0.0),
    ("FakeNormalHeadTraceNote", 0.0),
    ("FakeCriticalHeadTraceNote", 0.0),
    ("FakeNormalHeadTraceFlickNote", 0.0),
    ("FakeCriticalHeadTraceFlickNote", 0.0),
    ("FakeNormalHeadReleaseNote", 0.0),
    ("FakeCriticalHeadReleaseNote", 0.0),
    ("FakeNormalTailTapNote", 0.0),
    ("FakeCriticalTailTapNote", 0.0),
    ("FakeNormalTailFlickNote", 0.0),
    ("FakeCriticalTailFlickNote", 0.0),
    ("FakeNormalTailTraceNote", 0.0),
    ("FakeCriticalTailTraceNote", 0.0),
    ("FakeNormalTailTraceFlickNote", 0.0),
    ("FakeCriticalTailTraceFlickNote", 0.0),
    ("FakeNormalTailReleaseNote", 0.0),
    ("FakeCriticalTailReleaseNote", 0.0),
    ("FakeTransientHiddenTickNote", 0.0),
    ("FakeNormalTickNote", 0.0),
    ("FakeCriticalTickNote", 0.0),
    ("FakeAnchorNote", 0.0),
    ("FakeDamageNote", 0.0),
    ("Skill", 0.0),
    ("FeverChance", 0.0),
    ("FeverStart", 0.0),
    // Horizon family.
    ("TapNote", 1.0),
    ("DoubleHoldNote", 0.2),
    ("SingleHoldNote", 0.1),
    ("RotateEvent", 0.0),
    ("ShiftEvent", 0.0),
    ("ZoomEvent", 0.0),
    ("LaneEffect", 0.0),
    ("LaneActivation", 0.0),
    ("SingleHoldConnector", 0.0),
    ("DoubleHoldConnector", 0.0),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ScoreMode {
    #[default]
    Standard,
    Tournament,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ForecastConfig {
    /// Sono-Overlay's fixed team-power input. It does not increase with combo.
    pub team_power: f64,
    /// Uses LevelInfo.Rating when available; otherwise the user supplies it.
    pub level_rating: Option<f64>,
    pub score_mode: ScoreMode,
    /// Sono-Overlay's explicit all-flick note mapping, which changes weighted
    /// contributions for selected tap/trace archetypes.
    pub all_flick: bool,
    /// Sono-Overlay's `Skill` chart entity applies its fixed 2x/5s rule.
    pub chart_skills_enabled: bool,
    /// The reference default.ini selects judgment image 1 (PERFECT).
    /// None suppresses judgment art without changing the score forecast.
    pub judgment: Option<String>,
    pub initial_life: f64,
    pub minimum_life: f64,
    pub maximum_life: f64,
}

impl Default for ForecastConfig {
    fn default() -> Self {
        Self {
            team_power: 250_000.0,
            level_rating: None,
            score_mode: ScoreMode::Standard,
            all_flick: false,
            chart_skills_enabled: true,
            judgment: Some("PERFECT".into()),
            initial_life: 1_000.0,
            minimum_life: 0.0,
            maximum_life: 1_000.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NoteEvent {
    pub time: f64,
    pub beat: f64,
    pub source_entity: usize,
    pub archetype: String,
    pub weight: f64,
    /// The reference increments combo once for each positive-weight entity,
    /// including low-weight ticks. It does not count connectors.
    pub combo_contribution: u8,
    pub skill_activation: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForecastFrame {
    pub time: f64,
    pub source_entity: Option<usize>,
    pub archetype: Option<String>,
    pub skill_activation: bool,
    pub note_weight: f64,
    pub score: f64,
    pub score_delta: f64,
    pub combo: i64,
    pub life: f64,
    pub judgment: Option<String>,
    pub rank_code: f64,
    pub score_progress: f64,
    pub skill_active: bool,
    pub skill_activation_time: Option<f64>,
    /// Timestamp of the latest `d` row used by Sono-Overlay's HUD. The source
    /// writes both scoring notes and Skill activations to this timeline; a
    /// Skill can restart the combo pop without changing score/combo/judgment.
    pub combo_event_time: Option<f64>,
    /// Previous timeline-row timestamp, used by Sono-Overlay's sub-frame
    /// combo animation fallback.
    pub previous_combo_event_time: Option<f64>,
    /// Latest positive-weight note that can drive judgment artwork.
    pub judgment_time: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoringDiagnostic {
    pub archetype: String,
    pub entity_indices: Vec<usize>,
    pub potentially_scoring: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedEvents {
    pub events: Vec<NoteEvent>,
    pub weighted_note_count: f64,
    pub rank_weighted_note_count: f64,
    pub ignored_entities: usize,
    pub diagnostics: Vec<ScoringDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractionPolicy {
    Permissive,
    Strict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RatingSource {
    UserOverride,
    LevelMetadata,
    DefaultZero,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Forecast {
    pub events: Vec<NoteEvent>,
    /// Includes an initial frame at time zero and then stable source-order
    /// snapshots for each scored event or chart Skill activation.
    pub frames: Vec<ForecastFrame>,
    pub weighted_note_count: f64,
    /// The reference rank writer uses the base (non-all-flick) count for
    /// tournament rank thresholds even when score calculation uses all-flick.
    pub rank_weighted_note_count: f64,
    pub supported_notes: usize,
    pub ignored_entities: usize,
    pub level_rating: f64,
    pub rating_source: RatingSource,
    pub diagnostics: Vec<ScoringDiagnostic>,
}

impl Forecast {
    pub fn potentially_incomplete(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.potentially_scoring)
    }
}

impl Forecast {
    /// State before a Watch-time timestamp, matching Sono-Overlay's strict
    /// `event_time * fps < frame - OFFSET` selection rule. `partition_point`
    /// makes direct seeks logarithmic and independent of prior preview requests.
    pub fn state_at(&self, time: f64) -> Option<&ForecastFrame> {
        if !time.is_finite() || self.frames.is_empty() {
            return None;
        }
        let index = self.frames.partition_point(|frame| frame.time < time);
        index
            .checked_sub(1)
            .map(|index| &self.frames[index])
            // The synthetic initial row represents the reference's initialized
            // time-zero state. It must remain available at t=0 even though a
            // real event at t=0 is selected only after the strict boundary.
            .or_else(|| (time == 0.0).then(|| &self.frames[0]))
    }
}

#[derive(Debug, Clone, PartialEq)]
struct BpmChange {
    beat: f64,
    bpm: f64,
    source_entity: usize,
}

fn known_weight(archetype: &str) -> Option<f64> {
    WEIGHTS
        .iter()
        .find_map(|(name, weight)| (*name == archetype).then_some(*weight))
}

fn note_weight(archetype: &str, all_flick: bool) -> Option<f64> {
    if all_flick {
        let flick = match archetype {
            "NormalTapNote" => "NormalFlickNote",
            "CriticalTapNote" => "CriticalFlickNote",
            "NormalSlideEndNote" => "NormalSlideEndFlickNote",
            "CriticalSlideEndNote" => "CriticalSlideEndFlickNote",
            "NormalTraceNote" => "NormalTraceFlickNote",
            "CriticalTraceNote" => "CriticalTraceFlickNote",
            "NormalTraceSlideEndNote" => "NormalTraceSlideEndFlickNote",
            "CriticalTraceSlideEndNote" => "CriticalTraceSlideEndFlickNote",
            "NormalTailTapNote" => "NormalTailFlickNote",
            "CriticalTailTapNote" => "CriticalTailFlickNote",
            "NormalTailTraceNote" => "NormalTailTraceFlickNote",
            "CriticalTailTraceNote" => "CriticalTailTraceFlickNote",
            "NormalTailReleaseNote" => "NormalTailFlickNote",
            "CriticalTailReleaseNote" => "CriticalTailFlickNote",
            _ => archetype,
        };
        known_weight(flick)
    } else {
        known_weight(archetype)
    }
}

fn named_number(value: &Value, name: &str) -> Option<f64> {
    value.as_array()?.iter().find_map(|field| {
        (field.get("name").and_then(Value::as_str) == Some(name))
            .then(|| field.get("value").and_then(Value::as_f64))
            .flatten()
    })
}

fn has_named_field(value: &Value, name: &str) -> bool {
    value.as_array().is_some_and(|fields| {
        fields
            .iter()
            .any(|field| field.get("name").and_then(Value::as_str) == Some(name))
    })
}

fn inferred_rating(metadata: Option<&Value>) -> Option<f64> {
    let rating = metadata?.get("rating")?;
    rating.as_f64().or_else(|| {
        ["level", "value"]
            .into_iter()
            .find_map(|key| rating.get(key).and_then(Value::as_f64))
    })
}

fn resolve_event_times(events: &mut [NoteEvent], changes: &[BpmChange]) -> Result<()> {
    for event in events {
        let mut time = 0.0;
        for (index, change) in changes.iter().enumerate() {
            let Some(next) = changes.get(index + 1) else {
                time += (event.beat - change.beat) * (60.0 / change.bpm);
                break;
            };
            if event.beat >= change.beat && event.beat < next.beat {
                time += (event.beat - change.beat) * (60.0 / change.bpm);
                break;
            } else if event.beat >= next.beat {
                time += (next.beat - change.beat) * (60.0 / change.bpm);
            } else {
                // The reference returns the accumulated zero when a beat is
                // before the first BPM change.
                break;
            }
        }
        if !time.is_finite() {
            bail!(
                "note entity {} resolved to an invalid time",
                event.source_entity
            );
        }
        event.time = time;
    }
    Ok(())
}

/// Extracts the verified Sono-Overlay scoring subset. Unknown entities never
/// become notes by implication; beat-bearing unknowns are retained as
/// best-effort diagnostics while recognized notes continue to simulate.
pub fn extract_events_with_policy(
    level: &LevelData,
    all_flick: bool,
    policy: ExtractionPolicy,
) -> Result<ExtractedEvents> {
    let mut changes = Vec::new();
    let mut events = Vec::new();
    let mut weighted_note_count = 0.0;
    let mut rank_weighted_note_count = 0.0;
    let mut ignored_entities = 0;
    let mut diagnostics: std::collections::BTreeMap<String, ScoringDiagnostic> =
        std::collections::BTreeMap::new();

    for (source_entity, entity) in level.entities.iter().enumerate() {
        let archetype = entity.archetype.as_str().with_context(|| {
            format!("LevelData entity {source_entity} has a non-string archetype")
        })?;
        if archetype == "#BPM_CHANGE" {
            let beat = named_number(&entity.data, "#BEAT")
                .with_context(|| format!("BPM entity {source_entity} is missing numeric #BEAT"))?;
            let bpm = named_number(&entity.data, "#BPM")
                .with_context(|| format!("BPM entity {source_entity} is missing numeric #BPM"))?;
            if !beat.is_finite() || !bpm.is_finite() || bpm <= 0.0 {
                bail!("BPM entity {source_entity} has invalid beat/BPM values");
            }
            changes.push(BpmChange {
                beat,
                bpm,
                source_entity,
            });
            continue;
        }

        let rank_weight = known_weight(archetype).unwrap_or(0.0);
        if rank_weight > 0.0 {
            rank_weighted_note_count += rank_weight;
        }
        let weight = match note_weight(archetype, all_flick) {
            Some(weight) => weight,
            None => {
                let potentially_scoring = has_named_field(&entity.data, "#BEAT");
                if policy == ExtractionPolicy::Strict && potentially_scoring {
                    bail!(
                        "chart scoring is unsupported: beat-bearing archetype {archetype:?} at entity {source_entity} is not in Sono-Overlay's verified weight map"
                    );
                }
                ignored_entities += 1;
                let diagnostic =
                    diagnostics
                        .entry(archetype.to_owned())
                        .or_insert_with(|| ScoringDiagnostic {
                            archetype: archetype.to_owned(),
                            entity_indices: Vec::new(),
                            potentially_scoring: false,
                        });
                diagnostic.entity_indices.push(source_entity);
                diagnostic.potentially_scoring |= potentially_scoring;
                continue;
            }
        };
        if weight > 0.0 {
            weighted_note_count += weight;
            if entity.data.as_array().is_none_or(Vec::is_empty) {
                ignored_entities += 1;
                continue;
            }
            let beat = named_number(&entity.data, "#BEAT").with_context(|| {
                format!("scoring entity {source_entity} ({archetype}) is missing numeric #BEAT")
            })?;
            if !beat.is_finite() {
                bail!("scoring entity {source_entity} has a non-finite #BEAT");
            }
            events.push(NoteEvent {
                time: 0.0,
                beat,
                source_entity,
                archetype: archetype.to_owned(),
                weight,
                combo_contribution: 1,
                skill_activation: false,
            });
        } else if archetype == "Skill" {
            let beat = named_number(&entity.data, "#BEAT").with_context(|| {
                format!("Skill entity {source_entity} is missing numeric #BEAT")
            })?;
            if !beat.is_finite() {
                bail!("Skill entity {source_entity} has a non-finite #BEAT");
            }
            events.push(NoteEvent {
                time: 0.0,
                beat,
                source_entity,
                archetype: archetype.to_owned(),
                weight: 0.0,
                combo_contribution: 0,
                skill_activation: true,
            });
        } else {
            ignored_entities += 1;
        }
    }

    changes.sort_by(|left, right| {
        left.beat
            .total_cmp(&right.beat)
            .then_with(|| left.source_entity.cmp(&right.source_entity))
    });
    let diagnostics: Vec<_> = diagnostics.into_values().collect();
    if !weighted_note_count.is_finite()
        || (weighted_note_count <= 0.0
            && !diagnostics
                .iter()
                .any(|diagnostic| diagnostic.potentially_scoring))
    {
        bail!("chart has no scoring notes recognized by Sono-Overlay's weight map");
    }
    resolve_event_times(&mut events, &changes)?;
    events.sort_by(|left, right| {
        left.beat
            .total_cmp(&right.beat)
            .then_with(|| left.source_entity.cmp(&right.source_entity))
    });
    Ok(ExtractedEvents {
        events,
        weighted_note_count,
        rank_weighted_note_count,
        ignored_entities,
        diagnostics,
    })
}

/// Compatibility helper for callers that only need the extracted events.
/// Normal application behavior is permissive; use `extract_events_with_policy`
/// when diagnostics must be inspected or strict validation is requested.
pub fn extract_events(
    level: &LevelData,
    all_flick: bool,
) -> Result<(Vec<NoteEvent>, f64, f64, usize)> {
    let extracted = extract_events_with_policy(level, all_flick, ExtractionPolicy::Permissive)?;
    Ok((
        extracted.events,
        extracted.weighted_note_count,
        extracted.rank_weighted_note_count,
        extracted.ignored_entities,
    ))
}

fn rank(score: f64, rating: f64, weighted_count: f64, mode: ScoreMode) -> (f64, f64) {
    let rank_rating = rating.clamp(5.0, 40.0);
    let (border, s, a, b, c) = match mode {
        ScoreMode::Standard => (
            1_200_000.0 + (rank_rating - 5.0) * 4_100.0,
            1_040_000.0 + (rank_rating - 5.0) * 5_200.0,
            840_000.0 + (rank_rating - 5.0) * 4_200.0,
            400_000.0 + (rank_rating - 5.0) * 2_000.0,
            20_000.0 + (rank_rating - 5.0) * 100.0,
        ),
        ScoreMode::Tournament => (
            weighted_count.floor(),
            (weighted_count * 0.8).floor(),
            (weighted_count * 0.666).floor(),
            (weighted_count * 0.533).floor(),
            (weighted_count * 0.4).floor(),
        ),
    };
    let (rank_code, c_pos, b_pos, a_pos, s_pos, border_pos) = (
        1.0,
        746.0 / 1650.0,
        990.0 / 1650.0,
        1234.0 / 1650.0,
        1478.0 / 1650.0,
        1.0,
    );
    if score < 0.0 {
        return (5.0, 0.0);
    }
    if score >= border {
        return (4.0, border_pos);
    }
    let progress = if score >= s {
        s_pos + (score - s) / (border - s) * (border_pos - s_pos)
    } else if score >= a {
        a_pos + (score - a) / (s - a) * (s_pos - a_pos)
    } else if score >= b {
        b_pos + (score - b) / (a - b) * (a_pos - b_pos)
    } else if score >= c {
        c_pos + (score - c) / (b - c) * (b_pos - c_pos)
    } else if c > 0.0 {
        (score / c) * c_pos
    } else {
        0.0
    };
    let code = if score >= s {
        4.0
    } else if score >= a {
        3.0
    } else if score >= b {
        2.0
    } else if score >= c {
        rank_code
    } else {
        5.0
    };
    (code, progress.clamp(0.0, 1.0))
}

/// Simulates the deterministic all-perfect forecast generated by
/// Sono-Overlay's `CalculateScore` and `WritePedFile`.
pub fn simulate(level: &LevelData, config: &ForecastConfig) -> Result<Forecast> {
    if !config.team_power.is_finite()
        || !config.initial_life.is_finite()
        || !config.minimum_life.is_finite()
        || !config.maximum_life.is_finite()
        || config.minimum_life > config.maximum_life
    {
        bail!("Sono-Overlay forecast settings contain invalid numeric values");
    }
    let (rating, rating_source) = if let Some(rating) = config.level_rating {
        (rating, RatingSource::UserOverride)
    } else if let Some(rating) = inferred_rating(level.metadata.as_ref()).filter(|r| r.is_finite())
    {
        (rating, RatingSource::LevelMetadata)
    } else {
        (0.0, RatingSource::DefaultZero)
    };
    if !rating.is_finite() {
        bail!("Sono-Overlay level rating must be finite");
    }
    let extracted =
        extract_events_with_policy(level, config.all_flick, ExtractionPolicy::Permissive)?;
    let events = extracted.events;
    let weighted_note_count = extracted.weighted_note_count;
    let rank_weighted_note_count = extracted.rank_weighted_note_count;
    let ignored_entities = extracted.ignored_entities;
    let diagnostics = extracted.diagnostics;
    let level_fax = (rating - 5.0) * 0.005 + 1.0;
    let mut score = 0.0;
    let mut combo = 0_i64;
    let mut combo_fax: f64 = 1.0;
    let mut skill_active_until = None;
    let mut skill_activation_time = None;
    let mut previous_score = 0.0;
    let mut frames = vec![ForecastFrame {
        time: 0.0,
        source_entity: None,
        archetype: None,
        skill_activation: false,
        note_weight: 0.0,
        score: 0.0,
        score_delta: 0.0,
        combo: 0,
        life: config
            .initial_life
            .clamp(config.minimum_life, config.maximum_life),
        judgment: None,
        rank_code: 5.0,
        score_progress: 0.0,
        skill_active: false,
        skill_activation_time: None,
        combo_event_time: None,
        previous_combo_event_time: None,
        judgment_time: None,
    }];
    let mut combo_event_time = None;
    let mut judgment_time = None;
    let mut last_judgment = None;

    for event in &events {
        let previous_combo_event_time = combo_event_time;
        if event.skill_activation {
            if config.chart_skills_enabled {
                skill_active_until = Some(event.time + 5.0);
                skill_activation_time = Some(event.time);
            }
            // ped.go emits a `d` timeline row for a Skill marker. sekai.obj2
            // anchors the combo animation to PED_DATA.current.time, so the
            // unchanged combo pop restarts at this time while score, combo,
            // and judgment feedback remain untouched.
            combo_event_time = Some(event.time);
        } else {
            combo_event_time = Some(event.time);
            judgment_time = (event.time > 0.0)
                .then(|| config.judgment.clone())
                .flatten()
                .map(|_| event.time);
            last_judgment = (event.time > 0.0)
                .then(|| config.judgment.clone())
                .flatten();
            combo = combo.saturating_add(i64::from(event.combo_contribution));
            if combo > 1 && combo % 100 == 1 {
                combo_fax = (combo_fax + 0.01).min(1.1);
            }
            let skill_fax = if config.chart_skills_enabled
                && skill_active_until.is_some_and(|until| event.time <= until)
            {
                2.0
            } else {
                1.0
            };
            let increment = match config.score_mode {
                ScoreMode::Standard => {
                    (config.team_power / weighted_note_count)
                        * 4.0
                        * event.weight
                        * 1.0 // ped.go's judge weight is always PERFECT (1).
                        * level_fax
                        * combo_fax
                        * skill_fax
                }
                ScoreMode::Tournament => 3.0,
            };
            score += increment;
            if !score.is_finite() {
                bail!("Sono-Overlay forecast produced a non-finite score");
            }
        }

        let (rank_code, score_progress) =
            rank(score, rating, rank_weighted_note_count, config.score_mode);
        let score_delta = (score - previous_score).trunc();
        previous_score = score;
        let skill_active = config.chart_skills_enabled
            && skill_active_until.is_some_and(|until| event.time <= until);
        frames.push(ForecastFrame {
            time: event.time,
            source_entity: Some(event.source_entity),
            archetype: Some(event.archetype.clone()),
            skill_activation: event.skill_activation,
            note_weight: event.weight,
            score: score.round_ties_even(),
            score_delta,
            combo,
            life: config
                .initial_life
                .clamp(config.minimum_life, config.maximum_life),
            // Skill markers are not note/judgment events. Keep the latest
            // scored-note feedback visible until its timestamp-based expiry.
            judgment: last_judgment.clone(),
            rank_code,
            score_progress,
            skill_active,
            skill_activation_time,
            combo_event_time,
            previous_combo_event_time,
            judgment_time,
        });
    }

    frames.sort_by(|left, right| left.time.total_cmp(&right.time));

    Ok(Forecast {
        supported_notes: events
            .iter()
            .filter(|event| !event.skill_activation)
            .count(),
        events,
        frames,
        weighted_note_count,
        rank_weighted_note_count,
        ignored_entities,
        level_rating: rating,
        rating_source,
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entity(archetype: &str, beat: Option<f64>) -> crate::formats::LevelEntity {
        let data = beat
            .map(|beat| json!([{ "name": "#BEAT", "value": beat }]))
            .unwrap_or_else(|| json!([]));
        crate::formats::LevelEntity {
            archetype: Value::String(archetype.into()),
            data,
            extra: Default::default(),
        }
    }

    fn level() -> LevelData {
        LevelData {
            bgm_offset: Some(0.25),
            entities: vec![
                crate::formats::LevelEntity {
                    archetype: Value::String("#BPM_CHANGE".into()),
                    data: json!([
                        { "name": "#BEAT", "value": 0.0 },
                        { "name": "#BPM", "value": 120.0 }
                    ]),
                    extra: Default::default(),
                },
                entity("NormalTapNote", Some(1.0)),
                entity("CriticalTapNote", Some(2.0)),
                entity("Skill", Some(3.0)),
                entity("NormalTapNote", Some(4.0)),
                entity("Connector", Some(4.0)),
                entity("Stage", None),
            ],
            metadata: None,
            extra: Default::default(),
        }
    }

    #[test]
    fn extraction_uses_only_reference_weighted_entities_and_bpm_timeline() {
        let (events, weighted, rank_weighted, ignored) = extract_events(&level(), false).unwrap();
        assert_eq!(weighted, 4.0);
        assert_eq!(rank_weighted, 4.0);
        assert_eq!(ignored, 2);
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].time, 0.5);
        assert_eq!(events[0].weight, 1.0);
        assert_eq!(events[1].time, 1.0);
        assert_eq!(events[1].weight, 2.0);
        assert_eq!(events[2].time, 1.5);
        assert!(events[2].skill_activation);
        assert_eq!(events[2].combo_contribution, 0);
        assert_eq!(events[3].time, 2.0);
    }

    #[test]
    fn bpm_conversion_matches_reference_zero_fallback_and_stable_same_beat_changes() {
        let mut no_bpm = level();
        no_bpm.entities.remove(0);
        let extracted =
            extract_events_with_policy(&no_bpm, false, ExtractionPolicy::Permissive).unwrap();
        assert!(extracted.events.iter().all(|event| event.time == 0.0));

        let mut before_first = level();
        before_first.entities[0].data = json!([
            { "name": "#BEAT", "value": 2.0 },
            { "name": "#BPM", "value": 120.0 }
        ]);
        before_first.entities.insert(
            1,
            crate::formats::LevelEntity {
                archetype: Value::String("#BPM_CHANGE".into()),
                data: json!([
                    { "name": "#BEAT", "value": 4.0 },
                    { "name": "#BPM", "value": 60.0 }
                ]),
                extra: Default::default(),
            },
        );
        let extracted =
            extract_events_with_policy(&before_first, false, ExtractionPolicy::Permissive).unwrap();
        assert_eq!(
            extracted.events[0].time, 0.0,
            "pre-first-BPM beat follows ped.go"
        );

        let mut same_beat = level();
        same_beat.entities.insert(
            1,
            crate::formats::LevelEntity {
                archetype: Value::String("#BPM_CHANGE".into()),
                data: json!([
                    { "name": "#BEAT", "value": 0.0 },
                    { "name": "#BPM", "value": 60.0 }
                ]),
                extra: Default::default(),
            },
        );
        let extracted =
            extract_events_with_policy(&same_beat, false, ExtractionPolicy::Permissive).unwrap();
        assert_eq!(extracted.events[0].time, 1.0);
    }

    #[test]
    fn skill_rows_restart_combo_animation_without_scoring_or_judgment() {
        let forecast = simulate(
            &level(),
            &ForecastConfig {
                level_rating: Some(5.0),
                team_power: 3_000.0,
                ..Default::default()
            },
        )
        .unwrap();
        let skill = forecast
            .frames
            .iter()
            .find(|frame| frame.skill_activation)
            .unwrap();
        assert_eq!(skill.time, 1.5);
        assert_eq!(skill.combo, 2);
        assert_eq!(skill.score, 9_000.0);
        assert_eq!(skill.combo_event_time, Some(1.5));
        assert_eq!(skill.previous_combo_event_time, Some(1.0));
        assert_eq!(skill.judgment_time, Some(1.0));
        assert_eq!(skill.judgment.as_deref(), Some("PERFECT"));
    }

    #[test]
    fn talent_and_chart_skill_markers_change_score_not_note_timing_or_combo() {
        let source = level();
        let mut config = ForecastConfig {
            level_rating: Some(5.0),
            team_power: 3_000.0,
            ..Default::default()
        };
        let normal = simulate(&source, &config).unwrap();
        let note = normal.state_at(0.5 + 1.0e-9).unwrap();
        assert_eq!(note.score, 3_000.0);
        assert_eq!(note.combo, 1);
        assert_eq!(note.judgment.as_deref(), Some("PERFECT"));
        let later = normal.state_at(2.0 + 1.0e-9).unwrap();
        assert_eq!(later.combo, 3);
        assert_eq!(later.score, 15_000.0);

        config.team_power *= 2.0;
        let doubled_talent = simulate(&source, &config).unwrap();
        assert_eq!(
            doubled_talent.state_at(0.5 + 1.0e-9).unwrap().score,
            6_000.0
        );
        assert_eq!(
            doubled_talent.state_at(2.0 + 1.0e-9).unwrap().combo,
            later.combo,
            "talent must not change combo progression"
        );

        config.team_power = 3_000.0;
        config.chart_skills_enabled = false;
        let no_skills = simulate(&source, &config).unwrap();
        assert_eq!(no_skills.state_at(2.0 + 1.0e-9).unwrap().score, 12_000.0);
    }

    #[test]
    fn direct_seek_is_order_independent_and_matches_strict_reference_boundaries() {
        let forecast = simulate(
            &level(),
            &ForecastConfig {
                level_rating: Some(5.0),
                team_power: 3_000.0,
                ..Default::default()
            },
        )
        .unwrap();
        let exactly_first = forecast.state_at(0.5).unwrap();
        assert_eq!(
            exactly_first.combo, 0,
            "reference uses a strict event-time comparison"
        );
        let at_first = forecast.state_at(0.5 + 1.0e-9).unwrap().clone();
        let exactly_later = forecast.state_at(2.0).unwrap();
        assert_eq!(exactly_later.combo, 2);
        let at_later = forecast.state_at(2.0 + 1.0e-9).unwrap().clone();
        assert_eq!(forecast.state_at(2.0 + 1.0e-9), Some(&at_later));
        assert_eq!(forecast.state_at(0.5 + 1.0e-9), Some(&at_first));
        assert_eq!(forecast.state_at(0.499).unwrap().combo, 0);
        assert_eq!(at_later.combo, 3);
    }

    #[test]
    fn tournament_mode_and_standard_score_are_distinct_reference_paths() {
        let source = level();
        let mut config = ForecastConfig {
            level_rating: Some(5.0),
            team_power: 3_000.0,
            score_mode: ScoreMode::Tournament,
            ..Default::default()
        };
        let forecast = simulate(&source, &config).unwrap();
        assert_eq!(forecast.state_at(0.5 + 1.0e-9).unwrap().score, 3.0);
        assert_eq!(forecast.state_at(1.0 + 1.0e-9).unwrap().score, 6.0);
        config.score_mode = ScoreMode::Standard;
        assert_eq!(
            simulate(&source, &config)
                .unwrap()
                .state_at(1.0 + 1.0e-9)
                .unwrap()
                .score,
            9_000.0
        );
    }

    #[test]
    fn all_flick_mapping_changes_score_weight_but_not_the_rank_count() {
        let mut source = level();
        let note = &mut source.entities[1];
        note.archetype = Value::String("CriticalTapNote".into());
        let base = simulate(
            &source,
            &ForecastConfig {
                level_rating: Some(5.0),
                team_power: 4_000.0,
                ..Default::default()
            },
        )
        .unwrap();
        let all_flick = simulate(
            &source,
            &ForecastConfig {
                level_rating: Some(5.0),
                team_power: 4_000.0,
                all_flick: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(base.weighted_note_count, 5.0);
        assert_eq!(all_flick.weighted_note_count, 7.0);
        assert_eq!(all_flick.rank_weighted_note_count, 5.0);
        assert_eq!(base.state_at(0.5 + 1.0e-9).unwrap().score, 6_400.0);
        assert_eq!(all_flick.state_at(0.5 + 1.0e-9).unwrap().score, 6_857.0);
    }

    #[test]
    fn unknown_beat_archetypes_are_best_effort_and_strict_mode_remains_available() {
        let mut unsupported = level();
        unsupported
            .entities
            .push(entity("MysteryPlayableNote", Some(5.0)));
        unsupported
            .entities
            .push(entity("MysteryPlayableNote", Some(6.0)));
        let config = ForecastConfig {
            level_rating: Some(5.0),
            ..Default::default()
        };
        let forecast = simulate(&unsupported, &config).unwrap();
        assert!(forecast.potentially_incomplete());
        assert_eq!(forecast.supported_notes, 3);
        assert_eq!(forecast.state_at(2.0 + 1.0e-9).unwrap().combo, 3);
        assert_eq!(
            forecast.diagnostics.len(),
            1,
            "same unknown archetype is deduplicated"
        );
        assert_eq!(forecast.diagnostics[0].archetype, "MysteryPlayableNote");
        assert_eq!(forecast.diagnostics[0].entity_indices, vec![7, 8]);
        assert!(
            extract_events_with_policy(&unsupported, false, ExtractionPolicy::Strict)
                .unwrap_err()
                .to_string()
                .contains("not in Sono-Overlay's verified weight map")
        );
    }

    #[test]
    fn missing_rating_defaults_to_zero_and_override_beats_metadata() {
        let source = level();
        let defaulted = simulate(&source, &ForecastConfig::default()).unwrap();
        assert_eq!(defaulted.level_rating, 0.0);
        assert_eq!(defaulted.rating_source, RatingSource::DefaultZero);
        assert_eq!(defaulted.supported_notes, 3);

        let explicit_zero = simulate(
            &source,
            &ForecastConfig {
                level_rating: Some(0.0),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(explicit_zero.level_rating, 0.0);
        assert_eq!(explicit_zero.rating_source, RatingSource::UserOverride);

        let mut metadata_level = source.clone();
        metadata_level.metadata = Some(json!({"rating": {"level": 14.0}}));
        let metadata_rating = simulate(&metadata_level, &ForecastConfig::default()).unwrap();
        assert_eq!(metadata_rating.level_rating, 14.0);
        assert_eq!(metadata_rating.rating_source, RatingSource::LevelMetadata);

        let user_override = simulate(
            &metadata_level,
            &ForecastConfig {
                level_rating: Some(9.0),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(user_override.level_rating, 9.0);
        assert_eq!(user_override.rating_source, RatingSource::UserOverride);
    }

    #[test]
    fn real_bike_fixture_skips_camera_changes_and_keeps_recognized_note_timeline() {
        let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let level = crate::formats::load_level(
            &repository.join("TestingSuite/Next Sekai Engine/levels/bike/bike.json.gz"),
        )
        .expect("checked-in bike chart fixture");
        assert_eq!(level.entities[266].archetype.as_str(), Some("CameraChange"));
        let forecast = simulate(&level, &ForecastConfig::default())
            .expect("camera and stage changes do not disable recognized scoring");
        assert_eq!(forecast.level_rating, 0.0);
        assert!(forecast.supported_notes > 100);
        assert!(!forecast
            .events
            .iter()
            .any(|event| event.archetype == "CameraChange"));
        assert!(forecast.state_at(40.0).unwrap().combo > 0);
        assert!(forecast.potentially_incomplete());
        assert!(forecast.diagnostics.iter().any(|diagnostic| {
            diagnostic.archetype == "TransientHiddenDamageTickNote"
                && diagnostic.potentially_scoring
        }));
        assert!(!forecast
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.archetype == "CameraChange" }));
    }

    #[test]
    fn real_displayholic_fixture_generates_deterministic_all_perfect_note_events() {
        let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let level = crate::formats::load_level(&repository.join(
            "TestingSuite/Next Sekai Engine/levels/ABM - DISPLAYHOLIC/displayholic full.json.gz",
        ))
        .expect("checked-in DISPLAYHOLIC chart fixture");
        let forecast = simulate(&level, &ForecastConfig::default())
            .expect("recognized notes in DISPLAYHOLIC should be forecastable");
        assert!(forecast.supported_notes > 0);
        assert!(forecast
            .events
            .windows(2)
            .all(|pair| pair[0].time <= pair[1].time));
        assert!(forecast
            .events
            .iter()
            .filter(|event| !event.skill_activation)
            .all(|event| event.combo_contribution == 1 && event.weight > 0.0));
        for non_scoring in [
            "CameraChange",
            "StageMaskChange",
            "StagePivotChange",
            "StageStyleChange",
            "StageTransformChange",
            "#BPM_CHANGE",
            "#TIMESCALE_CHANGE",
        ] {
            assert!(
                forecast
                    .events
                    .iter()
                    .all(|event| event.archetype != non_scoring),
                "non-scoring entity {non_scoring} must not generate an AP event"
            );
        }
        let last_note = forecast
            .events
            .iter()
            .rev()
            .find(|event| !event.skill_activation)
            .expect("chart has scoring notes");
        let state = forecast.state_at(last_note.time + 1.0e-9).unwrap();
        assert_eq!(state.combo, forecast.supported_notes as i64);
        assert!(state.score > 0.0);
        assert_eq!(state.judgment.as_deref(), Some("PERFECT"));
    }

    #[test]
    fn real_next_sekai_chart_has_deterministic_note_progression() {
        let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let level = crate::formats::load_level(
            &repository.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz"),
        )
        .expect("checked-in Next Sekai chart fixture");
        let forecast = simulate(
            &level,
            &ForecastConfig {
                level_rating: Some(14.0),
                team_power: 250_000.0,
                ..Default::default()
            },
        )
        .expect("chart archetypes must be covered by the verified map");
        assert!(forecast.supported_notes > 100);
        assert!(forecast.weighted_note_count > 100.0);
        assert!(forecast
            .events
            .windows(2)
            .all(|pair| pair[0].time <= pair[1].time));
        let first = forecast
            .events
            .iter()
            .find(|event| !event.skill_activation)
            .unwrap();
        let before = forecast.state_at((first.time - 0.001).max(0.0)).unwrap();
        let at_boundary = forecast.state_at(first.time).unwrap();
        assert_eq!(at_boundary.combo, before.combo);
        let after = forecast.state_at(first.time + 1.0e-9).unwrap();
        assert!(after.combo > before.combo);
        assert!(after.score > before.score);
        assert_eq!(after.judgment.as_deref(), Some("PERFECT"));
        let later = forecast.state_at(first.time + 2.0).unwrap().clone();
        assert_eq!(forecast.state_at(first.time + 2.0), Some(&later));
    }

    #[test]
    fn baumkuchen_reference_video_score_and_combo_samples_match() {
        let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let level = crate::formats::load_level(
            &repository.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz"),
        )
        .expect("checked-in Baumkuchen chart fixture");
        // The reference clip does not include its original Sono-Overlay CLI
        // settings. These explicit values are the closest fit to its displayed
        // samples; they are not defaults and do not infer a universal rating.
        let forecast = simulate(
            &level,
            &ForecastConfig {
                team_power: 323_000.0,
                level_rating: Some(14.0),
                ..ForecastConfig::default()
            },
        )
        .expect("Baumkuchen scoring is covered by the verified map");

        // Values read from TestingSuite/ui_ref/baumkuchen x retry now
        // preview.mp4. The reference media timestamp has a small unknown
        // intro/offset component, so sample the corresponding chart states.
        let samples = [
            (2.64, 6, 11_374.0),
            (6.80, 33, 65_308.0),
            (8.80, 46, 89_523.0),
            (10.64, 56, 120_709.0),
        ];
        for (chart_time, expected_combo, expected_score) in samples {
            let state = forecast
                .state_at(chart_time)
                .expect("forecast contains an initial state");
            assert_eq!(
                state.combo, expected_combo,
                "combo at chart time {chart_time:.2}s"
            );
            assert!(
                (state.score - expected_score).abs() <= 15.0,
                "score at chart time {chart_time:.2}s: expected about {expected_score:.0}, got {:.0}",
                state.score
            );
            assert_eq!(state.judgment.as_deref(), Some("PERFECT"));
        }
    }
}
