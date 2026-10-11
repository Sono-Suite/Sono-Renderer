//! Optional renderer notifications. Events enter a bounded queue and are
//! delivered by a background worker so native services and network requests
//! never run on a render or export thread.

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    io::Write,
    process::{Command, Stdio},
    sync::{mpsc, mpsc::SyncSender, Mutex, OnceLock},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const QUEUE_CAPACITY: usize = 64;
const MAX_DISCORD_RETRIES: usize = 3;
const DIAGNOSTIC_CAPACITY: usize = 64;
const NOTIFICATION_CONFIG_FORMAT: &str = "sono-renderer-notification-config";
const NOTIFICATION_CONFIG_SCHEMA_VERSION: u32 = 1;

static DIAGNOSTICS: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();

fn record_diagnostic(message: String) {
    let diagnostics = DIAGNOSTICS.get_or_init(|| Mutex::new(VecDeque::new()));
    if let Ok(mut diagnostics) = diagnostics.lock() {
        if diagnostics.len() == DIAGNOSTIC_CAPACITY {
            diagnostics.pop_front();
        }
        diagnostics.push_back(message);
    }
}

/// Take delivery diagnostics accumulated by the background worker. The GUI
/// consumes these into its existing bounded export log.
pub fn take_diagnostics() -> Vec<String> {
    DIAGNOSTICS
        .get_or_init(|| Mutex::new(VecDeque::new()))
        .lock()
        .map(|mut diagnostics| diagnostics.drain(..).collect())
        .unwrap_or_default()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RendererEventKind {
    RenderStarted,
    RenderProgress,
    RenderCompleted,
    RenderFailed,
    JobQueued,
    JobStarted,
    JobCompiling,
    JobRendering,
    JobCompleted,
    JobFailed,
    JobCancelled,
}

impl RendererEventKind {
    pub const ALL: [Self; 11] = [
        Self::RenderStarted,
        Self::RenderProgress,
        Self::RenderCompleted,
        Self::RenderFailed,
        Self::JobQueued,
        Self::JobStarted,
        Self::JobCompiling,
        Self::JobRendering,
        Self::JobCompleted,
        Self::JobFailed,
        Self::JobCancelled,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::RenderStarted => "Render started",
            Self::RenderProgress => "Render progress",
            Self::RenderCompleted => "Render completed",
            Self::RenderFailed => "Render failed",
            Self::JobQueued => "Job queued",
            Self::JobStarted => "Job started",
            Self::JobCompiling => "Job compiling",
            Self::JobRendering => "Job rendering",
            Self::JobCompleted => "Job completed",
            Self::JobFailed => "Job failed",
            Self::JobCancelled => "Job cancelled",
        }
    }

    fn template_name(self) -> &'static str {
        match self {
            Self::RenderStarted => "render_started",
            Self::RenderProgress => "render_progress",
            Self::RenderCompleted => "render_completed",
            Self::RenderFailed => "render_failed",
            Self::JobQueued => "job_queued",
            Self::JobStarted => "job_started",
            Self::JobCompiling => "job_compiling",
            Self::JobRendering => "job_rendering",
            Self::JobCompleted => "job_completed",
            Self::JobFailed => "job_failed",
            Self::JobCancelled => "job_cancelled",
        }
    }
}

