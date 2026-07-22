use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    WorldEntered,
    WorldExited,
    LocalPlayerIdentified,
    PlayerJoined,
    PlayerLeft,
    SessionLoaded,
    SessionSaved,
    StageEntered,
    BossStarted,
    Intermission,
    Lobby,
    DamageDealt,
    DamageTaken,
    BossDefeated,
    BossDamageSummary,
    EnemySpawned,
    EnemyRetired,
    EnemyNamed,
    EnemyDiagnostic,
    OwnershipTransferred,
    StageEventAdvanced,
    StageProgressAdvanced,
    SpawnToken,
    Healing,
    BuffApplied,
    BuffRemoved,
    DebuffApplied,
    DebuffRemoved,
    PlayerDowned,
    Loot,
    GameMessage,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WorldEntered => "world_entered",
            Self::WorldExited => "world_exited",
            Self::LocalPlayerIdentified => "local_player_identified",
            Self::PlayerJoined => "player_joined",
            Self::PlayerLeft => "player_left",
            Self::SessionLoaded => "session_loaded",
            Self::SessionSaved => "session_saved",
            Self::StageEntered => "stage_entered",
            Self::BossStarted => "boss_started",
            Self::Intermission => "intermission",
            Self::Lobby => "lobby",
            Self::DamageDealt => "damage_dealt",
            Self::DamageTaken => "damage_taken",
            Self::BossDefeated => "boss_defeated",
            Self::BossDamageSummary => "boss_damage_summary",
            Self::EnemySpawned => "enemy_spawned",
            Self::EnemyRetired => "enemy_retired",
            Self::EnemyNamed => "enemy_named",
            Self::EnemyDiagnostic => "enemy_diagnostic",
            Self::OwnershipTransferred => "ownership_transferred",
            Self::StageEventAdvanced => "stage_event_advanced",
            Self::StageProgressAdvanced => "stage_progress_advanced",
            Self::SpawnToken => "spawn_token",
            Self::Healing => "healing",
            Self::BuffApplied => "buff_applied",
            Self::BuffRemoved => "buff_removed",
            Self::DebuffApplied => "debuff_applied",
            Self::DebuffRemoved => "debuff_removed",
            Self::PlayerDowned => "player_downed",
            Self::Loot => "loot",
            Self::GameMessage => "game_message",
        }
    }
}

/// One normalized gameplay event. Optional fields are intentionally generic: Ecliptica's
/// Udon log messages evolve frequently, while the stable event envelope lets older databases
/// remain readable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameEvent {
    pub sequence: u64,
    pub timestamp: NaiveDateTime,
    pub kind: EventKind,
    pub player: Option<String>,
    pub session_id: Option<u32>,
    pub world: Option<String>,
    /// VRChat world/instance string (for example `wrld_…:12345~region(us)`). This lets logs
    /// from several players be merged even before Ecliptica assigns a session ID.
    pub instance: Option<String>,
    pub stage: Option<String>,
    pub class_name: Option<String>,
    pub boss: Option<String>,
    pub phase: Option<f64>,
    pub amount: Option<f64>,
    pub damage_type: Option<String>,
    pub source: Option<String>,
    pub target: Option<String>,
    pub entity: Option<String>,
    pub pool_id: Option<u32>,
    pub enemy_id: Option<u32>,
    pub user_id: Option<String>,
    pub message: Option<String>,
    pub raw: String,
}

impl GameEvent {
    pub fn amount(&self) -> f64 {
        self.amount.unwrap_or_default()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DamageTotals {
    pub total: f64,
    pub strike: f64,
    pub non_strike: f64,
    pub hits: u64,
    pub biggest_hit: f64,
    pub dps: f64,
    pub rolling_5s: f64,
    pub rolling_15s: f64,
    pub rolling_30s: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IncomingTotals {
    pub total: f64,
    pub hits: u64,
    pub biggest_hit: f64,
    pub damage_per_second: f64,
    pub by_source: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AttackStats {
    pub name: String,
    pub damage_type: String,
    pub total: f64,
    pub hits: u64,
    pub min: f64,
    pub max: f64,
    pub average: f64,
    pub share: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlayerStats {
    pub player: String,
    pub class_name: Option<String>,
    pub damage: DamageTotals,
    pub incoming: IncomingTotals,
    #[serde(default)]
    pub attacks: Vec<AttackStats>,
    pub healing: f64,
    pub active_seconds: f64,
    pub deaths: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EncounterStats {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: String,
    pub stage: Option<String>,
    pub phase: Option<f64>,
    pub started_at: Option<NaiveDateTime>,
    pub ended_at: Option<NaiveDateTime>,
    pub duration_seconds: f64,
    pub players: Vec<PlayerStats>,
    pub outgoing: DamageTotals,
    pub incoming: IncomingTotals,
    pub attacks: Vec<AttackStats>,
    #[serde(default)]
    pub timeline: Vec<TimelinePoint>,
    pub completed: bool,
    #[serde(default)]
    pub end_reason: String,
    #[serde(default)]
    pub boundary_confidence: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunSummary {
    pub id: String,
    pub session_id: Option<u32>,
    /// Primary analysis window. When named boss windows are present, these timestamps and all
    /// primary combat totals below describe boss combat only.
    pub started_at: Option<NaiveDateTime>,
    pub ended_at: Option<NaiveDateTime>,
    pub duration_seconds: f64,
    #[serde(default)]
    pub metrics_scope: String,
    #[serde(default)]
    pub boss_count: usize,
    /// Complete visit observation retained for diagnostics and pre-boss accounting.
    #[serde(default)]
    pub observed_started_at: Option<NaiveDateTime>,
    #[serde(default)]
    pub observed_ended_at: Option<NaiveDateTime>,
    #[serde(default)]
    pub observed_duration_seconds: f64,
    #[serde(default)]
    pub observed_outgoing: DamageTotals,
    #[serde(default)]
    pub observed_incoming: IncomingTotals,
    #[serde(default)]
    pub pre_boss_duration_seconds: f64,
    #[serde(default)]
    pub pre_boss_outgoing: DamageTotals,
    #[serde(default)]
    pub pre_boss_incoming: IncomingTotals,
    pub class_names: Vec<String>,
    pub stages: Vec<String>,
    pub players: Vec<PlayerStats>,
    pub encounters: Vec<EncounterStats>,
    pub outgoing: DamageTotals,
    pub incoming: IncomingTotals,
    #[serde(default)]
    pub attacks: Vec<AttackStats>,
    #[serde(default)]
    pub timeline: Vec<TimelinePoint>,
    pub completed: bool,
    pub event_count: usize,
    pub source_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TimelinePoint {
    pub timestamp: NaiveDateTime,
    /// Combat-relative time for graphing. Combined boss-run timelines concatenate boss windows,
    /// while encounter timelines start at zero. Older payloads omit this field.
    #[serde(default)]
    pub elapsed_seconds: Option<f64>,
    pub outgoing: f64,
    pub incoming: f64,
    pub rolling_dps: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParserCoverage {
    pub lines_seen: u64,
    pub timestamped_lines: u64,
    pub events_emitted: u64,
    pub relevant_unparsed: u64,
}