fn default_terminal_events() -> Vec<RendererEventKind> {
    vec![
        RendererEventKind::RenderCompleted,
        RendererEventKind::RenderFailed,
    ]
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProgressMode {
    #[default]
    Disabled,
    Milestones,
    Periodic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NotificationConfig {
    pub enabled: bool,
    pub native: NativeNotificationConfig,
    pub discord: DiscordNotificationConfig,
    pub progress_mode: ProgressMode,
    pub progress_interval_seconds: u32,
    pub progress_milestones: Vec<u8>,
    /// Non-secret warning retained when a malformed notification section is
    /// ignored while loading a render configuration.
    #[serde(skip)]
    pub configuration_warning: Option<String>,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            native: NativeNotificationConfig::default(),
            discord: DiscordNotificationConfig::default(),
            progress_mode: ProgressMode::Disabled,
            progress_interval_seconds: 60,
            progress_milestones: vec![25, 50, 75],
            configuration_warning: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NativeNotificationConfig {
    pub enabled: bool,
    pub events: Vec<RendererEventKind>,
    pub templates: EventTemplates,
}

impl Default for NativeNotificationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            events: default_terminal_events(),
            templates: EventTemplates::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DiscordNotificationConfig {
    pub enabled: bool,
    /// Optional informational bot ID. Sending is authorized by the bot token.
    pub bot_id: Option<String>,
    /// Name of the environment variable containing the bot token. The token
    /// itself is never placed in a config file, process argument, or log.
    pub bot_token_env: String,
    pub destinations: Vec<DiscordDestination>,
}

impl Default for DiscordNotificationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bot_id: None,
            bot_token_env: "SONO_RENDERER_DISCORD_BOT_TOKEN".into(),
            destinations: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DiscordDestination {
    pub enabled: bool,
    pub guild_id: Option<String>,
    /// Discord channel snowflake. This is the API route and authoritative ID.
    pub channel_id: String,
    pub events: Vec<RendererEventKind>,
    pub templates: EventTemplates,
}

impl Default for DiscordDestination {
    fn default() -> Self {
        Self {
            enabled: true,
            guild_id: None,
            channel_id: String::new(),
            events: default_terminal_events(),
            templates: EventTemplates::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EventTemplates {
    pub render_started: EventMessageTemplate,
    pub render_progress: EventMessageTemplate,
    pub render_completed: EventMessageTemplate,
    pub render_failed: EventMessageTemplate,
    pub job_queued: EventMessageTemplate,
    pub job_started: EventMessageTemplate,
    pub job_compiling: EventMessageTemplate,
    pub job_rendering: EventMessageTemplate,
    pub job_completed: EventMessageTemplate,
    pub job_failed: EventMessageTemplate,
    pub job_cancelled: EventMessageTemplate,
}

impl Default for EventTemplates {
    fn default() -> Self {
        Self {
            render_started: EventMessageTemplate::new(
                "{renderer} render started",
                "Rendering {chart} to {output_path} ({resolution} at {FPS} FPS).",
            ),
            render_progress: EventMessageTemplate::new(
                "{renderer} render progress",
                "Rendering {chart}: {progress}% ({completed_frames}/{frame_count} frames).",
            ),
            render_completed: EventMessageTemplate::new(
                "{renderer} render completed",
                "{chart} finished in {render_time}s ({frame_count} frames). Output: {output_path}",
            ),
            render_failed: EventMessageTemplate::new(
                "{renderer} render failed",
                "{chart}: {error}",
            ),
            job_queued: EventMessageTemplate::new(
                "{renderer} job queued",
                "Job {job_id} for {chart} was queued ({execution_backend}).",
            ),
            job_started: EventMessageTemplate::new(
                "{renderer} job started",
                "Job {job_id} attempt {attempt} is preparing {chart}.",
            ),
            job_compiling: EventMessageTemplate::new(
                "{renderer} job compiling",
                "Job {job_id} is compiling the Sono-GCC cache miss for {chart}.",
            ),
            job_rendering: EventMessageTemplate::new(
                "{renderer} job rendering",
                "Job {job_id} is rendering {chart} to {output_path}.",
            ),
            job_completed: EventMessageTemplate::new(
                "{renderer} job completed",
                "Job {job_id} completed {chart}. Output: {output_path}",
            ),
            job_failed: EventMessageTemplate::new(
                "{renderer} job failed",
                "Job {job_id} for {chart}: {error}",
            ),
            job_cancelled: EventMessageTemplate::new(
                "{renderer} job cancelled",
                "Job {job_id} for {chart} was cancelled.",
            ),
        }
    }
}

impl EventTemplates {
    fn get(&self, kind: RendererEventKind) -> &EventMessageTemplate {
        match kind {
            RendererEventKind::RenderStarted => &self.render_started,
            RendererEventKind::RenderProgress => &self.render_progress,
            RendererEventKind::RenderCompleted => &self.render_completed,
            RendererEventKind::RenderFailed => &self.render_failed,
            RendererEventKind::JobQueued => &self.job_queued,
            RendererEventKind::JobStarted => &self.job_started,
            RendererEventKind::JobCompiling => &self.job_compiling,
            RendererEventKind::JobRendering => &self.job_rendering,
            RendererEventKind::JobCompleted => &self.job_completed,
            RendererEventKind::JobFailed => &self.job_failed,
            RendererEventKind::JobCancelled => &self.job_cancelled,
        }
    }

    fn validate(&self, label: &str, discord: bool, diagnostics: &mut Vec<String>) {
        for kind in RendererEventKind::ALL {
            let template = self.get(kind);
            for (name, source) in [
                ("title", template.title.as_str()),
                ("body", template.body.as_str()),
            ] {
                if let Err(error) = validate_template(source) {
                    diagnostics.push(format!("{label} {} {name}: {error}", kind.template_name()));
                }
            }
            if discord {
                for (name, source) in [
                    ("content", template.content.as_str()),
                    ("embed title", template.embed_title.as_str()),
                    ("embed description", template.embed_description.as_str()),
                    ("embed footer", template.embed_footer.as_str()),
                ] {
                    if let Err(error) = validate_template(source) {
                        diagnostics
                            .push(format!("{label} {} {name}: {error}", kind.template_name()));
                    }
                }
                for field in &template.embed_fields {
                    for (name, source) in [
                        ("embed field name", &field.name),
                        ("embed field value", &field.value),
                    ] {
                        if let Err(error) = validate_template(source) {
                            diagnostics
                                .push(format!("{label} {} {name}: {error}", kind.template_name()));
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EventMessageTemplate {
    pub title: String,
    pub body: String,
    /// Discord message content; native notifications ignore this field.
    pub content: String,
    /// Optional Discord embed fields. An embed is omitted when all are empty.
    pub embed_title: String,
    pub embed_description: String,
    pub embed_footer: String,
    pub embed_fields: Vec<DiscordEmbedFieldTemplate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct DiscordEmbedFieldTemplate {
    pub name: String,
    pub value: String,
    pub inline: bool,
}

impl EventMessageTemplate {
    fn new(title: &str, body: &str) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            content: "{title}\n{body}".into(),
            embed_title: String::new(),
            embed_description: String::new(),
            embed_footer: String::new(),
            embed_fields: Vec::new(),
        }
    }
}

impl Default for EventMessageTemplate {
    fn default() -> Self {
        Self::new("{renderer}", "{chart}")
    }
}

impl NotificationConfig {
    pub fn has_active_route(&self) -> bool {
        self.enabled
            && ((self.native.enabled && !self.native.events.is_empty())
                || (self.discord.enabled
                    && self
                        .discord
                        .destinations
                        .iter()
                        .any(|destination| destination.enabled && !destination.events.is_empty())))
    }

    pub fn routes_event(&self, kind: RendererEventKind) -> bool {
        should_enqueue(self, kind)
    }

    /// Non-fatal configuration diagnostics. Notification problems never
    /// invalidate a RenderConfig or prevent rendering.
    pub fn diagnostics(&self) -> Vec<String> {
        let mut diagnostics = self
            .configuration_warning
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        self.native
            .templates
            .validate("native", false, &mut diagnostics);
        if self.discord.enabled {
            if self.discord.bot_token_env.trim().is_empty() {
                diagnostics.push("Discord bot token environment-variable name is empty".into());
            } else if !valid_environment_key(&self.discord.bot_token_env) {
                diagnostics.push("Discord bot credential setting must be an environment-variable name, not a token".into());
            } else if std::env::var(&self.discord.bot_token_env)
                .ok()
                .is_none_or(|token| token.trim().is_empty())
            {
                diagnostics.push(
                    "Discord is enabled but its bot credential is not available in the environment"
                        .into(),
                );
            }
            if let Some(id) = self.discord.bot_id.as_deref() {
                if !valid_snowflake(id) {
                    diagnostics.push("Discord bot ID must be a positive numeric snowflake".into());
                }
            }
            if self.discord.destinations.is_empty() {
                diagnostics.push("Discord is enabled but has no destinations".into());
            }
            for (index, destination) in self.discord.destinations.iter().enumerate() {
                let label = format!("Discord destination {}", index + 1);
                if destination.channel_id.trim().is_empty()
                    || !valid_snowflake(destination.channel_id.trim())
                {
                    diagnostics.push(format!(
                        "{label} channel ID must be a positive numeric snowflake"
                    ));
                }
                if destination
                    .guild_id
                    .as_deref()
                    .is_some_and(|id| !valid_snowflake(id.trim()))
                {
                    diagnostics.push(format!(
                        "{label} guild ID must be a positive numeric snowflake"
                    ));
                }
                destination
                    .templates
                    .validate(&label, true, &mut diagnostics);
            }
        }
        if self.progress_interval_seconds == 0 {
            diagnostics
                .push("Progress notification interval must be greater than zero seconds".into());
        }
        if self
            .progress_milestones
            .iter()
            .any(|milestone| *milestone == 0 || *milestone > 100)
        {
            diagnostics.push("Progress milestones must be in 1..=100".into());
        }
        diagnostics
    }

    /// Parse the notifications section without allowing a notification-only
    /// schema error to invalidate the rest of a renderer config.
    pub fn from_json_or_disabled(bytes: &[u8]) -> Self {
        let parsed = (|| {
            let mut document: serde_json::Value = serde_json::from_slice(bytes)?;
            crate::render::strip_optional_flat_schema(
                &mut document,
                NOTIFICATION_CONFIG_FORMAT,
                NOTIFICATION_CONFIG_SCHEMA_VERSION,
                "notification configuration",
            )?;
            Ok::<Self, anyhow::Error>(serde_json::from_value(document)?)
        })();
        match parsed {
            Ok(config) => config,
            Err(_) => Self::disabled_with_warning(
                "notification settings could not be parsed and were disabled",
            ),
        }
    }

    pub fn disabled_with_warning(warning: impl Into<String>) -> Self {
        let mut config = Self::default();
        config.enabled = false;
        config.configuration_warning = Some(warning.into());
        config
    }

    pub fn progress_due(
        &self,
        percent: u8,
        elapsed: Duration,
        last_percent: u8,
        last_time: Duration,
    ) -> bool {
        if !self.enabled || percent > 100 {
            return false;
        }
        match self.progress_mode {
            ProgressMode::Disabled => false,
            ProgressMode::Milestones => self
                .progress_milestones
                .iter()
                .any(|milestone| last_percent < *milestone && percent >= *milestone),
            ProgressMode::Periodic => {
                elapsed.saturating_sub(last_time)
                    >= Duration::from_secs(u64::from(self.progress_interval_seconds.max(1)))
            }
        }
    }
}

fn valid_snowflake(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|id| id > 0)
}

fn valid_environment_key(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

#[derive(Debug, Clone)]
pub struct RendererEvent {
    pub kind: RendererEventKind,
    pub values: BTreeMap<String, String>,
}

impl RendererEvent {
    pub fn new(kind: RendererEventKind) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string();
        let mut values = BTreeMap::new();
        values.insert("renderer".into(), "Sono-Renderer".into());
        values.insert("renderer_version".into(), env!("CARGO_PKG_VERSION").into());
        values.insert("timestamp".into(), timestamp);
        Self { kind, values }
    }

    pub fn with(mut self, key: impl Into<String>, value: impl ToString) -> Self {
        self.values.insert(key.into(), value.to_string());
        self
    }
}

struct Delivery {
    config: NotificationConfig,
    event: RendererEvent,
}

enum QueueMessage {
    Event(Delivery),
    Flush(mpsc::Sender<()>),
}

static QUEUE: OnceLock<Option<SyncSender<QueueMessage>>> = OnceLock::new();

pub fn deserialize_tolerant<'de, D>(deserializer: D) -> Result<NotificationConfig, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match serde_json::from_value::<NotificationConfig>(value) {
        Ok(config) => config,
        Err(_) => NotificationConfig::disabled_with_warning(
            "notification settings could not be parsed and were disabled",
        ),
    })
}

fn sender() -> Option<&'static SyncSender<QueueMessage>> {
    QUEUE
        .get_or_init(|| {
            let (tx, rx) = mpsc::sync_channel::<QueueMessage>(QUEUE_CAPACITY);
            thread::Builder::new()
                .name("sono-notifications".into())
                .spawn(move || notification_worker(rx))
                .map(|_| tx)
                .map_err(|_| ())
                .ok()
        })
        .as_ref()
}

/// Non-blocking event submission. A full queue rejects the newest auxiliary
/// event; rendering is never back-pressured.
pub fn emit(config: &NotificationConfig, event: RendererEvent) {
    if !should_enqueue(config, event.kind) {
        return;
    }
    let Some(sender) = sender() else { return };
    // Queue pressure is deliberately silent here: even writing a diagnostic
    // can block on a pipe, and this function runs on renderer threads.
    let _ = try_enqueue(sender, config, event);
}

fn should_enqueue(config: &NotificationConfig, kind: RendererEventKind) -> bool {
    config.enabled && has_route(config, kind)
}

fn try_enqueue(
    sender: &SyncSender<QueueMessage>,
    config: &NotificationConfig,
    event: RendererEvent,
) -> bool {
    sender
        .try_send(QueueMessage::Event(Delivery {
            config: config.clone(),
            event,
        }))
        .is_ok()
}

/// Wait for the worker to process previously submitted events. The CLI uses
/// this after an export has completed; renderer and GUI calls never wait.
pub fn flush(timeout: Duration) -> bool {
    let Some(sender) = sender() else { return false };
    let deadline = Instant::now() + timeout;
    let (ack_tx, ack_rx) = mpsc::channel();
    loop {
        match sender.try_send(QueueMessage::Flush(ack_tx.clone())) {
            Ok(()) => break,
            Err(mpsc::TrySendError::Disconnected(_)) => return false,
            Err(mpsc::TrySendError::Full(_)) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(mpsc::TrySendError::Full(_)) => return false,
        }
    }
    ack_rx
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .is_ok()
}

fn has_route(config: &NotificationConfig, kind: RendererEventKind) -> bool {
    (config.native.enabled && config.native.events.contains(&kind))
        || (config.discord.enabled
            && config
                .discord
                .destinations
                .iter()
                .any(|destination| destination.enabled && destination.events.contains(&kind)))
}

fn notification_worker(rx: mpsc::Receiver<QueueMessage>) {
    let mut last_discord_delivery = HashMap::<String, Instant>::new();
    while let Ok(message) = rx.recv() {
        match message {
            QueueMessage::Event(delivery) => {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    deliver(&delivery, &mut last_discord_delivery)
                }));
                if result.is_err() {
                    record_diagnostic(
                        "notification backend panicked; renderer remains unaffected".into(),
                    );
                }
            }
            QueueMessage::Flush(ack) => {
                let _ = ack.send(());
            }
        }
    }
}

fn deliver(delivery: &Delivery, last_discord_delivery: &mut HashMap<String, Instant>) {
    let config = &delivery.config;
    let event = &delivery.event;
    if !config.enabled {
        return;
    }
    if config.native.enabled && config.native.events.contains(&event.kind) {
        if let Err(error) = safe_deliver(&NativeBackend, config, None, event) {
            record_diagnostic(format!("native notification failed: {error}"));
        }
    }
    if config.discord.enabled {
        for destination in &config.discord.destinations {
            if destination.enabled && destination.events.contains(&event.kind) {
                if let Some(last) = last_discord_delivery.get(&destination.channel_id) {
                    let wait = discord_rate_limit_delay(*last, Instant::now());
                    if !wait.is_zero() {
                        thread::sleep(wait);
                    }
                }
                last_discord_delivery.insert(destination.channel_id.clone(), Instant::now());
                if let Err(error) = safe_deliver(&DiscordBackend, config, Some(destination), event)
                {
                    record_diagnostic(format!(
                        "Discord notification failed for channel {}: {error}",
                        destination.channel_id
                    ));
                }
            }
        }
    }
}

fn discord_rate_limit_delay(last_sent: Instant, now: Instant) -> Duration {
    Duration::from_secs(1).saturating_sub(now.saturating_duration_since(last_sent))
}

trait NotificationBackend {
    fn send(
        &self,
        config: &NotificationConfig,
        destination: Option<&DiscordDestination>,
        event: &RendererEvent,
    ) -> Result<(), String>;
}

struct NativeBackend;
impl NotificationBackend for NativeBackend {
    fn send(
        &self,
        config: &NotificationConfig,
        _: Option<&DiscordDestination>,
        event: &RendererEvent,
    ) -> Result<(), String> {
        let template = config.native.templates.get(event.kind);
        let title = render_template(&template.title, &event.values)?;
        let mut values = event.values.clone();
        values.insert("title".into(), title.clone());
        let body = render_template(&template.body, &values)?;
        platform_native_notification(&title, &body)
    }
}

struct DiscordBackend;
impl NotificationBackend for DiscordBackend {
    fn send(
        &self,
        config: &NotificationConfig,
        destination: Option<&DiscordDestination>,
        event: &RendererEvent,
    ) -> Result<(), String> {
        let settings = &config.discord;
        let destination = destination.ok_or_else(|| "Discord destination is missing".to_owned())?;
        if !valid_snowflake(destination.channel_id.trim()) {
            return Err("channel ID is not a valid numeric snowflake".into());
        }
        if !valid_environment_key(&settings.bot_token_env) {
            return Err("bot credential setting is not an environment-variable name".into());
        }
        let token = std::env::var(&settings.bot_token_env)
            .map_err(|_| "bot credential environment variable is unavailable".to_owned())?;
        if token.trim().is_empty() || token.contains(['\r', '\n']) {
            return Err("bot credential environment variable is empty or malformed".into());
        }
        let payload = build_discord_payload(settings, destination, event)?;
        discord_post(&token, &destination.channel_id, &payload)
    }
}

fn build_discord_payload(
    settings: &DiscordNotificationConfig,
    destination: &DiscordDestination,
    event: &RendererEvent,
) -> Result<String, String> {
    let template = destination.templates.get(event.kind);
    let mut values = event.values.clone();
    values.insert(
        "guild_id".into(),
        destination.guild_id.clone().unwrap_or_default(),
    );
    values.insert("bot_id".into(), settings.bot_id.clone().unwrap_or_default());
    let title = render_template(&template.title, &values)?;
    let body = render_template(&template.body, &values)?;
    values.insert("title".into(), title);
    values.insert("body".into(), body);
    let content = render_template(&template.content, &values)?;
    let embed_title = render_template(&template.embed_title, &values)?;
    let embed_description = render_template(&template.embed_description, &values)?;
    let embed_footer = render_template(&template.embed_footer, &values)?;
    let mut payload = serde_json::Map::new();
    if !content.is_empty() {
        payload.insert(
            "content".into(),
            serde_json::Value::String(limit_chars(&content, 2000)),
        );
    }
    let mut embed = serde_json::Map::new();
    if !embed_title.is_empty() {
        embed.insert(
            "title".into(),
            serde_json::Value::String(limit_chars(&embed_title, 256)),
        );
    }
    if !embed_description.is_empty() {
        embed.insert(
            "description".into(),
            serde_json::Value::String(limit_chars(&embed_description, 4096)),
        );
    }
    if !embed_footer.is_empty() {
        embed.insert(
            "footer".into(),
            serde_json::json!({"text": limit_chars(&embed_footer, 2048)}),
        );
    }
    let fields = template
        .embed_fields
        .iter()
        .filter_map(|field| {
            let name = render_template(&field.name, &values).ok()?;
            let value = render_template(&field.value, &values).ok()?;
            if name.is_empty() || value.is_empty() {
                return None;
            }
            Some(serde_json::json!({
                "name": limit_chars(&name, 256),
                "value": limit_chars(&value, 1024),
                "inline": field.inline
            }))
        })
        .take(25)
        .collect::<Vec<_>>();
    if !fields.is_empty() {
        embed.insert("fields".into(), serde_json::Value::Array(fields));
    }
    let embed_text_length = embed
        .iter()
        .map(|(key, value)| match key.as_str() {
            "footer" => value["text"].as_str().unwrap_or_default().chars().count(),
            "fields" => value
                .as_array()
                .into_iter()
                .flatten()
                .map(|field| {
                    field["name"].as_str().unwrap_or_default().chars().count()
                        + field["value"].as_str().unwrap_or_default().chars().count()
                })
                .sum(),
            _ => value.as_str().unwrap_or_default().chars().count(),
        })
        .sum::<usize>();
    if embed_text_length > 6000 {
        return Err("Discord embed templates exceed the 6000-character aggregate limit".into());
    }
    if !embed.is_empty() {
        payload.insert(
            "embeds".into(),
            serde_json::Value::Array(vec![serde_json::Value::Object(embed)]),
        );
    }
    if payload.is_empty() {
        return Err("message template produced no content or embed".into());
    }
    payload.insert("allowed_mentions".into(), serde_json::json!({"parse": []}));
    Ok(serde_json::Value::Object(payload).to_string())
}

fn deliver_via<B: NotificationBackend>(
    backend: &B,
    config: &NotificationConfig,
    destination: Option<&DiscordDestination>,
    event: &RendererEvent,
) -> Result<(), String> {
    backend.send(config, destination, event)
}

fn safe_deliver<B: NotificationBackend>(
    backend: &B,
    config: &NotificationConfig,
    destination: Option<&DiscordDestination>,
    event: &RendererEvent,
) -> Result<(), String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        deliver_via(backend, config, destination, event)
    })) {
        Ok(result) => result,
        Err(_) => Err("backend panicked; delivery was isolated".into()),
    }
}

fn limit_chars(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

fn discord_post(token: &str, channel_id: &str, payload: &str) -> Result<(), String> {
    let url = format!("https://discord.com/api/v10/channels/{channel_id}/messages");
    for attempt in 0..=MAX_DISCORD_RETRIES {
        let output = curl_post(token, &url, payload)?;
        let marker = b"\n__SONO_HTTP_STATUS__";
        let split = output
            .windows(marker.len())
            .rposition(|window| window == marker)
            .ok_or_else(|| "HTTP response did not include a status code".to_owned())?;
        let body = String::from_utf8_lossy(&output[..split]);
        let status = String::from_utf8_lossy(&output[split + marker.len()..])
            .trim()
            .parse::<u16>()
            .map_err(|_| "HTTP response status was not valid".to_owned())?;
        if (200..300).contains(&status) {
            return Ok(());
        }
        if attempt < MAX_DISCORD_RETRIES {
            if let Some(delay) = discord_retry_delay(status, &body, attempt) {
                thread::sleep(delay);
                continue;
            }
        }
        return Err(format!("Discord API returned HTTP {status}"));
    }
    Err("Discord API retry limit reached".into())
}

fn discord_retry_delay(status: u16, body: &str, attempt: usize) -> Option<Duration> {
    if status == 429 {
        let seconds = serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|json| json.get("retry_after").and_then(serde_json::Value::as_f64))
            .unwrap_or(1.0)
            .clamp(0.25, 10.0);
        Some(Duration::from_secs_f64(seconds))
    } else if (500..600).contains(&status) {
        Some(Duration::from_millis(250 * (1 << attempt.min(5))))
    } else {
        None
    }
}

fn curl_post(token: &str, url: &str, payload: &str) -> Result<Vec<u8>, String> {
    let config = format!(
        "url = \"{}\"\nrequest = \"POST\"\nheader = \"Authorization: Bot {}\"\nheader = \"Content-Type: application/json\"\ndata = \"{}\"\nwrite-out = \"\\n__SONO_HTTP_STATUS__%{{http_code}}\"\nmax-time = \"8\"\nconnect-timeout = \"3\"\n",
        curl_config_escape(url),
        curl_config_escape(token),
        curl_config_escape(payload)
    );
    let mut child = Command::new("curl")
        .args(["--silent", "--show-error", "--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "curl is unavailable; Discord delivery skipped".to_owned())?;
    child
        .stdin
        .take()
        .ok_or_else(|| "could not open the Discord request pipe".to_owned())?
        .write_all(config.as_bytes())
        .map_err(|_| "could not write the Discord request".to_owned())?;
    let output = child
        .wait_with_output()
        .map_err(|_| "could not collect the Discord response".to_owned())?;
    if !output.status.success() {
        return Err("Discord network request failed".into());
    }
    Ok(output.stdout)
}

fn curl_config_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\r', "\\r")
        .replace('\n', "\\n")
}

fn run_native_command(mut command: Command) -> Result<(), String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "native notification command is unavailable".to_owned())?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => return Err("native notification service returned an error".into()),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("native notification timed out".into());
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("native notification process failed".into());
            }
        }
    }
}

#[cfg(windows)]
fn platform_native_notification(title: &str, body: &str) -> Result<(), String> {
    let xml = format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
        xml_escape(title),
        xml_escape(body)
    );
    let ps_quote = |value: &str| format!("'{}'", value.replace('\'', "''"));
    let script = format!(
        "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null; [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] > $null; $doc = New-Object Windows.Data.Xml.Dom.XmlDocument; $doc.LoadXml({}); $toast = [Windows.UI.Notifications.ToastNotification]::new($doc); [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('Sono-Renderer').Show($toast)",
        ps_quote(&xml)
    );
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-WindowStyle",
        "Hidden",
        "-Command",
        &script,
    ]);
    run_native_command(command)
}

#[cfg(target_os = "linux")]
fn platform_native_notification(title: &str, body: &str) -> Result<(), String> {
    let mut command = Command::new("notify-send");
    command.arg(title).arg(body);
    run_native_command(command)
}

#[cfg(target_os = "macos")]
fn platform_native_notification(title: &str, body: &str) -> Result<(), String> {
    let escape = |value: &str| value.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        escape(body),
        escape(title)
    );
    let mut command = Command::new("osascript");
    command.args(["-e", &script]);
    run_native_command(command)
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn platform_native_notification(_: &str, _: &str) -> Result<(), String> {
    Err("native notifications are not implemented for this operating system".into())
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

const TEMPLATE_KEYS: &[&str] = &[
    "renderer",
    "renderer_version",
    "engine",
    "chart",
    "output",
    "output_path",
    "execution_mode",
    "compilation_time",
    "render_time",
    "total_time",
    "duration",
    "resolution",
    "width",
    "height",
    "FPS",
    "frame_count",
    "completed_frames",
    "cache_status",
    "warning",
    "error",
    "job_id",
    "job_state",
    "attempt",
    "execution_backend",
    "timestamp",
    "progress",
    "title",
    "body",
    "guild_id",
    "bot_id",
];

pub fn validate_template(template: &str) -> Result<(), String> {
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
            }
            '{' => {
                let mut name = String::new();
                let mut closed = false;
                for next in chars.by_ref() {
                    if next == '}' {
                        closed = true;
                        break;
                    }
                    name.push(next);
                }
                if !closed {
                    return Err("unclosed '{' placeholder".into());
                }
                if !TEMPLATE_KEYS.contains(&name.as_str()) {
                    return Err(format!("unknown placeholder '{{{name}}}'"));
                }
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
            }
            '}' => return Err("unmatched '}' brace".into()),
            _ => {}
        }
    }
    Ok(())
}

pub fn render_template(
    template: &str,
    values: &BTreeMap<String, String>,
) -> Result<String, String> {
    validate_template(template)?;
    let mut output = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                output.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                output.push('}');
            }
            '{' => {
                let mut name = String::new();
                for next in chars.by_ref() {
                    if next == '}' {
                        break;
                    }
                    name.push(next);
                }
                output.push_str(values.get(&name).map(String::as_str).unwrap_or(""));
            }
            _ => output.push(ch),
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct FakeBackend {
        calls: Arc<AtomicUsize>,
        fail: bool,
        panic: bool,
    }

    impl NotificationBackend for FakeBackend {
        fn send(
            &self,
            _: &NotificationConfig,
            _: Option<&DiscordDestination>,
            _: &RendererEvent,
        ) -> Result<(), String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.panic {
                panic!("fake backend panic");
            }
            if self.fail {
                Err("fake delivery failure".into())
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn defaults_route_only_major_completion_events() {
        let config = NotificationConfig::default();
        assert!(config.enabled);
        assert!(config.native.enabled);
        assert!(!config.discord.enabled);
        assert!(config
            .native
            .events
            .contains(&RendererEventKind::RenderCompleted));
        assert!(config
            .native
            .events
            .contains(&RendererEventKind::RenderFailed));
        assert!(!config
            .native
            .events
            .contains(&RendererEventKind::RenderProgress));
        assert!(!config
            .diagnostics()
            .iter()
            .any(|entry| entry.contains("template")));
    }

    #[test]
    fn standalone_notification_config_rejects_future_schema_without_enabling_delivery() {
        let config = NotificationConfig::from_json_or_disabled(
            br#"{"format":"sono-renderer-notification-config","schema_version":2,"enabled":true}"#,
        );
        assert!(!config.enabled);
        assert!(config.configuration_warning.is_some());
    }

    #[test]
    fn job_lifecycle_kinds_can_be_filtered_and_render_job_context() {
        let mut config = NotificationConfig::default();
        config.native.events = vec![
            RendererEventKind::JobQueued,
            RendererEventKind::JobCompleted,
        ];
        assert!(config.routes_event(RendererEventKind::JobQueued));
        assert!(config.routes_event(RendererEventKind::JobCompleted));
        assert!(!config.routes_event(RendererEventKind::JobRendering));
        assert!(config.diagnostics().is_empty());

        let event = RendererEvent::new(RendererEventKind::JobQueued)
            .with("job_id", "job-42")
            .with("chart", "fixture-chart")
            .with("execution_backend", "CPU; Sono VM");
        let rendered =
            render_template(&config.native.templates.job_queued.body, &event.values).unwrap();
        assert!(rendered.contains("job-42"));
        assert!(rendered.contains("fixture-chart"));
        let calls = Arc::new(AtomicUsize::new(0));
        let backend = FakeBackend {
            calls: calls.clone(),
            fail: false,
            panic: false,
        };
        deliver_via(&backend, &config, None, &event).unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn event_filters_are_independent_for_each_backend_and_destination() {
        let mut config = NotificationConfig::default();
        config.native.enabled = false;
        config.discord.enabled = true;
        let a = DiscordDestination {
            channel_id: "123456789012345678".into(),
            events: vec![RendererEventKind::RenderFailed],
            ..Default::default()
        };
        let b = DiscordDestination {
            channel_id: "123456789012345679".into(),
            events: vec![RendererEventKind::RenderCompleted],
            ..Default::default()
        };
        config.discord.destinations = vec![a, b];
        assert!(has_route(&config, RendererEventKind::RenderFailed));
        assert!(has_route(&config, RendererEventKind::RenderCompleted));
        assert!(!has_route(&config, RendererEventKind::RenderStarted));
        config.enabled = false;
        assert!(!should_enqueue(&config, RendererEventKind::RenderFailed));
    }

    #[test]
    fn native_discord_both_neither_and_global_off_routing_modes_work() {
        let mut config = NotificationConfig::default();
        assert!(should_enqueue(&config, RendererEventKind::RenderCompleted));
        config.discord.enabled = true;
        config.discord.destinations.push(DiscordDestination {
            channel_id: "123456789012345678".into(),
            ..Default::default()
        });
        assert!(should_enqueue(&config, RendererEventKind::RenderCompleted));
        config.native.enabled = false;
        assert!(should_enqueue(&config, RendererEventKind::RenderCompleted));
        config.discord.enabled = false;
        assert!(!should_enqueue(&config, RendererEventKind::RenderCompleted));
        config.enabled = false;
        config.native.enabled = true;
        assert!(!should_enqueue(&config, RendererEventKind::RenderCompleted));
    }

    #[test]
    fn templates_use_actual_values_escape_braces_and_tolerate_missing_values() {
        let mut values = BTreeMap::new();
        values.insert("chart".into(), "demo".into());
        assert_eq!(
            render_template("{{{chart}}}:{error}", &values).unwrap(),
            "{demo}:"
        );
        assert!(validate_template("{unknown}").is_err());
        assert!(validate_template("unfinished {chart").is_err());
    }

    #[test]
    fn discord_custom_content_and_embeds_use_event_values_without_external_services() {
        let settings = DiscordNotificationConfig {
            bot_id: Some("123456789012345678".into()),
            ..Default::default()
        };
        let mut destination = DiscordDestination {
            guild_id: Some("123456789012345679".into()),
            channel_id: "123456789012345680".into(),
            ..Default::default()
        };
        let template = &mut destination.templates.render_completed;
        template.title = "Finished {chart}".into();
        template.body = "{frame_count} frames".into();
        template.content = "{title}: {body}".into();
        template.embed_title = "{bot_id} / {guild_id}".into();
        template.embed_description = "{resolution}".into();
        template.embed_footer = "{timestamp}".into();
        template.embed_fields.push(DiscordEmbedFieldTemplate {
            name: "Frames".into(),
            value: "{frame_count}".into(),
            inline: true,
        });
        let event = RendererEvent::new(RendererEventKind::RenderCompleted)
            .with("chart", "song \"name\"")
            .with("frame_count", 240)
            .with("resolution", "1920x1080");
        let payload: serde_json::Value =
            serde_json::from_str(&build_discord_payload(&settings, &destination, &event).unwrap())
                .unwrap();
        assert_eq!(payload["content"], "Finished song \"name\": 240 frames");
        assert_eq!(
            payload["embeds"][0]["title"],
            "123456789012345678 / 123456789012345679"
        );
        assert_eq!(payload["embeds"][0]["description"], "1920x1080");
        assert!(payload["embeds"][0]["footer"]["text"].is_string());
        assert_eq!(payload["embeds"][0]["fields"][0]["value"], "240");
        assert_eq!(payload["embeds"][0]["fields"][0]["inline"], true);
        assert_eq!(payload["allowed_mentions"]["parse"], serde_json::json!([]));
    }

    #[test]
    fn destination_ids_and_template_settings_are_diagnosed_without_panicking() {
        let mut config = NotificationConfig::default();
        config.discord.enabled = true;
        config.discord.destinations.push(DiscordDestination {
            channel_id: "channel-name".into(),
            ..Default::default()
        });
        config.discord.destinations[0].templates.render_failed.body = "{bogus}".into();
        let issues = config.diagnostics();
        assert!(issues.iter().any(|issue| issue.contains("channel ID")));
        assert!(issues
            .iter()
            .any(|issue| issue.contains("unknown placeholder")));
        assert!(!issues
            .iter()
            .any(|issue| issue.contains("TOKEN") || issue.contains("credential=")));
    }

    #[test]
    fn progress_modes_are_bounded_and_do_not_emit_every_frame_by_default() {
        let mut config = NotificationConfig::default();
        assert!(!config.progress_due(50, Duration::from_secs(30), 0, Duration::ZERO));
        config.progress_mode = ProgressMode::Milestones;
        assert!(!config.progress_due(24, Duration::from_secs(30), 0, Duration::ZERO));
        assert!(config.progress_due(26, Duration::from_secs(30), 0, Duration::ZERO));
        assert!(!config.progress_due(26, Duration::from_secs(31), 25, Duration::ZERO));
        config.progress_mode = ProgressMode::Periodic;
        assert!(!config.progress_due(50, Duration::from_secs(30), 0, Duration::ZERO));
        assert!(config.progress_due(50, Duration::from_secs(60), 49, Duration::ZERO));
    }

    #[test]
    fn snowflake_and_safe_formatting_helpers_accept_boundary_values() {
        assert!(valid_snowflake("1"));
        assert!(!valid_snowflake("0"));
        assert!(!valid_snowflake("18446744073709551616"));
        assert_eq!(limit_chars("ééé", 2), "éé");
        assert!(curl_config_escape("a\"b\\c\n").contains("\\\""));
    }

    #[test]
    fn bounded_queue_submission_never_waits_for_a_slow_backend() {
        let (sender, _receiver) = mpsc::sync_channel(1);
        let config = NotificationConfig::default();
        assert!(try_enqueue(
            &sender,
            &config,
            RendererEvent::new(RendererEventKind::RenderCompleted)
        ));
        assert!(!try_enqueue(
            &sender,
            &config,
            RendererEvent::new(RendererEventKind::RenderFailed)
        ));
    }

    #[test]
    fn backend_failures_and_panics_are_isolated_with_a_fake_backend() {
        let config = NotificationConfig::default();
        let event = RendererEvent::new(RendererEventKind::RenderCompleted);
        let failed_calls = Arc::new(AtomicUsize::new(0));
        let failed = FakeBackend {
            calls: failed_calls.clone(),
            fail: true,
            panic: false,
        };
        assert!(safe_deliver(&failed, &config, None, &event).is_err());
        let panic_calls = Arc::new(AtomicUsize::new(0));
        let panicking = FakeBackend {
            calls: panic_calls.clone(),
            fail: false,
            panic: true,
        };
        assert!(safe_deliver(&panicking, &config, None, &event)
            .unwrap_err()
            .contains("isolated"));
        let working_calls = Arc::new(AtomicUsize::new(0));
        let working = FakeBackend {
            calls: working_calls.clone(),
            fail: false,
            panic: false,
        };
        assert!(safe_deliver(&working, &config, None, &event).is_ok());
        assert_eq!(failed_calls.load(Ordering::Relaxed), 1);
        assert_eq!(panic_calls.load(Ordering::Relaxed), 1);
        assert_eq!(working_calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn unavailable_discord_credentials_fail_as_diagnostic_only() {
        let mut config = NotificationConfig::default();
        config.discord.enabled = true;
        config.discord.bot_token_env = format!(
            "SONO_RENDERER_TEST_UNSET_CREDENTIAL_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let destination = DiscordDestination {
            channel_id: "123456789012345678".into(),
            ..Default::default()
        };
        let error = safe_deliver(
            &DiscordBackend,
            &config,
            Some(&destination),
            &RendererEvent::new(RendererEventKind::RenderCompleted),
        )
        .unwrap_err();
        assert!(error.contains("credential environment variable is unavailable"));
        assert!(!error.contains(&config.discord.bot_token_env));
    }

    #[test]
    fn credential_setting_rejects_what_looks_like_a_raw_token_without_echoing_it() {
        let mut config = NotificationConfig::default();
        config.discord.enabled = true;
        config.discord.bot_token_env = "MTAxMjM0NTY3ODkw.Ghijkl.very-secret-token".into();
        let diagnostics = config.diagnostics().join("\n");
        assert!(diagnostics.contains("environment-variable name, not a token"));
        assert!(!diagnostics.contains("very-secret-token"));
    }

    #[test]
    fn unavailable_native_command_is_reported_without_a_desktop_service() {
        let command = Command::new(format!(
            "sono-renderer-missing-notifier-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let error = run_native_command(command).unwrap_err();
        assert!(error.contains("unavailable"));
    }

    #[test]
    fn discord_rate_limit_and_retry_backoff_are_bounded() {
        assert_eq!(
            discord_retry_delay(429, r#"{"retry_after":0.5}"#, 0),
            Some(Duration::from_millis(500))
        );
        assert_eq!(
            discord_retry_delay(429, r#"{"retry_after":300}"#, 0),
            Some(Duration::from_secs(10))
        );
        assert_eq!(
            discord_retry_delay(503, "", 2),
            Some(Duration::from_secs(1))
        );
        assert_eq!(discord_retry_delay(400, "", 0), None);
        let last = Instant::now();
        assert!(
            discord_rate_limit_delay(last, last + Duration::from_millis(200))
                >= Duration::from_millis(799)
        );
        assert_eq!(
            discord_rate_limit_delay(last, last + Duration::from_secs(2)),
            Duration::ZERO
        );
    }

    #[test]
    fn malformed_notification_json_disables_notifications_without_rejecting_renderer_config() {
        let malformed = NotificationConfig::from_json_or_disabled(b"{not json");
        assert!(!malformed.enabled);
        assert!(malformed
            .diagnostics()
            .iter()
            .any(|entry| entry.contains("disabled")));
        let config: crate::render::RenderConfig = serde_json::from_value(serde_json::json!({
            "engine": "e", "resources": "r", "level": "l",
            "notifications": { "progress_mode": "made_up" }
        }))
        .unwrap();
        assert!(!config.notifications.enabled);
        assert!(config
            .notifications
            .diagnostics()
            .iter()
            .any(|entry| entry.contains("disabled")));
    }
}
