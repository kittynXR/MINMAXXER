use crate::model::{
    AttackStats, DamageTotals, EncounterStats, EventKind, GameEvent, IncomingTotals,
    ParserCoverage, PlayerStats, RunSummary, TimelinePoint,
};
use chrono::{NaiveDateTime, Timelike};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const RECENT_EVENT_LIMIT: usize = 40;
const PHASE_HIT_LIMIT: usize = 12;
const DAMAGE_WINDOW_SECONDS: i64 = 30;
const MERGED_BOUNDARY_TOLERANCE_SECONDS: i64 = 10;
const FOCUS_RECENT_SECONDS: f64 = 45.0;
const FOCUS_AGING_SECONDS: f64 = 90.0;
const FOCUS_CORROBORATION_WINDOW_SECONDS: f64 = 5.0;
const FOCUS_CORROBORATION_DISPLAY_SECONDS: f64 = 8.0;
const RUN_PROGRESS_EPSILON: f64 = 0.000_001;

// The 13 stage-entry progress values observed in the audited Ecliptica run. Normal live
// tracking counts StageEntered announcements, but these anchors let a collector which attaches
// halfway through a run recover a useful (and explicitly labelled) estimate.
const BOSS_PROGRESS_ANCHORS: [f64; 13] = [
    0.0,
    0.058_578_34,
    0.117_840_4,
    0.180_254_8,
    0.242_994,
    0.328_492_1,
    0.418_829_3,
    0.510_591_1,
    0.607_867_4,
    0.698_260_7,
    0.795_897,
    0.900_034_6,
    1.0,
];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LiveEncounter {
    pub name: String,
    pub kind: String,
    pub phase: Option<f64>,
    pub started_at: Option<NaiveDateTime>,
    pub duration_seconds: f64,
    pub active: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecentHit {
    pub timestamp: NaiveDateTime,
    pub amount: f64,
    pub damage_type: String,
    pub age_seconds: f64,
}

/// Best-effort signal derived from an ownership transfer of the entity whose name matches the
/// active boss. Ecliptica does not log an authoritative hate table, so consumers must preserve
/// the `confidence` and `source_note` labels.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FocusSignal {
    pub player: String,
    pub entity: String,
    pub observed_at: NaiveDateTime,
    pub age_seconds: f64,
    pub confidence: String,
    #[serde(default)]
    pub evidence: String,
    #[serde(default)]
    pub corroborating_hits: u32,
    #[serde(default)]
    pub corroborated_at: Option<NaiveDateTime>,
    pub source_note: String,
}

/// A fresh exact-boss focus observation plus the local identity known at that log line. The app
/// consumes this discrete signal for audio alerts so rapid ownership changes cannot be collapsed
/// by the latest-value HUD snapshot channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BossTargetObservationCause {
    Baseline,
    OwnershipTransfer,
    LocalIdentity,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BossTargetObservation {
    pub focus: FocusSignal,
    pub observed_player: Option<String>,
    pub encounter_name: String,
    pub encounter_started_at: Option<NaiveDateTime>,
    pub cause: BossTargetObservationCause,
}

/// Live position within Ecliptica's 13-boss run. `boss_number` describes the upcoming boss while
/// a stage is active and the current boss once its BossStarted line arrives. A recovered mid-log
/// value remains labelled by `boss_number_inferred` until an authoritative special-stage marker
/// is observed or a fresh run is seen from progress zero.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunContext {
    pub progress: Option<f64>,
    pub phase_name: Option<String>,
    pub boss_number: Option<u8>,
    #[serde(default)]
    pub boss_number_inferred: bool,
    pub boss_subphase: Option<u8>,
}

/// Shop selection shape reserved for a future authoritative Ecliptica log signal.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadoutItem {
    pub name: String,
    pub stacks: u32,
}

/// The audited VRChat log does not expose shop selections or stack counts. Keep an additive,
/// honest capability object in the API so every display surface can distinguish unavailable data
/// from an observed empty loadout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoadoutState {
    pub available: bool,
    #[serde(default)]
    pub items: Vec<LoadoutItem>,
    pub source_note: String,
}

impl Default for LoadoutState {
    fn default() -> Self {
        Self {
            available: false,
            items: Vec::new(),
            source_note: "Ecliptica does not expose shop item selections or stack counts in the audited VRChat log.".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineSnapshot {
    pub version: u64,
    pub connected: bool,
    #[serde(default)]
    pub in_world: bool,
    pub status: String,
    pub observed_player: Option<String>,
    pub session_id: Option<u32>,
    pub world: Option<String>,
    pub stage: Option<String>,
    pub class_name: Option<String>,
    #[serde(default)]
    pub run_context: RunContext,
    #[serde(default)]
    pub loadout: LoadoutState,
    pub encounter: LiveEncounter,
    pub focus: Option<FocusSignal>,
    pub outgoing: DamageTotals,
    pub incoming: IncomingTotals,
    pub players: Vec<PlayerStats>,
    pub roster: Vec<String>,
    pub attacks: Vec<AttackStats>,
    pub timeline: Vec<TimelinePoint>,
    /// Newest-first local outgoing hit feed. This is deliberately precomputed so the OBS,
    /// desktop, and SteamVR surfaces can render it without scanning the full event stream.
    pub recent_hits: Vec<RecentHit>,
    pub recent_events: Vec<GameEvent>,
    pub parser_coverage: ParserCoverage,
    pub source_file: Option<String>,
    pub last_event_at: Option<NaiveDateTime>,
    pub capability_note: String,
}

impl Default for EngineSnapshot {
    fn default() -> Self {
        Self {
            version: 0,
            connected: false,
            in_world: false,
            status: "Waiting for VRChat".to_owned(),
            observed_player: None,
            session_id: None,
            world: None,
            stage: None,
            class_name: None,
            run_context: RunContext::default(),
            loadout: LoadoutState::default(),
            encounter: LiveEncounter::default(),
            focus: None,
            outgoing: DamageTotals::default(),
            incoming: IncomingTotals::default(),
            players: Vec::new(),
            roster: Vec::new(),
            attacks: Vec::new(),
            timeline: Vec::new(),
            recent_hits: Vec::new(),
            recent_events: Vec::new(),
            parser_coverage: ParserCoverage::default(),
            source_file: None,
            last_event_at: None,
            capability_note: capability_note(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CombatEngine {
    version: u64,
    connected: bool,
    in_world: bool,
    observed_player: Option<String>,
    session_id: Option<u32>,
    world: Option<String>,
    stage: Option<String>,
    class_name: Option<String>,
    run_context: RunContext,
    run_stage_markers: BTreeSet<(String, u64)>,
    run_session_id: Option<u32>,
    encounter: LiveEncounter,
    focus: Option<FocusSignal>,
    outgoing: DamageTotals,
    outgoing_healing: f64,
    incoming: IncomingTotals,
    roster: BTreeSet<String>,
    damage_by_type: BTreeMap<String, (f64, u64, f64, f64)>,
    damage_window: VecDeque<(NaiveDateTime, f64)>,
    timeline: VecDeque<TimelinePoint>,
    phase_hits: VecDeque<RecentHit>,
    recent_events: VecDeque<GameEvent>,
    coverage: ParserCoverage,
    source_file: Option<String>,
    last_event_at: Option<NaiveDateTime>,
}

impl Default for CombatEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl CombatEngine {
    pub fn new() -> Self {
        Self {
            version: 0,
            connected: false,
            in_world: false,
            observed_player: None,
            session_id: None,
            world: None,
            stage: None,
            class_name: None,
            run_context: RunContext::default(),
            run_stage_markers: BTreeSet::new(),
            run_session_id: None,
            encounter: LiveEncounter::default(),
            focus: None,
            outgoing: DamageTotals::default(),
            outgoing_healing: 0.0,
            incoming: IncomingTotals::default(),
            roster: BTreeSet::new(),
            damage_by_type: BTreeMap::new(),
            damage_window: VecDeque::new(),
            timeline: VecDeque::new(),
            phase_hits: VecDeque::new(),
            recent_events: VecDeque::new(),
            coverage: ParserCoverage::default(),
            source_file: None,
            last_event_at: None,
        }
    }

    pub fn set_source_file(&mut self, source_file: Option<String>) {
        self.source_file = source_file;
        self.connected = self.source_file.is_some();
        self.version += 1;
    }

    pub fn set_coverage(&mut self, coverage: ParserCoverage) {
        self.coverage = coverage;
    }

    pub fn ingest(&mut self, event: GameEvent) {
        let _ = self.ingest_inner(event);
    }

    pub fn ingest_with_boss_target_observation(
        &mut self,
        event: GameEvent,
    ) -> Option<BossTargetObservation> {
        self.ingest_inner(event)
    }

    /// Returns the retained exact-boss target candidate without requiring it to be fresh. Alert
    /// consumers use this only to synchronize silently after a log replay or source reset.
    pub fn boss_target_baseline(&self) -> Option<BossTargetObservation> {
        if !self.encounter.active || self.encounter.kind != "boss" {
            return None;
        }
        Some(BossTargetObservation {
            focus: self.focus.clone()?,
            observed_player: self.observed_player.clone(),
            encounter_name: self.encounter.name.clone(),
            encounter_started_at: self.encounter.started_at,
            cause: BossTargetObservationCause::Baseline,
        })
    }

    fn ingest_inner(&mut self, event: GameEvent) -> Option<BossTargetObservation> {
        let event_timestamp = event.timestamp;
        let local_identity_changed = event.kind == EventKind::LocalPlayerIdentified;
        let mut focus_changed = false;
        self.version += 1;
        self.last_event_at = Some(event.timestamp);
        if event.player.is_some() {
            self.observed_player = event.player.clone();
        }
        if event.session_id.is_some() {
            self.session_id = event.session_id;
        }
        if event.world.is_some() {
            self.world = event.world.clone();
        }
        // BossStarted lines can arrive late after a later run-progress value is already active.
        // Delay their stage assignment until the transition has passed the stale-line guard.
        if !matches!(event.kind, EventKind::StageEntered | EventKind::BossStarted)
            && event.stage.is_some()
        {
            self.stage = event.stage.clone();
        }
        if event.class_name.is_some() {
            self.class_name = event.class_name.clone();
        }

        match event.kind {
            EventKind::WorldEntered => {
                self.in_world = true;
                self.reset_encounter();
                self.reset_run_context();
                self.session_id = None;
                self.stage = None;
                self.class_name = None;
                self.roster.clear();
                self.recent_events.clear();
                if let Some(player) = self.observed_player.as_ref() {
                    self.roster.insert(player.clone());
                }
            }
            EventKind::WorldExited => {
                self.in_world = false;
                self.encounter.active = false;
                self.focus = None;
                self.phase_hits.clear();
                self.reset_run_context();
                self.session_id = None;
                self.world = None;
                self.stage = None;
                self.class_name = None;
                self.roster.clear();
            }
            EventKind::PlayerJoined => {
                if let Some(player) = event.target.as_ref() {
                    self.roster.insert(player.clone());
                }
            }
            EventKind::PlayerLeft => {
                if let Some(player) = event.target.as_ref() {
                    self.roster.remove(player);
                    if self
                        .focus
                        .as_ref()
                        .is_some_and(|focus| focus.player == *player)
                    {
                        self.focus = None;
                    }
                }
            }
            EventKind::LocalPlayerIdentified => {
                if let Some(player) = event.player.as_ref() {
                    self.observed_player = Some(player.clone());
                    self.roster.insert(player.clone());
                }
            }
            EventKind::StageEntered => {
                if self.observe_stage_entered(&event) {
                    if event.stage.is_some() {
                        self.stage = event.stage.clone();
                    }
                    self.begin_encounter(
                        event.stage.clone().unwrap_or_else(|| "Stage".to_owned()),
                        "stage",
                        event.timestamp,
                        event.phase,
                    );
                }
            }
            EventKind::BossStarted => {
                if self.observe_boss_started(&event) {
                    if event.stage.is_some() {
                        self.stage = event.stage.clone();
                    }
                    self.focus = None;
                    self.begin_encounter(
                        event.boss.clone().unwrap_or_else(|| "Boss".to_owned()),
                        "boss",
                        event.timestamp,
                        event.phase,
                    );
                }
            }
            EventKind::Intermission | EventKind::Lobby => {
                self.update_duration(event.timestamp);
                self.encounter.active = false;
                self.focus = None;
                self.phase_hits.clear();
                self.run_context.boss_subphase = None;
                if event.kind == EventKind::Lobby {
                    self.stage = None;
                    self.class_name = None;
                    self.reset_run_context();
                }
            }
            EventKind::BossDefeated => {
                // Phase N's named completion can be buffered until after phase N+1 starts. Never
                // let that late event close the new live phase.
                let closes_current = event
                    .boss
                    .as_ref()
                    .is_some_and(|boss| entity_key(boss) == entity_key(&self.encounter.name));
                if closes_current {
                    self.update_duration(event.timestamp);
                    self.encounter.active = false;
                    self.focus = None;
                    self.phase_hits.clear();
                    self.run_context.boss_subphase = None;
                }
            }
            EventKind::GameMessage if event.message.as_deref() == Some("session_invalidated") => {
                self.session_id = None;
            }
            EventKind::DamageDealt => self.add_outgoing(&event),
            EventKind::Healing => self.add_outgoing_healing(&event),
            EventKind::DamageTaken => self.add_incoming(&event),
            EventKind::OwnershipTransferred => focus_changed = self.observe_focus(&event),
            _ => {}
        }

        if self.recent_events.len() == RECENT_EVENT_LIMIT {
            self.recent_events.pop_front();
        }
        self.recent_events.push_back(event);

        if !focus_changed && !local_identity_changed {
            return None;
        }
        if !self.encounter.active || self.encounter.kind != "boss" {
            return None;
        }
        let focus = self.current_focus_at(Some(event_timestamp))?;
        if !matches!(focus.confidence.as_str(), "possible" | "likely") {
            return None;
        }
        Some(BossTargetObservation {
            focus,
            observed_player: self.observed_player.clone(),
            encounter_name: self.encounter.name.clone(),
            encounter_started_at: self.encounter.started_at,
            cause: if focus_changed {
                BossTargetObservationCause::OwnershipTransfer
            } else {
                BossTargetObservationCause::LocalIdentity
            },
        })
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        self.snapshot_at(self.last_event_at)
    }

    /// Builds a live view at a wall-clock instant without mutating parser state. Collectors use
    /// this while an encounter is active so rolling DPS, elapsed time, recent-hit ages, and the
    /// inferred focus expiry continue to advance even when the log is momentarily quiet.
    pub fn snapshot_at(&self, now: Option<NaiveDateTime>) -> EngineSnapshot {
        let clock = match (self.last_event_at, now) {
            (Some(event_at), Some(now)) => Some(event_at.max(now)),
            (event_at, None) => event_at,
            (None, now) => now,
        };
        let observation_end = if self.encounter.active {
            clock
        } else {
            self.last_event_at
        };
        let mut outgoing = self.outgoing.clone();
        if let (Some(start), Some(end)) = (self.encounter.started_at, observation_end) {
            let seconds = seconds_between(start, end).max(1.0);
            outgoing.dps = outgoing.total / seconds;
        }
        outgoing.rolling_5s = rolling_sum(&self.damage_window, clock, 5) / 5.0;
        outgoing.rolling_15s = rolling_sum(&self.damage_window, clock, 15) / 15.0;
        outgoing.rolling_30s = rolling_sum(&self.damage_window, clock, 30) / 30.0;

        let mut incoming = self.incoming.clone();
        if let (Some(start), Some(end)) = (self.encounter.started_at, observation_end) {
            incoming.damage_per_second = incoming.total / seconds_between(start, end).max(1.0);
        }

        let active_seconds = self
            .encounter
            .started_at
            .zip(observation_end)
            .map(|(start, end)| seconds_between(start, end))
            .unwrap_or_default();
        let mut encounter = self.encounter.clone();
        if let (Some(start), Some(end)) = (encounter.started_at, observation_end) {
            encounter.duration_seconds = seconds_between(start, end);
        }
        let players = self
            .observed_player
            .as_ref()
            .map(|player| {
                vec![PlayerStats {
                    player: player.clone(),
                    class_name: self.class_name.clone(),
                    damage: outgoing.clone(),
                    incoming: incoming.clone(),
                    attacks: self.attack_stats(),
                    healing: self.outgoing_healing,
                    active_seconds,
                    deaths: 0,
                }]
            })
            .unwrap_or_default();
        let mut timeline: Vec<_> = self.timeline.iter().cloned().collect();
        if let Some(started_at) = self.encounter.started_at {
            if timeline
                .first()
                .is_none_or(|point| point.timestamp > started_at)
            {
                timeline.insert(
                    0,
                    TimelinePoint {
                        timestamp: started_at,
                        elapsed_seconds: Some(0.0),
                        outgoing: 0.0,
                        incoming: 0.0,
                        rolling_dps: 0.0,
                    },
                );
            }
        }
        if let Some(timestamp) = observation_end {
            if let Some(last) = timeline
                .last_mut()
                .filter(|point| point.timestamp == timestamp)
            {
                last.rolling_dps = outgoing.rolling_5s;
            } else if timeline
                .last()
                .is_none_or(|point| point.timestamp < timestamp)
            {
                timeline.push(TimelinePoint {
                    timestamp,
                    elapsed_seconds: self
                        .encounter
                        .started_at
                        .map(|start| seconds_between(start, timestamp)),
                    outgoing: 0.0,
                    incoming: 0.0,
                    rolling_dps: outgoing.rolling_5s,
                });
            }
        }

        EngineSnapshot {
            version: self.version,
            connected: self.connected,
            in_world: self.in_world,
            status: if !self.connected {
                "Waiting for a VRChat log".to_owned()
            } else if !self.in_world {
                "Watching VRChat — outside Ecliptica".to_owned()
            } else if self.encounter.active {
                format!("Live: {}", self.encounter.name)
            } else {
                "Ecliptica — between encounters".to_owned()
            },
            observed_player: self.observed_player.clone(),
            session_id: self.session_id,
            world: self.world.clone(),
            stage: self.stage.clone(),
            class_name: self.class_name.clone(),
            run_context: self.run_context.clone(),
            loadout: LoadoutState::default(),
            encounter,
            focus: self.current_focus_at(clock),
            outgoing,
            incoming,
            players,
            roster: self.roster.iter().cloned().collect(),
            attacks: self.attack_stats(),
            timeline,
            recent_hits: self.recent_hits_at(12, clock),
            recent_events: self.recent_events.iter().cloned().collect(),
            parser_coverage: self.coverage.clone(),
            source_file: self.source_file.clone(),
            last_event_at: self.last_event_at,
            capability_note: capability_note(),
        }
    }

    pub fn requires_clock_tick(&self) -> bool {
        self.requires_clock_tick_at(self.last_event_at)
    }

    pub fn requires_clock_tick_at(&self, now: Option<NaiveDateTime>) -> bool {
        self.encounter.active || self.current_focus_at(now).is_some()
    }

    fn begin_encounter(
        &mut self,
        name: String,
        kind: &str,
        timestamp: NaiveDateTime,
        phase: Option<f64>,
    ) {
        // Buffered network events can repeat an identical boss-start line. Preserve the existing
        // totals when the same encounter is announced again within a few seconds.
        if self.encounter.active
            && self.encounter.name == name
            && self
                .encounter
                .started_at
                .is_some_and(|start| seconds_between(start, timestamp) < 5.0)
        {
            return;
        }
        self.reset_encounter();
        self.encounter = LiveEncounter {
            name,
            kind: kind.to_owned(),
            phase,
            started_at: Some(timestamp),
            duration_seconds: 0.0,
            active: true,
        };
    }

    /// Records a distinct run stage and returns whether it should begin a new live encounter.
    fn observe_stage_entered(&mut self, event: &GameEvent) -> bool {
        let progress = valid_run_progress(event.phase);
        let stage = event.stage.as_deref().unwrap_or("Stage");
        let session_changed = self
            .run_session_id
            .zip(event.session_id)
            .is_some_and(|(previous, current)| previous != current);
        let progress_rolled_back = self
            .run_context
            .progress
            .zip(progress)
            .is_some_and(|(previous, current)| current + RUN_PROGRESS_EPSILON < previous);
        let marker = (
            entity_key(stage),
            progress.map(f64::to_bits).unwrap_or(u64::MAX),
        );

        // Buffered stage announcements can arrive after the run has advanced. Check a known
        // marker before considering rollback so a delayed copy can never reset the live boss,
        // its damage, or its target evidence. A changed session is allowed to reuse the same
        // stage marker because it is an explicit new-run boundary.
        if !session_changed && self.run_stage_markers.contains(&marker) {
            return false;
        }

        // Numeric progress alone is not a trustworthy run boundary: old stage lines are often
        // replayed out of order. Only a session change or a newly observed progress-zero stage
        // can start a fresh count. Any other backwards announcement is ignored in full.
        let verified_zero_start = progress.is_some_and(|current| {
            current <= RUN_PROGRESS_EPSILON
                && self
                    .run_context
                    .progress
                    .is_some_and(|previous| previous > RUN_PROGRESS_EPSILON)
        });
        if session_changed || verified_zero_start {
            self.reset_run_context();
        } else if progress_rolled_back {
            return false;
        }

        let boss_number = if is_bringer_stage(stage) {
            Some(13)
        } else if let Some(previous) = self.run_context.boss_number {
            Some(previous.saturating_add(1).min(13))
        } else if progress.is_some_and(|value| value <= RUN_PROGRESS_EPSILON) {
            Some(1)
        } else {
            progress.and_then(infer_boss_number)
        };
        let boss_number_inferred = if is_bringer_stage(stage)
            || progress.is_some_and(|value| value <= RUN_PROGRESS_EPSILON)
        {
            false
        } else if self.run_context.boss_number.is_some() {
            self.run_context.boss_number_inferred
        } else {
            boss_number.is_some()
        };

        self.set_run_progress(progress);
        self.run_context.boss_number = boss_number;
        self.run_context.boss_number_inferred = boss_number_inferred;
        self.run_context.boss_subphase = None;
        self.run_stage_markers.insert(marker);
        if event.session_id.is_some() {
            self.run_session_id = event.session_id;
        }
        true
    }

    /// Updates run position for an accepted boss announcement. Returns false for an old replay so
    /// the caller can reject the entire encounter transition, not merely its ordinal metadata.
    fn observe_boss_started(&mut self, event: &GameEvent) -> bool {
        let progress = valid_run_progress(event.phase);
        let boss = event.boss.as_deref().unwrap_or("Boss");
        let is_bringer = event.stage.as_deref().is_some_and(is_bringer_stage)
            || entity_key(boss).starts_with("jimbringer");
        let session_changed = self
            .run_session_id
            .zip(event.session_id)
            .is_some_and(|(previous, current)| previous != current);
        if session_changed {
            // A collector may miss the new run's first StageEntered line. Session identity is a
            // stronger boundary than a lower progress value, so reset before the stale guard and
            // recover from this boss announcement.
            self.reset_run_context();
        }
        let announces_earlier_progress = self
            .run_context
            .progress
            .zip(progress)
            .is_some_and(|(previous, current)| current + RUN_PROGRESS_EPSILON < previous);
        let announces_unseen_progress = match (self.run_context.progress, progress) {
            // BossStarted is not a run-boundary signal. Accept forward progress when a stage line
            // was missed, but never let a delayed/replayed earlier boss roll live context back.
            (Some(previous), Some(current)) => current > previous + RUN_PROGRESS_EPSILON,
            (None, Some(_)) => true,
            _ => false,
        };

        if is_bringer {
            // Some late/replayed JimBringer announcements carry a misleading zero phase. The
            // named final boss and Stage_Bringer are stronger signals than that recycled field.
            self.set_run_progress(Some(1.0));
            self.run_context.boss_number = Some(13);
            self.run_context.boss_number_inferred = false;
        } else {
            if announces_earlier_progress {
                return false;
            }
            if self.run_context.boss_number.is_none() || announces_unseen_progress {
                self.set_run_progress(progress);
                self.run_context.boss_number = progress.and_then(infer_boss_number);
                self.run_context.boss_number_inferred = self.run_context.boss_number.is_some();
            }
        }
        self.run_context.boss_subphase = parse_boss_subphase(boss);
        if let (Some(stage), Some(progress)) = (event.stage.as_deref(), progress) {
            self.run_stage_markers
                .insert((entity_key(stage), progress.to_bits()));
        }
        if event.session_id.is_some() {
            self.run_session_id = event.session_id;
        }
        true
    }

    fn set_run_progress(&mut self, progress: Option<f64>) {
        if let Some(progress) = progress {
            self.run_context.progress = Some(progress);
            self.run_context.phase_name = phase_name_for_progress(progress).map(str::to_owned);
        }
    }

    fn reset_run_context(&mut self) {
        self.run_context = RunContext::default();
        self.run_stage_markers.clear();
        self.run_session_id = None;
    }

    fn reset_encounter(&mut self) {
        self.encounter = LiveEncounter::default();
        self.focus = None;
        self.outgoing = DamageTotals::default();
        self.outgoing_healing = 0.0;
        self.incoming = IncomingTotals::default();
        self.damage_by_type.clear();
        self.damage_window.clear();
        self.timeline.clear();
        self.phase_hits.clear();
    }

    fn add_outgoing(&mut self, event: &GameEvent) {
        let amount = event.amount();
        if amount <= 0.0 {
            return;
        }
        self.ensure_combat_started(event.timestamp);
        self.outgoing.total += amount;
        self.outgoing.hits += 1;
        self.outgoing.biggest_hit = self.outgoing.biggest_hit.max(amount);
        let damage_type = event
            .damage_type
            .clone()
            .unwrap_or_else(|| "unknown".to_owned());
        if damage_type == "strike" {
            self.outgoing.strike += amount;
        } else {
            self.outgoing.non_strike += amount;
        }
        let entry =
            self.damage_by_type
                .entry(damage_type.clone())
                .or_insert((0.0, 0, f64::MAX, 0.0));
        entry.0 += amount;
        entry.1 += 1;
        entry.2 = entry.2.min(amount);
        entry.3 = entry.3.max(amount);
        self.damage_window.push_back((event.timestamp, amount));
        while self.damage_window.front().is_some_and(|(timestamp, _)| {
            (event.timestamp - *timestamp).num_seconds() > DAMAGE_WINDOW_SECONDS
        }) {
            self.damage_window.pop_front();
        }
        self.push_timeline(event.timestamp, amount, 0.0);
        if self.phase_hits.len() == PHASE_HIT_LIMIT {
            self.phase_hits.pop_back();
        }
        self.phase_hits.push_front(RecentHit {
            timestamp: event.timestamp,
            amount,
            damage_type,
            age_seconds: 0.0,
        });
        self.update_duration(event.timestamp);
    }

    fn add_outgoing_healing(&mut self, event: &GameEvent) {
        let healer = event.player.as_deref().or(self.observed_player.as_deref());
        let Some(amount) = healer.and_then(|healer| outgoing_healing_amount(event, healer)) else {
            return;
        };
        self.ensure_combat_started(event.timestamp);
        self.outgoing_healing += amount;
        self.update_duration(event.timestamp);
    }

    fn add_incoming(&mut self, event: &GameEvent) {
        let amount = event.amount();
        if amount <= 0.0 {
            return;
        }
        self.ensure_combat_started(event.timestamp);
        self.incoming.total += amount;
        self.incoming.hits += 1;
        self.incoming.biggest_hit = self.incoming.biggest_hit.max(amount);
        *self
            .incoming
            .by_source
            .entry(event.source.clone().unwrap_or_else(|| "unknown".to_owned()))
            .or_default() += amount;
        self.push_timeline(event.timestamp, 0.0, amount);
        self.update_duration(event.timestamp);
        self.corroborate_focus(event);
    }

    fn ensure_combat_started(&mut self, timestamp: NaiveDateTime) {
        if self.encounter.started_at.is_none() {
            self.encounter.started_at = Some(timestamp);
            self.encounter.active = true;
            self.encounter.kind = "combat".to_owned();
            if self.encounter.name.is_empty() {
                self.encounter.name = self
                    .stage
                    .clone()
                    .unwrap_or_else(|| "Ecliptica combat".to_owned());
            }
        }
    }

    fn update_duration(&mut self, timestamp: NaiveDateTime) {
        if let Some(started_at) = self.encounter.started_at {
            self.encounter.duration_seconds = seconds_between(started_at, timestamp);
        }
    }

    fn push_timeline(&mut self, timestamp: NaiveDateTime, outgoing: f64, incoming: f64) {
        let rolling_dps = rolling_sum(&self.damage_window, Some(timestamp), 5) / 5.0;
        if let Some(last) = self.timeline.back_mut() {
            if last.timestamp == timestamp {
                last.outgoing += outgoing;
                last.incoming += incoming;
                last.rolling_dps = rolling_dps;
                return;
            }
        }
        if self.timeline.len() == 600 {
            self.timeline.pop_front();
        }
        self.timeline.push_back(TimelinePoint {
            timestamp,
            elapsed_seconds: self
                .encounter
                .started_at
                .map(|start| seconds_between(start, timestamp)),
            outgoing,
            incoming,
            rolling_dps,
        });
    }

    fn attack_stats(&self) -> Vec<AttackStats> {
        let total = self.outgoing.total.max(1.0);
        let mut attacks: Vec<_> = self
            .damage_by_type
            .iter()
            .map(|(name, (damage, hits, min, max))| AttackStats {
                name: friendly_damage_type(name),
                damage_type: name.clone(),
                total: *damage,
                hits: *hits,
                min: if *min == f64::MAX { 0.0 } else { *min },
                max: *max,
                average: if *hits == 0 {
                    0.0
                } else {
                    *damage / *hits as f64
                },
                share: *damage / total,
            })
            .collect();
        attacks.sort_by(|a, b| b.total.total_cmp(&a.total));
        attacks
    }

    fn observe_focus(&mut self, event: &GameEvent) -> bool {
        if !self.encounter.active || self.encounter.kind != "boss" {
            return false;
        }
        let (Some(entity), Some(player)) = (event.entity.as_ref(), event.target.as_ref()) else {
            return false;
        };
        let entity_name_key = entity_key(entity);
        let boss_key = entity_key(&self.encounter.name);
        // Sub-entities such as Fly, M41DPillar, M41DTower, and GravetenderOrb transfer ownership
        // independently. Symmetric substring matching turns those mechanics into false targets.
        if entity_name_key.is_empty() || boss_key.is_empty() || entity_name_key != boss_key {
            return false;
        }
        self.focus = Some(FocusSignal {
            player: player.clone(),
            entity: entity.clone(),
            observed_at: event.timestamp,
            age_seconds: 0.0,
            confidence: "possible".to_owned(),
            evidence: "boss_network_ownership".to_owned(),
            corroborating_hits: 0,
            corroborated_at: None,
            source_note:
                "Exact boss network ownership; a useful target proxy, not authoritative hate/aggro."
                    .to_owned(),
        });
        true
    }

    fn corroborate_focus(&mut self, event: &GameEvent) {
        let Some(focus) = self.focus.as_mut() else {
            return;
        };
        let Some(local_player) = event.player.as_deref().or(self.observed_player.as_deref()) else {
            return;
        };
        if focus.player != local_player
            || seconds_between(focus.observed_at, event.timestamp)
                > FOCUS_CORROBORATION_WINDOW_SECONDS
        {
            return;
        }

        focus.corroborating_hits = focus.corroborating_hits.saturating_add(1);
        let source_key = event.source.as_deref().map(entity_key).unwrap_or_default();
        let boss_key = entity_key(&self.encounter.name);
        let explicitly_boss_named =
            !source_key.is_empty() && !boss_key.is_empty() && source_key.contains(&boss_key);
        if explicitly_boss_named || focus.corroborating_hits >= 2 {
            focus.corroborated_at = Some(event.timestamp);
            focus.evidence = "boss_owner_plus_local_incoming".to_owned();
        }
    }

    fn current_focus_at(&self, now: Option<NaiveDateTime>) -> Option<FocusSignal> {
        let mut focus = self.focus.clone()?;
        focus.age_seconds = self
            .last_event_at
            .max(now)
            .map(|now| seconds_between(focus.observed_at, now))
            .unwrap_or_default();
        let corroboration_age = focus
            .corroborated_at
            .zip(self.last_event_at.max(now))
            .map(|(observed, now)| seconds_between(observed, now));
        if corroboration_age.is_some_and(|age| age <= FOCUS_CORROBORATION_DISPLAY_SECONDS) {
            focus.confidence = "likely".to_owned();
            focus.source_note = "Exact boss ownership plus immediate local incoming damage; still not an authoritative hate table."
                .to_owned();
        } else if focus.age_seconds <= FOCUS_RECENT_SECONDS {
            focus.confidence = "possible".to_owned();
            focus.source_note = "Exact boss network ownership; a useful target proxy, not authoritative hate/aggro."
                .to_owned();
        } else if focus.age_seconds <= FOCUS_AGING_SECONDS {
            focus.confidence = "aging".to_owned();
            focus.source_note = "No newer exact boss-owner transfer has been logged; showing the last observed candidate."
                .to_owned();
        } else {
            focus.confidence = "stale".to_owned();
            focus.source_note =
                "Stale boss-owner candidate; retained until a new transfer or encounter boundary."
                    .to_owned();
        }
        Some(focus)
    }

    fn recent_hits_at(&self, limit: usize, now: Option<NaiveDateTime>) -> Vec<RecentHit> {
        self.phase_hits
            .iter()
            .take(limit)
            .cloned()
            .map(|mut hit| {
                hit.age_seconds = now
                    .map(|now| seconds_between(hit.timestamp, now))
                    .unwrap_or_default();
                hit
            })
            .collect()
    }
}

pub fn analyze_runs(events: &[GameEvent]) -> Vec<RunSummary> {
    let mut runs: Vec<_> = group_run_events(events)
        .into_values()
        .map(|mut run| {
            run.events
                .sort_by_key(|event| (event.timestamp, event.sequence));
            summarize_run(run.id, run.session_id, &run.events)
        })
        .collect();
    runs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    runs
}

/// Returns the normalized event membership for one run ID using the same visit/session
/// segmentation as [`analyze_runs`]. Consumers should use this instead of reconstructing the
/// old session-or-clock-hour grouping themselves.
pub fn events_for_run(events: &[GameEvent], run_id: &str) -> Vec<GameEvent> {
    let Some(mut run) = group_run_events(events).remove(run_id) else {
        return Vec::new();
    };
    run.events
        .sort_by_key(|event| (event.timestamp, event.sequence));
    run.events
}

#[derive(Debug)]
struct GroupedRun {
    id: String,
    session_id: Option<u32>,
    events: Vec<GameEvent>,
}

fn group_run_events(events: &[GameEvent]) -> BTreeMap<String, GroupedRun> {
    let mut streams: BTreeMap<String, Vec<GameEvent>> = BTreeMap::new();
    for event in events.iter().filter(|event| is_ecliptica_event(event)) {
        let stream = event
            .player
            .clone()
            .unwrap_or_else(|| "__unknown_local_player__".to_owned());
        streams.entry(stream).or_default().push(event.clone());
    }

    let mut grouped: BTreeMap<String, GroupedRun> = BTreeMap::new();
    for (player, mut stream) in streams {
        stream.sort_by_key(|event| (event.timestamp, event.sequence));
        for visit in split_world_visits(stream) {
            if !visit.iter().any(is_run_activity) {
                continue;
            }

            let session_id = select_visit_session(&visit);
            let id = match session_id {
                Some(session_id) => format!("session-{session_id}"),
                None => visit
                    .iter()
                    .find_map(|event| event.instance.as_deref())
                    .map(|instance| format!("instance-{}", slug(instance)))
                    .unwrap_or_else(|| {
                        let observed_at = visit
                            .first()
                            .expect("a visit always contains an event")
                            .timestamp;
                        format!(
                            "observed-{}-{}",
                            observed_at.format("%Y%m%d-%H%M"),
                            slug(&player)
                        )
                    }),
            };
            grouped
                .entry(id.clone())
                .and_modify(|run| run.events.extend(visit.clone()))
                .or_insert_with(|| GroupedRun {
                    id,
                    session_id,
                    events: visit,
                });
        }
    }
    grouped
}

fn split_world_visits(events: Vec<GameEvent>) -> Vec<Vec<GameEvent>> {
    let mut visits = Vec::new();
    let mut current = Vec::new();
    for event in events {
        if event.kind == EventKind::WorldEntered && !current.is_empty() {
            visits.push(std::mem::take(&mut current));
        }
        let exits_world = event.kind == EventKind::WorldExited;
        current.push(event);
        if exits_world {
            visits.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        visits.push(current);
    }
    visits
}

fn select_visit_session(events: &[GameEvent]) -> Option<u32> {
    // A save is Ecliptica's strongest statement of the active run. In particular, it overrides
    // the stale numeric load often seen immediately before a mismatch line.
    if let Some(session_id) = events.iter().rev().find_map(|event| {
        (event.kind == EventKind::SessionSaved)
            .then_some(event.session_id)
            .flatten()
    }) {
        return Some(session_id);
    }

    // A loaded ID is accepted only when gameplay subsequently carries that same ID before an
    // explicit invalidation. This also rejects old databases written before invalidation lines
    // were normalized: their post-mismatch gameplay events have a cleared session_id.
    for (index, event) in events.iter().enumerate().rev() {
        if event.kind != EventKind::SessionLoaded {
            continue;
        }
        let Some(candidate) = event.session_id else {
            continue;
        };
        if events[index + 1..]
            .iter()
            .take_while(|event| !is_session_invalidation(event))
            .any(|event| is_run_activity(event) && event.session_id == Some(candidate))
        {
            return Some(candidate);
        }
    }

    // Partial imports may begin after the load/save line. A session ID repeated on gameplay is
    // still trustworthy; session-only metadata is deliberately excluded from this fallback.
    let mut activity_sessions: BTreeMap<u32, usize> = BTreeMap::new();
    for session_id in events
        .iter()
        .filter(|event| is_run_activity(event))
        .filter_map(|event| event.session_id)
    {
        *activity_sessions.entry(session_id).or_default() += 1;
    }
    activity_sessions
        .into_iter()
        .max_by_key(|(session_id, count)| (*count, *session_id))
        .map(|(session_id, _)| session_id)
}

fn is_ecliptica_event(event: &GameEvent) -> bool {
    event
        .world
        .as_deref()
        .is_some_and(|world| world.to_ascii_lowercase().contains("ecliptica"))
}

fn is_session_invalidation(event: &GameEvent) -> bool {
    event.kind == EventKind::GameMessage && event.message.as_deref() == Some("session_invalidated")
}

fn is_run_activity(event: &GameEvent) -> bool {
    matches!(
        event.kind,
        EventKind::StageEntered
            | EventKind::BossStarted
            | EventKind::DamageDealt
            | EventKind::DamageTaken
            | EventKind::BossDefeated
            | EventKind::EnemySpawned
            | EventKind::StageEventAdvanced
            | EventKind::StageProgressAdvanced
    )
}

fn summarize_run(id: String, session_id: Option<u32>, events: &[GameEvent]) -> RunSummary {
    let observed_started_at = events
        .iter()
        .find(|event| {
            matches!(
                event.kind,
                EventKind::StageEntered | EventKind::BossStarted | EventKind::DamageDealt
            )
        })
        .or_else(|| events.first())
        .map(|event| event.timestamp);
    // Lobby is the strongest end-of-run marker exposed by the audited logs. Retain a short peer
    // tolerance for imported collectors whose final hit lands just after the first lobby line,
    // then exclude the arbitrary post-run stay from both numerator and denominator.
    let first_completion = observed_started_at.and_then(|started_at| {
        events
            .iter()
            .find(|event| event.kind == EventKind::Lobby && event.timestamp >= started_at)
            .or_else(|| {
                events.iter().find(|event| {
                    event.kind == EventKind::WorldExited && event.timestamp >= started_at
                })
            })
            .map(|event| event.timestamp)
    });
    let effective_len = first_completion
        .map(|completed_at| {
            let deadline =
                completed_at + chrono::Duration::seconds(MERGED_BOUNDARY_TOLERANCE_SECONDS);
            events
                .iter()
                .take_while(|event| event.timestamp <= deadline)
                .count()
                .max(1)
        })
        .unwrap_or(events.len());
    let events = &events[..effective_len];
    let observed_ended_at = first_completion
        .map(|completed_at| {
            events
                .iter()
                .filter(|event| {
                    event.timestamp >= observed_started_at.unwrap_or(event.timestamp)
                        && matches!(
                            event.kind,
                            EventKind::DamageDealt
                                | EventKind::DamageTaken
                                | EventKind::Lobby
                                | EventKind::WorldExited
                        )
                })
                .map(|event| event.timestamp)
                .max()
                .unwrap_or(completed_at)
        })
        .or_else(|| events.last().map(|event| event.timestamp));
    let observed_duration_seconds = observed_started_at
        .zip(observed_ended_at)
        .map(|(start, end)| seconds_between(start, end))
        .unwrap_or_default();
    let completed = events
        .iter()
        .any(|event| matches!(event.kind, EventKind::Lobby | EventKind::WorldExited));

    let mut classes = BTreeSet::new();
    let mut stages = Vec::new();
    let mut recent_stage_boundaries: BTreeMap<String, NaiveDateTime> = BTreeMap::new();
    for event in events {
        if let Some(class_name) = event.class_name.as_ref() {
            classes.insert(class_name.clone());
        }
        if event.kind == EventKind::StageEntered {
            if let Some(stage) = event.stage.as_ref() {
                let duplicate = recent_stage_boundaries.get(stage).is_some_and(|seen_at| {
                    (event.timestamp - *seen_at).num_seconds().unsigned_abs()
                        <= MERGED_BOUNDARY_TOLERANCE_SECONDS as u64
                });
                recent_stage_boundaries.insert(stage.clone(), event.timestamp);
                if !duplicate {
                    // Stage names repeat legitimately later in a run (GMFuncFlat/GMBigcity), so
                    // retain ordered occurrences and suppress only near-simultaneous peer copies.
                    stages.push(stage.clone());
                }
            }
        }
    }

    let windows = build_encounter_windows(events);
    let encounters: Vec<_> = windows.iter().map(|window| window.stats.clone()).collect();
    let boss_windows: Vec<_> = windows
        .iter()
        .filter(|window| window.stats.kind == "boss")
        .collect();
    let pre_boss_windows: Vec<_> = windows
        .iter()
        .filter(|window| window.stats.kind == "pre_boss")
        .collect();
    let boss_count = boss_windows.len();
    let boss_events: Vec<_> = boss_windows
        .iter()
        .flat_map(|window| events[window.start..window.effective_end].iter())
        .collect();
    let pre_boss_events: Vec<_> = pre_boss_windows
        .iter()
        .flat_map(|window| events[window.start..window.effective_end].iter())
        .collect();

    let observed_outgoing = summarize_damage_events(events.iter(), observed_duration_seconds);
    let observed_incoming = summarize_incoming_events(events.iter(), observed_duration_seconds);
    let pre_boss_duration_seconds = pre_boss_windows
        .iter()
        .map(|window| window.stats.duration_seconds)
        .sum();
    let pre_boss_outgoing =
        summarize_damage_events(pre_boss_events.iter().copied(), pre_boss_duration_seconds);
    let pre_boss_incoming =
        summarize_incoming_events(pre_boss_events.iter().copied(), pre_boss_duration_seconds);

    // A run with named boss windows is analyzed exclusively over those windows. Partial imports
    // that begin after a start marker retain an honest observed-combat fallback.
    let metrics_scope = if boss_count > 0 {
        "boss"
    } else {
        "observed_combat"
    };
    let duration_seconds = if boss_count > 0 {
        boss_windows
            .iter()
            .map(|window| window.stats.duration_seconds)
            .sum()
    } else {
        observed_duration_seconds
    };
    let started_at = if boss_count > 0 {
        boss_windows
            .iter()
            .find_map(|window| window.stats.started_at)
    } else {
        observed_started_at
    };
    let ended_at = if boss_count > 0 {
        boss_windows
            .iter()
            .rev()
            .find_map(|window| window.stats.ended_at)
    } else {
        observed_ended_at
    };
    let metric_events: Vec<&GameEvent> = if boss_count > 0 {
        boss_events
    } else {
        events.iter().collect()
    };
    let mut player_events: BTreeMap<String, Vec<&GameEvent>> = BTreeMap::new();
    for event in &metric_events {
        if let Some(player) = event.player.as_ref() {
            player_events.entry(player.clone()).or_default().push(event);
        }
    }
    let mut players: Vec<_> = player_events
        .iter()
        .map(|(player, events)| summarize_player(player, events, duration_seconds))
        .collect();
    if boss_count > 0 {
        // Active time is participation within combat, not the wall-clock span between a player's
        // first hit on boss one and last hit on a later boss. Sum the already bounded per-boss
        // spans while retaining the shared total boss duration as every player's DPS denominator.
        for player in &mut players {
            player.active_seconds = boss_windows
                .iter()
                .filter_map(|window| {
                    window
                        .stats
                        .players
                        .iter()
                        .find(|candidate| candidate.player == player.player)
                })
                .map(|candidate| candidate.active_seconds)
                .sum::<f64>()
                .min(duration_seconds);
        }
    }
    let outgoing = if boss_count > 0 {
        summarize_damage_events(metric_events.iter().copied(), duration_seconds)
    } else {
        observed_outgoing.clone()
    };
    let incoming = if boss_count > 0 {
        summarize_incoming_events(metric_events.iter().copied(), duration_seconds)
    } else {
        observed_incoming.clone()
    };
    let attacks = summarize_attack_types(metric_events.iter().copied(), outgoing.total);
    let timeline = if boss_count > 0 {
        let mut combat_offset = 0.0;
        let mut timeline = Vec::new();
        for window in &boss_windows {
            timeline.extend(window.stats.timeline.iter().cloned().map(|mut point| {
                let local_elapsed = point.elapsed_seconds.unwrap_or_else(|| {
                    window
                        .stats
                        .started_at
                        .map(|start| seconds_between(start, point.timestamp))
                        .unwrap_or_default()
                });
                point.elapsed_seconds = Some(combat_offset + local_elapsed);
                point
            }));
            combat_offset += window.stats.duration_seconds;
        }
        timeline
    } else {
        summarize_timeline(metric_events.iter().copied(), started_at, ended_at)
    };

    RunSummary {
        id,
        session_id,
        started_at,
        ended_at,
        duration_seconds,
        metrics_scope: metrics_scope.to_owned(),
        boss_count,
        observed_started_at,
        observed_ended_at,
        observed_duration_seconds,
        observed_outgoing,
        observed_incoming,
        pre_boss_duration_seconds,
        pre_boss_outgoing,
        pre_boss_incoming,
        class_names: classes.into_iter().collect(),
        stages,
        players,
        encounters,
        outgoing,
        incoming,
        attacks,
        timeline,
        completed,
        event_count: events.len(),
        source_count: events
            .iter()
            .filter_map(|event| event.player.as_ref())
            .collect::<BTreeSet<_>>()
            .len()
            .max(1),
    }
}

fn summarize_player(player: &str, events: &[&GameEvent], observation_seconds: f64) -> PlayerStats {
    let first_combat = events
        .iter()
        .find(|event| {
            matches!(event.kind, EventKind::DamageDealt | EventKind::DamageTaken)
                || outgoing_healing_amount(event, player).is_some()
        })
        .map(|event| event.timestamp);
    let last_combat = events
        .iter()
        .rev()
        .find(|event| {
            matches!(event.kind, EventKind::DamageDealt | EventKind::DamageTaken)
                || outgoing_healing_amount(event, player).is_some()
        })
        .map(|event| event.timestamp);
    let active_seconds = first_combat
        .zip(last_combat)
        .map(|(start, end)| seconds_between(start, end).max(1.0))
        .unwrap_or_default()
        .min(observation_seconds);
    let mut damage = DamageTotals::default();
    let mut incoming = IncomingTotals::default();
    let mut healing = 0.0;
    let mut deaths = 0;
    for event in events {
        match event.kind {
            EventKind::DamageDealt => add_damage_total(&mut damage, event),
            EventKind::DamageTaken => add_incoming_total(&mut incoming, event),
            EventKind::Healing => {
                if let Some(amount) = outgoing_healing_amount(event, player) {
                    healing += amount;
                }
            }
            EventKind::PlayerDowned => deaths += 1,
            _ => {}
        }
    }
    // Player and combined DPS must share one observation window. Using a player's own
    // first-to-last event span makes a late join or a single hit look artificially dominant.
    damage.dps = if observation_seconds > 0.0 {
        damage.total / observation_seconds
    } else {
        0.0
    };
    incoming.damage_per_second = if observation_seconds > 0.0 {
        incoming.total / observation_seconds
    } else {
        0.0
    };
    let attacks = summarize_attack_types(events.iter().copied(), damage.total);
    PlayerStats {
        player: player.to_owned(),
        class_name: events
            .iter()
            .rev()
            .find_map(|event| event.class_name.clone()),
        damage,
        incoming,
        attacks,
        healing,
        active_seconds,
        deaths,
    }
}

#[derive(Debug)]
struct EncounterWindow {
    start: usize,
    effective_end: usize,
    stats: EncounterStats,
}

fn build_encounter_windows(events: &[GameEvent]) -> Vec<EncounterWindow> {
    let mut starts = Vec::new();
    let mut recent_boundaries: BTreeMap<String, NaiveDateTime> = BTreeMap::new();
    for (index, event) in events.iter().enumerate() {
        let Some(boundary) = encounter_boundary_key(event) else {
            continue;
        };
        let is_duplicate = recent_boundaries.get(&boundary).is_some_and(|seen_at| {
            (event.timestamp - *seen_at).num_seconds().unsigned_abs()
                <= MERGED_BOUNDARY_TOLERANCE_SECONDS as u64
        });
        recent_boundaries.insert(boundary, event.timestamp);
        if !is_duplicate {
            starts.push(index);
        }
    }

    let mut result = Vec::with_capacity(starts.len());
    for (position, start) in starts.iter().copied().enumerate() {
        let end = starts.get(position + 1).copied().unwrap_or(events.len());
        if start < end {
            let next_boundary = events.get(end);
            // A few phase transitions announce the next phase immediately before writing the
            // previous phase's named dead header. Look only through the peer-tolerance window so
            // delayed buffered repeats cannot rewrite a much earlier structural boundary.
            let late_named_end = if events[start].kind == EventKind::BossStarted {
                let boss = events[start].boss.as_deref().unwrap_or_default();
                next_boundary.and_then(|boundary| {
                    let deadline = boundary.timestamp
                        + chrono::Duration::seconds(MERGED_BOUNDARY_TOLERANCE_SECONDS);
                    events[end..]
                        .iter()
                        .take_while(|event| event.timestamp <= deadline)
                        .find(|event| {
                            matches!(
                                event.kind,
                                EventKind::BossDefeated | EventKind::BossDamageSummary
                            ) && event
                                .boss
                                .as_deref()
                                .is_some_and(|candidate| entity_key(candidate) == entity_key(boss))
                        })
                })
            } else {
                None
            };
            let (stats, effective_len) = summarize_encounter_with_boundary(
                &events[start..end],
                next_boundary,
                late_named_end,
            );
            result.push(EncounterWindow {
                start,
                effective_end: start + effective_len,
                stats,
            });
        }
    }
    result
}

fn encounter_boundary_key(event: &GameEvent) -> Option<String> {
    match event.kind {
        EventKind::StageEntered => event.stage.as_ref().map(|stage| format!("stage:{stage}")),
        EventKind::BossStarted => event.boss.as_ref().map(|boss| format!("boss:{boss}")),
        _ => None,
    }
}

#[cfg(test)]
fn summarize_encounter(events: &[GameEvent]) -> EncounterStats {
    summarize_encounter_with_boundary(events, None, None).0
}

fn summarize_encounter_with_boundary(
    events: &[GameEvent],
    next_boundary: Option<&GameEvent>,
    late_named_end: Option<&GameEvent>,
) -> (EncounterStats, usize) {
    debug_assert!(!events.is_empty());
    let first = &events[0];
    let kind = if first.kind == EventKind::BossStarted {
        "boss"
    } else {
        "pre_boss"
    };
    let name = first
        .boss
        .clone()
        .or_else(|| first.stage.clone())
        .unwrap_or_else(|| "Encounter".to_owned());

    let local_structural_end = events
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(index, event)| match event.kind {
            EventKind::Intermission => Some((index, event.timestamp, "intermission", "structural")),
            EventKind::Lobby => Some((index, event.timestamp, "lobby", "structural")),
            EventKind::WorldExited => Some((index, event.timestamp, "world_exit", "structural")),
            _ => None,
        });
    let boss_key = entity_key(&name);
    let local_named_end = events
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(index, event)| {
            let matching_named_boss_end = kind == "boss"
                && matches!(
                    event.kind,
                    EventKind::BossDefeated | EventKind::BossDamageSummary
                )
                && event
                    .boss
                    .as_deref()
                    .is_some_and(|boss| entity_key(boss) == boss_key);
            if !matching_named_boss_end {
                return None;
            }

            // Buffered completion lines can legitimately trail intermission by a few seconds.
            // Lobby and world-exit are hard run boundaries, though, so never resurrect a
            // named record logged after either of them (even at the same wall-clock second).
            let within_valid_window =
                local_structural_end.is_none_or(|(structural_index, structural_at, reason, _)| {
                    match reason {
                        "intermission" => {
                            event.timestamp
                                <= structural_at
                                    + chrono::Duration::seconds(MERGED_BOUNDARY_TOLERANCE_SECONDS)
                        }
                        _ => index < structural_index,
                    }
                });
            if !within_valid_window {
                return None;
            }

            let reason = if event.kind == EventKind::BossDefeated {
                "boss_defeated"
            } else {
                // Databases created by v0.1 stored the summary even when a zero total
                // suppressed BossDefeated. A named summary still proves the dead header
                // immediately preceded it and upgrades old history without a re-import.
                "boss_summary"
            };
            Some((event.timestamp, reason, "explicit"))
        });
    let late_explicit_end = late_named_end.and_then(|event| {
        let within_valid_window =
            local_structural_end.is_none_or(|(_, structural_at, reason, _)| {
                reason == "intermission"
                    && event.timestamp
                        <= structural_at
                            + chrono::Duration::seconds(MERGED_BOUNDARY_TOLERANCE_SECONDS)
            });
        if !within_valid_window {
            return None;
        }
        let reason = if event.kind == EventKind::BossDefeated {
            "boss_defeated"
        } else {
            "boss_summary"
        };
        Some((event.timestamp, reason, "explicit"))
    });
    let named_end = local_named_end.or(late_explicit_end);
    let next_structural_end = next_boundary.map(|event| {
        let reason = match event.kind {
            EventKind::BossStarted if kind == "pre_boss" => "boss_started",
            EventKind::BossStarted => "next_boss",
            EventKind::StageEntered => "next_stage",
            _ => "next_boundary",
        };
        (event.timestamp, reason, "structural")
    });
    let local_structural_boundary =
        local_structural_end.map(|(_, at, reason, confidence)| (at, reason, confidence));
    let accepted_local_named_at = local_named_end.map(|(at, _, _)| at);
    let boundary = named_end
        .or(local_structural_boundary)
        .or(next_structural_end);
    let boundary_is_local = local_named_end.is_some()
        || (late_explicit_end.is_none() && local_structural_end.is_some());
    // A named completion buffered until after phase N+1 starts is stronger provenance than the
    // structural transition, but it must not extend phase N into phase N+1's combat window.
    let clamped_late_named_at = if local_named_end.is_none() {
        late_explicit_end.and_then(|(late_at, _, _)| {
            next_structural_end.map(|(next_at, _, _)| late_at.min(next_at))
        })
    } else {
        None
    };
    let first_local_completion = local_named_end
        .map(|(at, _, _)| at)
        .into_iter()
        .chain(local_structural_end.map(|(_, at, _, _)| at))
        .min();
    let effective_len = first_local_completion
        .map(|completed_at| {
            let deadline =
                completed_at + chrono::Duration::seconds(MERGED_BOUNDARY_TOLERANCE_SECONDS);
            events
                .iter()
                .take_while(|event| event.timestamp <= deadline)
                .count()
                .max(1)
        })
        .unwrap_or(events.len());
    let events = &events[..effective_len];
    let started_at = Some(first.timestamp);
    let ended_at = boundary
        .map(|(completed_at, _, _)| {
            if !boundary_is_local {
                return clamped_late_named_at.unwrap_or(completed_at);
            }
            events
                .iter()
                .filter(|event| {
                    matches!(event.kind, EventKind::DamageDealt | EventKind::DamageTaken)
                        || matches!(
                            event.kind,
                            EventKind::Intermission | EventKind::Lobby | EventKind::WorldExited
                        )
                        || (matches!(
                            event.kind,
                            EventKind::BossDefeated | EventKind::BossDamageSummary
                        ) && accepted_local_named_at == Some(event.timestamp)
                            && event
                                .boss
                                .as_deref()
                                .is_some_and(|boss| entity_key(boss) == entity_key(&name)))
                })
                .map(|event| event.timestamp)
                .max()
                .unwrap_or(completed_at)
        })
        .or_else(|| {
            events
                .iter()
                .rev()
                .find(|event| matches!(event.kind, EventKind::DamageDealt | EventKind::DamageTaken))
                .or_else(|| events.last())
                .map(|event| event.timestamp)
        });
    let duration_seconds = started_at
        .zip(ended_at)
        .map(|(start, end)| seconds_between(start, end).max(1.0))
        .unwrap_or_default();
    let mut player_events: BTreeMap<String, Vec<&GameEvent>> = BTreeMap::new();
    for event in events {
        if let Some(player) = event.player.as_ref() {
            player_events.entry(player.clone()).or_default().push(event);
        }
    }
    let players: Vec<_> = player_events
        .iter()
        .map(|(player, events)| summarize_player(player, events, duration_seconds))
        .collect();
    let outgoing = summarize_damage_events(events.iter(), duration_seconds);
    let incoming = summarize_incoming_events(events.iter(), duration_seconds);
    let attacks = summarize_attack_types(events.iter(), outgoing.total);
    let timeline = summarize_timeline(events.iter(), started_at, ended_at);

    let stats = EncounterStats {
        id: format!("{}-{}", first.timestamp.format("%Y%m%d%H%M%S"), slug(&name)),
        name,
        kind: kind.to_owned(),
        stage: first.stage.clone(),
        phase: first.phase,
        started_at,
        ended_at,
        duration_seconds,
        players,
        outgoing,
        incoming,
        attacks,
        timeline,
        completed: boundary.is_some(),
        end_reason: boundary
            .map(|(_, reason, _)| reason)
            .unwrap_or("open")
            .to_owned(),
        boundary_confidence: boundary
            .map(|(_, _, confidence)| confidence)
            .unwrap_or("open")
            .to_owned(),
    };
    (stats, effective_len)
}

fn summarize_attack_types<'a>(
    events: impl IntoIterator<Item = &'a GameEvent>,
    outgoing_total: f64,
) -> Vec<AttackStats> {
    let mut damage_types: BTreeMap<String, (f64, u64, f64, f64)> = BTreeMap::new();
    for event in events {
        if event.kind != EventKind::DamageDealt {
            continue;
        }
        let amount = event.amount();
        if amount <= 0.0 {
            continue;
        }
        let damage_type = event
            .damage_type
            .clone()
            .unwrap_or_else(|| "unknown".to_owned());
        let entry = damage_types
            .entry(damage_type)
            .or_insert((0.0, 0, f64::MAX, 0.0));
        entry.0 += amount;
        entry.1 += 1;
        entry.2 = entry.2.min(amount);
        entry.3 = entry.3.max(amount);
    }

    let mut attacks: Vec<_> = damage_types
        .into_iter()
        .map(|(damage_type, (total, hits, min, max))| AttackStats {
            name: friendly_damage_type(&damage_type),
            damage_type,
            total,
            hits,
            min: if min == f64::MAX { 0.0 } else { min },
            max,
            average: if hits == 0 { 0.0 } else { total / hits as f64 },
            share: if outgoing_total <= 0.0 {
                0.0
            } else {
                total / outgoing_total
            },
        })
        .collect();
    attacks.sort_by(|a, b| b.total.total_cmp(&a.total));
    attacks
}

fn summarize_timeline<'a>(
    events: impl IntoIterator<Item = &'a GameEvent>,
    observation_start: Option<NaiveDateTime>,
    observation_end: Option<NaiveDateTime>,
) -> Vec<TimelinePoint> {
    let floor_second = |timestamp: NaiveDateTime| timestamp.with_nanosecond(0).unwrap_or(timestamp);
    let is_observed = |timestamp: NaiveDateTime| {
        observation_start.is_none_or(|start| timestamp >= start)
            && observation_end.is_none_or(|end| timestamp <= end)
    };

    let mut buckets: BTreeMap<NaiveDateTime, (f64, f64)> = BTreeMap::new();
    for event in events {
        if !matches!(event.kind, EventKind::DamageDealt | EventKind::DamageTaken) {
            continue;
        }
        let amount = event.amount();
        if amount <= 0.0 {
            continue;
        }
        if !is_observed(event.timestamp) {
            continue;
        }
        // Damage is bucketed to whole seconds, except at a fractional observation boundary where
        // flooring would incorrectly place the first bucket before the run began.
        let mut timestamp = floor_second(event.timestamp);
        if let Some(start) = observation_start {
            timestamp = timestamp.max(start);
        }
        if let Some(end) = observation_end {
            timestamp = timestamp.min(end);
        }
        let bucket = buckets.entry(timestamp).or_default();
        match event.kind {
            EventKind::DamageDealt => bucket.0 += amount,
            EventKind::DamageTaken => bucket.1 += amount,
            _ => unreachable!("non-damage events were filtered above"),
        }
    }

    // A point only at each damage second can draw a non-zero value straight across a long idle
    // gap. Keep the series sparse, but add the exact points where a five-second contribution is
    // last present and where it expires. One-second shoulders keep the per-second outgoing and
    // incoming buckets from being interpolated across an otherwise empty gap.
    let mut timestamps = BTreeSet::new();
    timestamps.extend(observation_start);
    timestamps.extend(observation_end);
    for (&timestamp, &(outgoing, _)) in &buckets {
        timestamps.insert(timestamp);
        for offset_seconds in [-1, 1] {
            if let Some(anchor) =
                timestamp.checked_add_signed(chrono::Duration::seconds(offset_seconds))
            {
                if is_observed(anchor) {
                    timestamps.insert(anchor);
                }
            }
        }
        if outgoing > 0.0 {
            for offset_seconds in [5, 6] {
                if let Some(anchor) =
                    timestamp.checked_add_signed(chrono::Duration::seconds(offset_seconds))
                {
                    if is_observed(anchor) {
                        timestamps.insert(anchor);
                    }
                }
            }
        }
    }

    let mut result = Vec::with_capacity(timestamps.len());
    let mut outgoing_window: VecDeque<(NaiveDateTime, f64)> = VecDeque::new();
    let mut rolling_outgoing = 0.0;
    for timestamp in timestamps {
        while outgoing_window
            .front()
            .is_some_and(|(observed_at, _)| (timestamp - *observed_at).num_seconds() > 5)
        {
            if let Some((_, expired)) = outgoing_window.pop_front() {
                rolling_outgoing -= expired;
            }
        }
        let (outgoing, incoming) = buckets.get(&timestamp).copied().unwrap_or_default();
        if outgoing > 0.0 {
            outgoing_window.push_back((timestamp, outgoing));
            rolling_outgoing += outgoing;
        }
        result.push(TimelinePoint {
            timestamp,
            elapsed_seconds: observation_start.map(|start| seconds_between(start, timestamp)),
            outgoing,
            incoming,
            rolling_dps: rolling_outgoing.max(0.0) / 5.0,
        });
    }
    result
}

fn add_damage_total(total: &mut DamageTotals, event: &GameEvent) {
    let amount = event.amount();
    if amount <= 0.0 {
        return;
    }
    total.total += amount;
    total.hits += 1;
    total.biggest_hit = total.biggest_hit.max(amount);
    if event.damage_type.as_deref() == Some("strike") {
        total.strike += amount;
    } else {
        total.non_strike += amount;
    }
}

fn add_incoming_total(total: &mut IncomingTotals, event: &GameEvent) {
    let amount = event.amount();
    if amount <= 0.0 {
        return;
    }
    total.total += amount;
    total.hits += 1;
    total.biggest_hit = total.biggest_hit.max(amount);
    *total
        .by_source
        .entry(event.source.clone().unwrap_or_else(|| "unknown".to_owned()))
        .or_default() += amount;
}

fn summarize_damage_events<'a>(
    events: impl IntoIterator<Item = &'a GameEvent>,
    seconds: f64,
) -> DamageTotals {
    let mut result = DamageTotals::default();
    for event in events {
        if event.kind == EventKind::DamageDealt {
            add_damage_total(&mut result, event);
        }
    }
    if seconds > 0.0 {
        result.dps = result.total / seconds;
    }
    result
}

fn summarize_incoming_events<'a>(
    events: impl IntoIterator<Item = &'a GameEvent>,
    seconds: f64,
) -> IncomingTotals {
    let mut result = IncomingTotals::default();
    for event in events {
        if event.kind == EventKind::DamageTaken {
            add_incoming_total(&mut result, event);
        }
    }
    if seconds > 0.0 {
        result.damage_per_second = result.total / seconds;
    }
    result
}

fn outgoing_healing_amount(event: &GameEvent, healer: &str) -> Option<f64> {
    let amount = event.amount();
    if event.kind != EventKind::Healing || !amount.is_finite() || amount <= 0.0 {
        return None;
    }
    let target = event.target.as_deref()?.trim();
    (!target.is_empty() && target != healer.trim()).then_some(amount)
}

fn rolling_sum(
    values: &VecDeque<(NaiveDateTime, f64)>,
    now: Option<NaiveDateTime>,
    seconds: i64,
) -> f64 {
    let Some(now) = now else {
        return 0.0;
    };
    values
        .iter()
        .filter(|(timestamp, _)| (now - *timestamp).num_seconds() <= seconds)
        .map(|(_, amount)| amount)
        .sum()
}

fn seconds_between(start: NaiveDateTime, end: NaiveDateTime) -> f64 {
    (end - start).num_milliseconds().max(0) as f64 / 1000.0
}

fn friendly_damage_type(value: &str) -> String {
    match value {
        "strike" => "Strike".to_owned(),
        "non-strike" | "non_strike" => "Non-strike".to_owned(),
        other => other.replace(['_', '-'], " "),
    }
}

fn slug(value: &str) -> String {
    value
        .chars()
        .filter_map(|character| {
            if character.is_ascii_alphanumeric() {
                Some(character.to_ascii_lowercase())
            } else if character.is_whitespace() || character == '_' || character == '-' {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}

fn entity_key(value: &str) -> String {
    value
        .trim_end_matches("(Clone)")
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn valid_run_progress(progress: Option<f64>) -> Option<f64> {
    progress.filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
}

// Ecliptica logs the numeric value but not these names. These half-open bands implement the
// documented third-party EACT convention; they are a presentation mapping, not a world API.
fn phase_name_for_progress(progress: f64) -> Option<&'static str> {
    match progress {
        value if (0.0..0.2).contains(&value) => Some("PRIME"),
        value if (0.2..0.4).contains(&value) => Some("PENUMBRA"),
        value if (0.4..0.6).contains(&value) => Some("ANTUMBRA"),
        value if (0.6..0.8).contains(&value) => Some("UMBRA"),
        value if (0.8..=1.0).contains(&value) => Some("ECLIPSE"),
        _ => None,
    }
}

fn infer_boss_number(progress: f64) -> Option<u8> {
    valid_run_progress(Some(progress)).and_then(|progress| {
        BOSS_PROGRESS_ANCHORS
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| {
                (progress - **left)
                    .abs()
                    .total_cmp(&(progress - **right).abs())
            })
            .map(|(index, _)| (index + 1) as u8)
    })
}

fn is_bringer_stage(stage: &str) -> bool {
    entity_key(stage) == "stagebringer"
}

fn parse_boss_subphase(boss: &str) -> Option<u8> {
    let normalized = boss.trim().trim_end_matches("(Clone)");
    let lowercase = normalized.to_ascii_lowercase();
    let phase_at = lowercase.rfind("phase")?;
    let suffix = &normalized[phase_at + "phase".len()..];
    (!suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_digit()))
        .then(|| suffix.parse::<u8>().ok().filter(|value| *value > 0))
        .flatten()
}

fn capability_note() -> String {
    "VRChat logs expose this client's outgoing hits and incoming damage. Party DPS is available when each player's collector data is imported or connected; ownership-transfer lines are never treated as damage attribution.".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn event(second: u32, kind: EventKind, amount: f64) -> GameEvent {
        GameEvent {
            sequence: second as u64,
            timestamp: NaiveDate::from_ymd_opt(2026, 7, 21)
                .unwrap()
                .and_hms_opt(20, 0, second)
                .unwrap(),
            kind,
            player: Some("PlayerOne".to_owned()),
            session_id: Some(42),
            world: Some("Ecliptica".to_owned()),
            instance: Some("wrld_test:1".to_owned()),
            stage: Some("Stage_Test".to_owned()),
            class_name: Some("Spellhammer".to_owned()),
            boss: Some("TestBoss".to_owned()),
            phase: Some(0.0),
            amount: Some(amount),
            damage_type: Some("strike".to_owned()),
            source: None,
            target: None,
            entity: None,
            pool_id: None,
            enemy_id: None,
            user_id: None,
            message: None,
            raw: String::new(),
        }
    }

    fn visit_event(second: u32, kind: EventKind, session_id: Option<u32>) -> GameEvent {
        let mut value = event(second, kind, 0.0);
        value.session_id = session_id;
        value.stage = None;
        value.class_name = None;
        value.boss = None;
        value.phase = None;
        value.amount = None;
        value.damage_type = None;
        value
    }

    fn stage_event(second: u32, stage: &str, progress: f64) -> GameEvent {
        let mut value = event(second, EventKind::StageEntered, 0.0);
        value.stage = Some(stage.to_owned());
        value.boss = None;
        value.phase = Some(progress);
        value
    }

    #[test]
    fn run_phase_names_use_documented_third_party_progress_bands() {
        assert_eq!(phase_name_for_progress(0.0), Some("PRIME"));
        assert_eq!(phase_name_for_progress(0.199_999), Some("PRIME"));
        assert_eq!(phase_name_for_progress(0.2), Some("PENUMBRA"));
        assert_eq!(phase_name_for_progress(0.399_999), Some("PENUMBRA"));
        assert_eq!(phase_name_for_progress(0.4), Some("ANTUMBRA"));
        assert_eq!(phase_name_for_progress(0.599_999), Some("ANTUMBRA"));
        assert_eq!(phase_name_for_progress(0.6), Some("UMBRA"));
        assert_eq!(phase_name_for_progress(0.799_999), Some("UMBRA"));
        assert_eq!(phase_name_for_progress(0.8), Some("ECLIPSE"));
        assert_eq!(phase_name_for_progress(1.0), Some("ECLIPSE"));
        assert_eq!(phase_name_for_progress(-0.01), None);
        assert_eq!(phase_name_for_progress(1.01), None);
    }

    #[test]
    fn stage_entries_count_all_thirteen_bosses_without_counting_boss_subphases() {
        let mut engine = CombatEngine::new();
        for (index, progress) in BOSS_PROGRESS_ANCHORS.iter().copied().enumerate() {
            let stage = if index == 12 {
                "Stage_Bringer".to_owned()
            } else {
                format!("Stage_{index}")
            };
            let stage = stage_event(index as u32, &stage, progress);
            engine.ingest(stage.clone());
            // A repeated stage announcement is a network duplicate, not another boss.
            engine.ingest(stage);

            let context = engine.snapshot().run_context;
            assert_eq!(context.progress, Some(progress));
            assert_eq!(context.boss_number, Some((index + 1) as u8));
            assert!(!context.boss_number_inferred);
        }

        let mut phase_two = event(20, EventKind::BossStarted, 0.0);
        phase_two.stage = Some("Stage_Bringer".to_owned());
        phase_two.boss = Some("JimBringerPhase2".to_owned());
        // A pre-sync replay observed in the audited log reports zero here. It must not roll the
        // already-established final-boss context back to PRIME.
        phase_two.phase = Some(0.0);
        engine.ingest(phase_two.clone());
        engine.ingest(phase_two);
        let phase_two_context = engine.snapshot().run_context;
        assert_eq!(phase_two_context.progress, Some(1.0));
        assert_eq!(phase_two_context.phase_name.as_deref(), Some("ECLIPSE"));
        assert_eq!(phase_two_context.boss_number, Some(13));
        assert_eq!(phase_two_context.boss_subphase, Some(2));
        assert!(!phase_two_context.boss_number_inferred);

        let mut phase_three = event(21, EventKind::BossStarted, 0.0);
        phase_three.stage = Some("Stage_Bringer".to_owned());
        phase_three.boss = Some("JimBringerPhase3(Clone)".to_owned());
        phase_three.phase = Some(1.0);
        engine.ingest(phase_three);
        let phase_three_context = engine.snapshot().run_context;
        assert_eq!(phase_three_context.boss_number, Some(13));
        assert_eq!(phase_three_context.boss_subphase, Some(3));
    }

    #[test]
    fn delayed_duplicate_stage_does_not_replace_a_boss_or_reset_its_damage() {
        let mut engine = CombatEngine::new();
        let stage = stage_event(1, "Stage_First", 0.0);
        engine.ingest(stage.clone());

        let mut boss = event(2, EventKind::BossStarted, 0.0);
        boss.stage = Some("Stage_First".to_owned());
        boss.boss = Some("FirstBoss".to_owned());
        boss.phase = Some(0.0);
        engine.ingest(boss);
        let mut hit = event(3, EventKind::DamageDealt, 450.0);
        hit.stage = Some("Stage_First".to_owned());
        engine.ingest(hit);
        let before_duplicate = engine.snapshot();

        let mut delayed_stage = stage;
        delayed_stage.timestamp = event(4, EventKind::StageEntered, 0.0).timestamp;
        engine.ingest(delayed_stage);
        let after_duplicate = engine.snapshot();

        assert_eq!(after_duplicate.stage.as_deref(), Some("Stage_First"));
        assert_eq!(after_duplicate.encounter.name, "FirstBoss");
        assert_eq!(after_duplicate.encounter.kind, "boss");
        assert_eq!(
            after_duplicate.encounter.started_at,
            before_duplicate.encounter.started_at
        );
        assert_eq!(after_duplicate.outgoing.total, 450.0);
        assert_eq!(after_duplicate.outgoing.hits, 1);
        assert_eq!(after_duplicate.run_context.boss_number, Some(1));
    }

    #[test]
    fn delayed_earlier_stage_does_not_roll_back_an_active_boss() {
        let mut engine = CombatEngine::new();
        for (index, progress) in BOSS_PROGRESS_ANCHORS.iter().copied().enumerate().take(10) {
            engine.ingest(stage_event(
                index as u32,
                &format!("Stage_{index}"),
                progress,
            ));
        }

        let mut boss = event(10, EventKind::BossStarted, 0.0);
        boss.stage = Some("Stage_9".to_owned());
        boss.boss = Some("CurrentBoss".to_owned());
        boss.phase = Some(BOSS_PROGRESS_ANCHORS[9]);
        engine.ingest(boss);

        let mut ownership = event(11, EventKind::OwnershipTransferred, 0.0);
        ownership.stage = Some("Stage_9".to_owned());
        ownership.phase = Some(BOSS_PROGRESS_ANCHORS[9]);
        ownership.entity = Some("CurrentBoss(Clone)".to_owned());
        ownership.target = Some("PlayerTwo".to_owned());
        engine.ingest(ownership);

        let mut hit = event(12, EventKind::DamageDealt, 725.0);
        hit.stage = Some("Stage_9".to_owned());
        hit.phase = Some(BOSS_PROGRESS_ANCHORS[9]);
        engine.ingest(hit);
        let before = engine.snapshot();

        let mut delayed = stage_event(13, "Stage_4", BOSS_PROGRESS_ANCHORS[4]);
        delayed.session_id = Some(42);
        engine.ingest(delayed);
        let after = engine.snapshot();

        assert_eq!(after.run_context, before.run_context);
        assert_eq!(after.stage, before.stage);
        assert_eq!(after.encounter.name, before.encounter.name);
        assert_eq!(after.encounter.kind, before.encounter.kind);
        assert_eq!(after.encounter.phase, before.encounter.phase);
        assert_eq!(after.encounter.started_at, before.encounter.started_at);
        assert_eq!(after.encounter.active, before.encounter.active);
        assert_eq!(after.outgoing.total, 725.0);
        assert_eq!(after.outgoing.hits, 1);
        let before_focus = before
            .focus
            .expect("current boss should have target evidence");
        let after_focus = after
            .focus
            .expect("delayed stage must preserve target evidence");
        assert_eq!(after_focus.player, before_focus.player);
        assert_eq!(after_focus.entity, before_focus.entity);
        assert_eq!(after_focus.observed_at, before_focus.observed_at);
        assert_eq!(after_focus.evidence, before_focus.evidence);
        assert_eq!(after_focus.confidence, before_focus.confidence);
        assert_eq!(
            after_focus.corroborating_hits,
            before_focus.corroborating_hits
        );
    }

    #[test]
    fn partial_run_uses_calibrated_boss_anchor_and_keeps_inference_label() {
        let mut engine = CombatEngine::new();
        let eighth = stage_event(1, "Stage_MidLog", BOSS_PROGRESS_ANCHORS[7]);
        engine.ingest(eighth.clone());
        engine.ingest(eighth);
        let recovered = engine.snapshot().run_context;
        assert_eq!(recovered.boss_number, Some(8));
        assert!(recovered.boss_number_inferred);
        assert_eq!(recovered.phase_name.as_deref(), Some("ANTUMBRA"));

        let mut phase_two = event(2, EventKind::BossStarted, 0.0);
        phase_two.stage = Some("Stage_MidLog".to_owned());
        phase_two.boss = Some("RecoveredBossPhase2".to_owned());
        phase_two.phase = Some(BOSS_PROGRESS_ANCHORS[7]);
        engine.ingest(phase_two);
        assert_eq!(engine.snapshot().run_context.boss_number, Some(8));
        assert_eq!(engine.snapshot().run_context.boss_subphase, Some(2));

        engine.ingest(stage_event(
            3,
            "Stage_AfterReconnect",
            BOSS_PROGRESS_ANCHORS[8],
        ));
        let counted = engine.snapshot().run_context;
        assert_eq!(counted.boss_number, Some(9));
        assert!(counted.boss_number_inferred);
        assert_eq!(counted.phase_name.as_deref(), Some("UMBRA"));
    }

    #[test]
    fn boss_only_mid_log_recovery_is_inferred_without_incrementing_multiphase() {
        let mut engine = CombatEngine::new();
        let mut boss = event(1, EventKind::BossStarted, 0.0);
        boss.stage = Some("Stage_Unknown".to_owned());
        boss.boss = Some("Corus".to_owned());
        boss.phase = Some(BOSS_PROGRESS_ANCHORS[9]);
        engine.ingest(boss);
        assert_eq!(engine.snapshot().run_context.boss_number, Some(10));
        assert!(engine.snapshot().run_context.boss_number_inferred);

        // A delayed copy of the same stage header must not turn the current boss into #11.
        engine.ingest(stage_event(2, "Stage_Unknown", BOSS_PROGRESS_ANCHORS[9]));
        assert_eq!(engine.snapshot().run_context.boss_number, Some(10));

        let mut phase_two = event(3, EventKind::BossStarted, 0.0);
        phase_two.stage = Some("Stage_Unknown".to_owned());
        phase_two.boss = Some("CorusPhase2".to_owned());
        phase_two.phase = Some(BOSS_PROGRESS_ANCHORS[9]);
        engine.ingest(phase_two);
        let context = engine.snapshot().run_context;
        assert_eq!(context.boss_number, Some(10));
        assert_eq!(context.boss_subphase, Some(2));
        assert!(context.boss_number_inferred);

        let mut ownership = event(4, EventKind::OwnershipTransferred, 0.0);
        ownership.stage = Some("Stage_Unknown".to_owned());
        ownership.phase = Some(BOSS_PROGRESS_ANCHORS[9]);
        ownership.entity = Some("CorusPhase2(Clone)".to_owned());
        ownership.target = Some("PlayerTwo".to_owned());
        engine.ingest(ownership);

        let mut hit = event(5, EventKind::DamageDealt, 320.0);
        hit.stage = Some("Stage_Unknown".to_owned());
        hit.phase = Some(BOSS_PROGRESS_ANCHORS[9]);
        engine.ingest(hit);
        let before_stale = engine.snapshot();
        assert_eq!(before_stale.encounter.name, "CorusPhase2");
        assert_eq!(before_stale.outgoing.total, 320.0);
        assert_eq!(before_stale.focus.as_ref().unwrap().player, "PlayerTwo");

        let mut stale_phase = event(6, EventKind::BossStarted, 0.0);
        stale_phase.stage = Some("Stage_Older".to_owned());
        stale_phase.boss = Some("OlderBossPhase3".to_owned());
        stale_phase.phase = Some(BOSS_PROGRESS_ANCHORS[4]);
        engine.ingest(stale_phase);
        let after_stale = engine.snapshot();
        assert_eq!(
            after_stale.run_context.progress,
            Some(BOSS_PROGRESS_ANCHORS[9])
        );
        assert_eq!(after_stale.run_context.boss_number, Some(10));
        assert_eq!(after_stale.run_context.boss_subphase, Some(2));
        assert_eq!(after_stale.stage.as_deref(), Some("Stage_Unknown"));
        assert_eq!(after_stale.encounter.name, "CorusPhase2");
        assert_eq!(
            after_stale.encounter.started_at,
            before_stale.encounter.started_at
        );
        assert_eq!(after_stale.outgoing.total, 320.0);
        assert_eq!(after_stale.outgoing.hits, 1);
        let focus = after_stale
            .focus
            .expect("stale start must preserve target proxy");
        assert_eq!(focus.player, "PlayerTwo");
        assert_eq!(focus.entity, "CorusPhase2(Clone)");
    }

    #[test]
    fn new_session_boss_only_recovery_accepts_lower_progress() {
        let mut engine = CombatEngine::new();
        let mut prior_stage = stage_event(1, "Stage_Prior", BOSS_PROGRESS_ANCHORS[10]);
        prior_stage.session_id = Some(42);
        engine.ingest(prior_stage);

        let mut prior_boss = event(2, EventKind::BossStarted, 0.0);
        prior_boss.stage = Some("Stage_Prior".to_owned());
        prior_boss.boss = Some("PriorBoss".to_owned());
        prior_boss.phase = Some(BOSS_PROGRESS_ANCHORS[10]);
        prior_boss.session_id = Some(42);
        engine.ingest(prior_boss);
        engine.ingest(event(3, EventKind::DamageDealt, 500.0));

        let mut new_session_boss = event(4, EventKind::BossStarted, 0.0);
        new_session_boss.stage = Some("Stage_NewRun".to_owned());
        new_session_boss.boss = Some("NewRunBoss".to_owned());
        new_session_boss.phase = Some(0.0);
        new_session_boss.session_id = Some(77);
        engine.ingest(new_session_boss);

        let recovered = engine.snapshot();
        assert_eq!(recovered.session_id, Some(77));
        assert_eq!(recovered.stage.as_deref(), Some("Stage_NewRun"));
        assert_eq!(recovered.encounter.name, "NewRunBoss");
        assert_eq!(recovered.outgoing.total, 0.0);
        assert_eq!(recovered.run_context.progress, Some(0.0));
        assert_eq!(recovered.run_context.boss_number, Some(1));
        assert!(recovered.run_context.boss_number_inferred);
    }

    #[test]
    fn jimbringer_name_is_an_authoritative_final_boss_marker() {
        let mut engine = CombatEngine::new();
        let mut boss = event(1, EventKind::BossStarted, 0.0);
        boss.stage = None;
        boss.boss = Some("JimBringerPhase2".to_owned());
        // Audited late/replayed Bringer lines can carry zero despite being the final stage.
        boss.phase = Some(0.0);
        engine.ingest(boss);

        let context = engine.snapshot().run_context;
        assert_eq!(context.progress, Some(1.0));
        assert_eq!(context.phase_name.as_deref(), Some("ECLIPSE"));
        assert_eq!(context.boss_number, Some(13));
        assert!(!context.boss_number_inferred);
        assert_eq!(context.boss_subphase, Some(2));
    }

    #[test]
    fn run_progress_rollback_starts_a_fresh_exact_count() {
        let mut engine = CombatEngine::new();
        engine.ingest(stage_event(1, "Stage_Late", BOSS_PROGRESS_ANCHORS[8]));
        assert_eq!(engine.snapshot().run_context.boss_number, Some(9));
        assert!(engine.snapshot().run_context.boss_number_inferred);

        engine.ingest(stage_event(2, "Stage_Hall of Beginnings", 0.0));
        let restarted = engine.snapshot().run_context;
        assert_eq!(restarted.progress, Some(0.0));
        assert_eq!(restarted.phase_name.as_deref(), Some("PRIME"));
        assert_eq!(restarted.boss_number, Some(1));
        assert!(!restarted.boss_number_inferred);

        engine.ingest(stage_event(3, "Stage_Second", BOSS_PROGRESS_ANCHORS[1]));
        let second = engine.snapshot().run_context;
        assert_eq!(second.boss_number, Some(2));
        assert!(!second.boss_number_inferred);
    }

    #[test]
    fn lobby_and_world_boundaries_clear_run_context_while_intermission_retains_progress() {
        let mut engine = CombatEngine::new();
        engine.ingest(stage_event(1, "Stage_Test", BOSS_PROGRESS_ANCHORS[5]));
        let mut phase_two = event(2, EventKind::BossStarted, 0.0);
        phase_two.boss = Some("TestBossPhase2".to_owned());
        phase_two.phase = Some(BOSS_PROGRESS_ANCHORS[5]);
        engine.ingest(phase_two);
        engine.ingest(event(3, EventKind::Intermission, 0.0));
        let intermission = engine.snapshot().run_context;
        assert_eq!(intermission.boss_number, Some(6));
        assert_eq!(intermission.progress, Some(BOSS_PROGRESS_ANCHORS[5]));
        assert_eq!(intermission.boss_subphase, None);

        engine.ingest(visit_event(4, EventKind::SessionSaved, Some(42)));
        let saved = engine.snapshot();
        assert_eq!(saved.run_context.boss_number, Some(6));
        assert!(!saved.loadout.available);
        assert!(saved.loadout.items.is_empty());
        assert!(saved.loadout.source_note.contains("does not expose"));

        engine.ingest(event(5, EventKind::Lobby, 0.0));
        assert_eq!(engine.snapshot().run_context, RunContext::default());

        engine.ingest(stage_event(6, "Stage_New", 0.0));
        engine.ingest(event(7, EventKind::WorldExited, 0.0));
        assert_eq!(engine.snapshot().run_context, RunContext::default());

        engine.ingest(stage_event(8, "Stage_Recovered", BOSS_PROGRESS_ANCHORS[3]));
        engine.ingest(visit_event(9, EventKind::WorldEntered, None));
        assert_eq!(engine.snapshot().run_context, RunContext::default());
    }

    #[test]
    fn additive_snapshot_fields_deserialize_from_older_payloads() {
        let mut value = serde_json::to_value(EngineSnapshot::default()).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("run_context");
        object.remove("loadout");

        let decoded: EngineSnapshot = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.run_context, RunContext::default());
        assert_eq!(decoded.loadout, LoadoutState::default());
    }

    #[test]
    fn computes_live_dps_windows() {
        let mut engine = CombatEngine::new();
        engine.ingest(event(0, EventKind::BossStarted, 0.0));
        engine.ingest(event(1, EventKind::DamageDealt, 100.0));
        engine.ingest(event(6, EventKind::DamageDealt, 200.0));
        let snapshot = engine.snapshot();
        assert_eq!(snapshot.outgoing.total, 300.0);
        assert_eq!(snapshot.outgoing.hits, 2);
        assert_eq!(snapshot.outgoing.rolling_5s, 60.0);
        assert_eq!(snapshot.outgoing.dps, 50.0);
    }

    #[test]
    fn live_snapshot_accumulates_outgoing_healing_and_resets_for_the_next_encounter() {
        let mut engine = CombatEngine::new();
        engine.ingest(event(0, EventKind::BossStarted, 0.0));
        let mut healing = event(1, EventKind::Healing, 240.0);
        healing.target = Some("PlayerTwo".to_owned());
        engine.ingest(healing);
        let mut self_healing = event(2, EventKind::Healing, 90.0);
        self_healing.target = Some("PlayerOne".to_owned());
        engine.ingest(self_healing);
        engine.ingest(event(3, EventKind::Healing, 120.0));

        let snapshot = engine.snapshot();
        assert_eq!(snapshot.players.len(), 1);
        assert_eq!(snapshot.players[0].healing, 240.0);

        let mut next_boss = event(4, EventKind::BossStarted, 0.0);
        next_boss.stage = Some("Stage_Next".to_owned());
        next_boss.boss = Some("NextBoss".to_owned());
        next_boss.phase = Some(BOSS_PROGRESS_ANCHORS[1]);
        engine.ingest(next_boss);

        assert_eq!(engine.snapshot().players[0].healing, 0.0);
    }

    #[test]
    fn historical_player_healing_excludes_self_heals_and_unattributed_targets() {
        let mut other_player = event(1, EventKind::Healing, 240.0);
        other_player.target = Some("PlayerTwo".to_owned());
        let mut self_heal = event(2, EventKind::Healing, 90.0);
        self_heal.target = Some("PlayerOne".to_owned());
        let missing_target = event(3, EventKind::Healing, 120.0);
        let mut later_other_player = event(4, EventKind::Healing, 60.0);
        later_other_player.target = Some("PlayerThree".to_owned());
        let mut negative = event(5, EventKind::Healing, -40.0);
        negative.target = Some("PlayerTwo".to_owned());
        let mut non_finite = event(6, EventKind::Healing, f64::NAN);
        non_finite.target = Some("PlayerTwo".to_owned());
        let events = [
            &other_player,
            &self_heal,
            &missing_target,
            &later_other_player,
            &negative,
            &non_finite,
        ];

        let stats = summarize_player("PlayerOne", &events, 10.0);

        assert_eq!(stats.healing, 300.0);
        assert_eq!(stats.active_seconds, 3.0);
    }

    #[test]
    fn wall_clock_snapshots_decay_windows_and_age_inferred_focus() {
        let mut engine = CombatEngine::new();
        engine.ingest(event(0, EventKind::BossStarted, 0.0));
        engine.ingest(event(1, EventKind::DamageDealt, 100.0));
        let mut ownership = event(2, EventKind::OwnershipTransferred, 0.0);
        ownership.entity = Some("TestBoss(Clone)".to_owned());
        ownership.target = Some("PlayerOne".to_owned());
        engine.ingest(ownership);
        assert!(engine.snapshot().focus.is_some());
        assert!(engine.requires_clock_tick());

        let now = event(20, EventKind::GameMessage, 0.0).timestamp;
        let snapshot = engine.snapshot_at(Some(now));
        assert_eq!(snapshot.encounter.duration_seconds, 20.0);
        assert_eq!(snapshot.outgoing.dps, 5.0);
        assert_eq!(snapshot.outgoing.rolling_5s, 0.0);
        assert_eq!(snapshot.recent_hits[0].age_seconds, 19.0);
        let focus = snapshot.focus.as_ref().expect("target candidate persists");
        assert_eq!(focus.player, "PlayerOne");
        assert_eq!(focus.confidence, "possible");
        assert_eq!(
            snapshot.timeline.first().unwrap().timestamp,
            event(0, EventKind::BossStarted, 0.0).timestamp
        );
        assert_eq!(snapshot.timeline.last().unwrap().timestamp, now);
        assert_eq!(snapshot.timeline.last().unwrap().rolling_dps, 0.0);

        engine.ingest(event(21, EventKind::Lobby, 0.0));
        let expiry_clock =
            event(2, EventKind::GameMessage, 0.0).timestamp + chrono::Duration::seconds(60);
        assert!(engine
            .snapshot_at(Some(expiry_clock))
            .recent_hits
            .is_empty());
        assert!(!engine.requires_clock_tick_at(Some(expiry_clock + chrono::Duration::seconds(1))));
    }

    #[test]
    fn phase_hits_survive_idle_time_and_unrelated_event_churn_until_the_next_phase() {
        let mut engine = CombatEngine::new();
        engine.ingest(event(0, EventKind::BossStarted, 0.0));
        engine.ingest(event(1, EventKind::DamageDealt, 345.0));
        for second in 2..=50 {
            engine.ingest(event(second, EventKind::GameMessage, 0.0));
        }

        let five_minutes_later =
            event(1, EventKind::DamageDealt, 0.0).timestamp + chrono::Duration::minutes(5);
        let retained = engine.snapshot_at(Some(five_minutes_later));
        assert_eq!(retained.recent_hits.len(), 1);
        assert_eq!(retained.recent_hits[0].amount, 345.0);
        assert_eq!(retained.recent_hits[0].age_seconds, 300.0);

        engine.ingest(stage_event(51, "Stage_Next", BOSS_PROGRESS_ANCHORS[1]));
        assert!(engine.snapshot().recent_hits.is_empty());
    }

    #[test]
    fn only_a_matching_boss_defeat_clears_the_current_phase_hits() {
        let mut engine = CombatEngine::new();
        engine.ingest(event(0, EventKind::BossStarted, 0.0));
        engine.ingest(event(1, EventKind::DamageDealt, 345.0));

        let mut stale_defeat = event(2, EventKind::BossDefeated, 0.0);
        stale_defeat.boss = Some("EarlierBoss".to_owned());
        engine.ingest(stale_defeat);
        assert_eq!(engine.snapshot().recent_hits.len(), 1);

        engine.ingest(event(3, EventKind::BossDefeated, 0.0));
        assert!(engine.snapshot().recent_hits.is_empty());
    }

    #[test]
    fn phase_hit_feed_is_bounded_and_newest_first() {
        let mut engine = CombatEngine::new();
        engine.ingest(event(0, EventKind::BossStarted, 0.0));
        for second in 1..=13 {
            engine.ingest(event(second, EventKind::DamageDealt, f64::from(second)));
        }

        let hits = engine.snapshot().recent_hits;
        assert_eq!(hits.len(), PHASE_HIT_LIMIT);
        assert_eq!(hits.first().map(|hit| hit.amount), Some(13.0));
        assert_eq!(hits.last().map(|hit| hit.amount), Some(2.0));
    }

    #[test]
    fn inferred_focus_uses_exact_boss_entities_and_persists_as_aging_then_stale() {
        let mut engine = CombatEngine::new();
        let mut boss = event(0, EventKind::BossStarted, 0.0);
        boss.boss = Some("FlyLord".to_owned());
        engine.ingest(boss);

        let mut sub_entity = event(1, EventKind::OwnershipTransferred, 0.0);
        sub_entity.entity = Some("Fly(Clone)".to_owned());
        sub_entity.target = Some("PlayerOne".to_owned());
        engine.ingest(sub_entity);
        assert!(engine.snapshot().focus.is_none());

        let mut exact = event(2, EventKind::OwnershipTransferred, 0.0);
        exact.entity = Some("FlyLord(Clone)".to_owned());
        exact.target = Some("PlayerOne".to_owned());
        engine.ingest(exact.clone());
        assert_eq!(engine.snapshot().focus.unwrap().confidence, "possible");

        let aging_at = exact.timestamp + chrono::Duration::seconds(60);
        let aging = engine.snapshot_at(Some(aging_at)).focus.unwrap();
        assert_eq!(aging.confidence, "aging");
        assert_eq!(aging.age_seconds, 60.0);

        let stale_at = exact.timestamp + chrono::Duration::seconds(120);
        let stale = engine.snapshot_at(Some(stale_at)).focus.unwrap();
        assert_eq!(stale.confidence, "stale");
        assert_eq!(stale.age_seconds, 120.0);

        let mut next_boss = event(3, EventKind::BossStarted, 0.0);
        next_boss.boss = Some("M41D".to_owned());
        engine.ingest(next_boss);
        let mut pillar = event(4, EventKind::OwnershipTransferred, 0.0);
        pillar.entity = Some("M41DPillar(Clone)".to_owned());
        pillar.target = Some("PlayerOne".to_owned());
        engine.ingest(pillar);
        assert!(engine.snapshot().focus.is_none());
    }

    #[test]
    fn immediate_local_incoming_damage_corroborates_focus_candidate() {
        let mut engine = CombatEngine::new();
        engine.ingest(event(0, EventKind::BossStarted, 0.0));

        let mut ownership = event(1, EventKind::OwnershipTransferred, 0.0);
        ownership.entity = Some("TestBoss(Clone)".to_owned());
        ownership.target = Some("PlayerOne".to_owned());
        engine.ingest(ownership);

        let mut incoming = event(2, EventKind::DamageTaken, 25.0);
        incoming.source = Some("TestBoss(Clone)".to_owned());
        engine.ingest(incoming);

        let focus = engine.snapshot().focus.expect("corroborated focus");
        assert_eq!(focus.confidence, "likely");
        assert_eq!(focus.evidence, "boss_owner_plus_local_incoming");
        assert_eq!(focus.corroborating_hits, 1);
        assert!(focus.corroborated_at.is_some());
    }

    #[test]
    fn visit_boundaries_clear_nullable_live_context() {
        let mut engine = CombatEngine::new();
        let mut stage = event(1, EventKind::StageEntered, 0.0);
        stage.session_id = Some(77);
        engine.ingest(stage);
        let mut joined = event(2, EventKind::PlayerJoined, 0.0);
        joined.target = Some("PartyMember".to_owned());
        engine.ingest(joined);

        let entered = visit_event(3, EventKind::WorldEntered, None);
        engine.ingest(entered);
        let entered_snapshot = engine.snapshot();
        assert!(entered_snapshot.in_world);
        assert_eq!(entered_snapshot.session_id, None);
        assert_eq!(entered_snapshot.stage, None);
        assert_eq!(entered_snapshot.class_name, None);
        assert!(!entered_snapshot
            .roster
            .iter()
            .any(|name| name == "PartyMember"));

        engine.ingest(visit_event(4, EventKind::SessionSaved, Some(88)));
        let mut invalidated = visit_event(5, EventKind::GameMessage, None);
        invalidated.message = Some("session_invalidated".to_owned());
        engine.ingest(invalidated);
        assert_eq!(engine.snapshot().session_id, None);

        let mut lobby = event(6, EventKind::Lobby, 0.0);
        lobby.stage = None;
        lobby.class_name = None;
        engine.ingest(lobby);
        assert_eq!(engine.snapshot().stage, None);
        assert_eq!(engine.snapshot().class_name, None);

        engine.ingest(event(7, EventKind::WorldExited, 0.0));
        let exited = engine.snapshot();
        assert!(!exited.in_world);
        assert_eq!(exited.world, None);
        assert_eq!(exited.session_id, None);
        assert!(exited.roster.is_empty());
    }

    #[test]
    fn merges_players_sharing_a_session() {
        let mut first = event(1, EventKind::DamageDealt, 100.0);
        first.player = Some("Alpha".to_owned());
        let mut second = event(2, EventKind::DamageDealt, 250.0);
        second.player = Some("Beta".to_owned());
        let runs = analyze_runs(&[first, second]);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].players.len(), 2);
        assert_eq!(runs[0].outgoing.total, 350.0);
        assert_eq!(runs[0].source_count, 2);
    }

    #[test]
    fn partial_import_keeps_unattributed_damage_in_run_totals() {
        let mut hit = event(1, EventKind::DamageDealt, 125.0);
        hit.player = None;
        let runs = analyze_runs(&[hit]);
        assert_eq!(runs.len(), 1);
        assert!(runs[0].players.is_empty());
        assert_eq!(runs[0].outgoing.total, 125.0);
        assert_eq!(runs[0].outgoing.hits, 1);
        assert_eq!(runs[0].attacks[0].total, 125.0);
    }

    #[test]
    fn summarizes_run_and_player_attacks_with_a_five_second_timeline() {
        let started = event(0, EventKind::BossStarted, 0.0);

        let mut alpha_strike = event(1, EventKind::DamageDealt, 100.0);
        alpha_strike.player = Some("Alpha".to_owned());

        let mut alpha_incoming = event(1, EventKind::DamageTaken, 30.0);
        alpha_incoming.player = Some("Alpha".to_owned());
        alpha_incoming.source = Some("TestBoss".to_owned());
        alpha_incoming.timestamp += chrono::Duration::milliseconds(750);

        let mut alpha_non_strike = event(2, EventKind::DamageDealt, 50.0);
        alpha_non_strike.player = Some("Alpha".to_owned());
        alpha_non_strike.damage_type = Some("non-strike".to_owned());

        let mut beta_non_strike = event(3, EventKind::DamageDealt, 200.0);
        beta_non_strike.player = Some("Beta".to_owned());
        beta_non_strike.damage_type = Some("non-strike".to_owned());

        let mut alpha_late_strike = event(7, EventKind::DamageDealt, 25.0);
        alpha_late_strike.player = Some("Alpha".to_owned());

        let finished = event(8, EventKind::Lobby, 0.0);
        let run = summarize_run(
            "session-42".to_owned(),
            Some(42),
            &[
                started,
                alpha_strike,
                alpha_incoming,
                alpha_non_strike,
                beta_non_strike,
                alpha_late_strike,
                finished,
            ],
        );

        assert_eq!(run.attacks.len(), 2);
        assert_eq!(run.attacks[0].damage_type, "non-strike");
        assert_eq!(run.attacks[0].total, 250.0);
        assert_eq!(run.attacks[0].hits, 2);
        assert_eq!(run.attacks[0].share, 2.0 / 3.0);
        assert_eq!(run.attacks[1].damage_type, "strike");
        assert_eq!(run.attacks[1].total, 125.0);

        let alpha = run
            .players
            .iter()
            .find(|player| player.player == "Alpha")
            .unwrap();
        assert_eq!(alpha.attacks.len(), 2);
        assert_eq!(alpha.attacks[0].damage_type, "strike");
        assert_eq!(alpha.attacks[0].total, 125.0);
        assert_eq!(alpha.attacks[0].min, 25.0);
        assert_eq!(alpha.attacks[0].max, 100.0);
        assert_eq!(alpha.attacks[0].average, 62.5);
        assert_eq!(alpha.attacks[1].damage_type, "non-strike");
        assert_eq!(alpha.attacks[1].total, 50.0);
        assert_eq!(alpha.damage.dps, 175.0 / 8.0);
        assert_eq!(alpha.incoming.damage_per_second, 30.0 / 8.0);
        let beta = run
            .players
            .iter()
            .find(|player| player.player == "Beta")
            .unwrap();
        assert_eq!(beta.damage.dps, 200.0 / 8.0);

        assert_eq!(run.timeline.len(), 8);
        let timeline_start = run.started_at.unwrap();
        let point_at = |second| {
            run.timeline
                .iter()
                .find(|point| point.timestamp == timeline_start + chrono::Duration::seconds(second))
                .unwrap()
        };
        assert_eq!(point_at(0).rolling_dps, 0.0);
        assert_eq!(point_at(1).timestamp.nanosecond(), 0);
        assert_eq!(point_at(1).outgoing, 100.0);
        assert_eq!(point_at(1).incoming, 30.0);
        assert_eq!(point_at(1).rolling_dps, 20.0);
        assert_eq!(point_at(2).rolling_dps, 30.0);
        assert_eq!(point_at(3).rolling_dps, 70.0);
        assert_eq!(point_at(6).rolling_dps, 70.0);
        assert_eq!(point_at(7).outgoing, 25.0);
        assert_eq!(point_at(7).rolling_dps, 55.0);
        assert_eq!(point_at(8).rolling_dps, 45.0);
    }

    #[test]
    fn historical_timeline_drops_to_zero_across_a_two_minute_gap() {
        let started = event(0, EventKind::BossStarted, 0.0);
        let first_hit = event(1, EventKind::DamageDealt, 100.0);
        let mut second_hit = event(2, EventKind::DamageDealt, 50.0);
        second_hit.timestamp += chrono::Duration::minutes(2);
        let mut finished = event(3, EventKind::Lobby, 0.0);
        finished.timestamp += chrono::Duration::minutes(2);

        let run = summarize_run(
            "session-42".to_owned(),
            Some(42),
            &[started, first_hit, second_hit.clone(), finished],
        );

        assert!(run.timeline.len() <= 10, "idle time should stay sparse");
        assert_eq!(
            run.timeline.first().unwrap().timestamp,
            run.started_at.unwrap()
        );
        assert_eq!(
            run.timeline.last().unwrap().timestamp,
            run.ended_at.unwrap()
        );

        let first_expired_at = run.started_at.unwrap() + chrono::Duration::seconds(7);
        let zero_after_first = run
            .timeline
            .iter()
            .find(|point| point.timestamp == first_expired_at)
            .unwrap();
        let zero_before_second = run
            .timeline
            .iter()
            .find(|point| point.timestamp == second_hit.timestamp - chrono::Duration::seconds(1))
            .unwrap();
        assert_eq!(zero_after_first.rolling_dps, 0.0);
        assert_eq!(zero_before_second.rolling_dps, 0.0);
        assert!((zero_before_second.timestamp - zero_after_first.timestamp).num_seconds() > 100);

        let second_bucket = run
            .timeline
            .iter()
            .find(|point| point.timestamp == second_hit.timestamp)
            .unwrap();
        assert_eq!(second_bucket.outgoing, 50.0);
        assert_eq!(second_bucket.rolling_dps, 10.0);
    }

    #[test]
    fn historical_timeline_preserves_exact_observation_boundaries() {
        let mut started = event(0, EventKind::BossStarted, 0.0);
        started.timestamp += chrono::Duration::milliseconds(250);
        let mut finished = event(1, EventKind::Lobby, 0.0);
        finished.timestamp += chrono::Duration::milliseconds(750);

        let run = summarize_run(
            "session-42".to_owned(),
            Some(42),
            &[started.clone(), finished.clone()],
        );

        assert_eq!(run.timeline.len(), 2);
        assert_eq!(run.timeline[0].timestamp, started.timestamp);
        assert_eq!(run.timeline[1].timestamp, finished.timestamp);
        assert!(run
            .timeline
            .iter()
            .all(|point| point.outgoing == 0.0 && point.incoming == 0.0));
    }

    #[test]
    fn encounter_closes_near_the_first_completion_marker() {
        let started = event(0, EventKind::BossStarted, 0.0);
        let hit = event(1, EventKind::DamageDealt, 100.0);
        let completed = event(2, EventKind::Lobby, 0.0);
        let mut peer_late_hit = event(7, EventKind::DamageDealt, 25.0);
        peer_late_hit.player = Some("Beta".to_owned());
        let mut noise = event(10, EventKind::GameMessage, 0.0);
        noise.timestamp += chrono::Duration::seconds(90);
        let mut exited = event(20, EventKind::WorldExited, 0.0);
        exited.timestamp += chrono::Duration::seconds(100);

        let events = vec![started, hit, completed, peer_late_hit, noise, exited];
        let encounter = summarize_encounter(&events);

        assert_eq!(encounter.duration_seconds, 7.0);
        assert_eq!(encounter.outgoing.total, 125.0);
        assert!(encounter.completed);

        let run = summarize_run("session-42".to_owned(), Some(42), &events);
        assert_eq!(run.duration_seconds, 7.0);
        assert_eq!(run.outgoing.total, 125.0);
        assert_eq!(run.event_count, 4);
    }

    #[test]
    fn boss_scope_excludes_and_separately_reports_pre_boss_damage() {
        let mut stage = event(0, EventKind::StageEntered, 0.0);
        stage.boss = None;
        stage.phase = Some(0.1);

        let mut trash_hit = event(1, EventKind::DamageDealt, 100.0);
        trash_hit.boss = None;
        trash_hit.phase = Some(0.1);

        let mut boss_started = event(10, EventKind::BossStarted, 0.0);
        boss_started.phase = Some(1.0);
        let boss_hit = event(11, EventKind::DamageDealt, 200.0);
        let boss_defeated = event(20, EventKind::BossDefeated, 0.0);

        let run = summarize_run(
            "session-42".to_owned(),
            Some(42),
            &[stage, trash_hit, boss_started, boss_hit, boss_defeated],
        );

        assert_eq!(run.metrics_scope, "boss");
        assert_eq!(run.boss_count, 1);
        assert_eq!(run.duration_seconds, 10.0);
        assert_eq!(run.outgoing.total, 200.0);
        assert_eq!(run.outgoing.hits, 1);
        assert_eq!(run.outgoing.dps, 20.0);
        assert_eq!(run.pre_boss_duration_seconds, 10.0);
        assert_eq!(run.pre_boss_outgoing.total, 100.0);
        assert_eq!(run.observed_outgoing.total, 300.0);
        assert_eq!(run.observed_duration_seconds, 20.0);

        assert_eq!(run.encounters.len(), 2);
        assert_eq!(run.encounters[0].kind, "pre_boss");
        assert_eq!(run.encounters[0].end_reason, "boss_started");
        assert_eq!(run.encounters[0].boundary_confidence, "structural");
        assert_eq!(run.encounters[0].outgoing.total, 100.0);
        assert_eq!(run.encounters[1].kind, "boss");
        assert_eq!(run.encounters[1].end_reason, "boss_defeated");
        assert_eq!(run.encounters[1].boundary_confidence, "explicit");
        assert_eq!(run.encounters[1].outgoing.total, 200.0);
        assert_eq!(
            run.encounters[1].timeline.first().unwrap().timestamp,
            event(10, EventKind::BossStarted, 0.0).timestamp
        );
        assert_eq!(
            run.encounters[1].timeline.last().unwrap().timestamp,
            event(20, EventKind::BossDefeated, 0.0).timestamp
        );
        assert!(run.encounters[1]
            .timeline
            .iter()
            .any(|point| point.outgoing == 200.0));
    }

    #[test]
    fn late_prior_phase_death_attaches_by_name_without_closing_next_phase() {
        let mut phase_two = event(0, EventKind::BossStarted, 0.0);
        phase_two.boss = Some("JimBringerPhase2".to_owned());
        phase_two.phase = Some(2.0);
        let mut phase_two_hit = event(1, EventKind::DamageDealt, 125.0);
        phase_two_hit.boss = Some("JimBringerPhase2".to_owned());
        phase_two_hit.phase = Some(2.0);

        let mut phase_three = event(10, EventKind::BossStarted, 0.0);
        phase_three.boss = Some("JimBringerPhase3".to_owned());
        phase_three.phase = Some(3.0);
        let mut delayed_phase_two_death = event(10, EventKind::BossDefeated, 0.0);
        delayed_phase_two_death.boss = Some("JimBringerPhase2".to_owned());
        delayed_phase_two_death.phase = Some(3.0);
        let mut phase_three_hit = event(11, EventKind::DamageDealt, 250.0);
        phase_three_hit.boss = Some("JimBringerPhase3".to_owned());
        phase_three_hit.phase = Some(3.0);

        let events = vec![
            phase_two,
            phase_two_hit,
            phase_three,
            delayed_phase_two_death,
            phase_three_hit,
        ];
        let run = summarize_run("session-42".to_owned(), Some(42), &events);
        assert_eq!(run.encounters.len(), 2);
        assert_eq!(run.encounters[0].name, "JimBringerPhase2");
        assert_eq!(run.encounters[0].end_reason, "boss_defeated");
        assert_eq!(run.encounters[0].boundary_confidence, "explicit");
        assert!(run.encounters[0].completed);
        assert_eq!(run.encounters[0].duration_seconds, 10.0);
        assert_eq!(run.encounters[0].outgoing.total, 125.0);
        assert_eq!(run.encounters[1].name, "JimBringerPhase3");
        assert_eq!(run.encounters[1].end_reason, "open");
        assert!(!run.encounters[1].completed);
        assert_eq!(run.encounters[1].outgoing.total, 250.0);

        let mut engine = CombatEngine::new();
        for value in events {
            engine.ingest(value);
        }
        let live = engine.snapshot();
        assert_eq!(live.encounter.name, "JimBringerPhase3");
        assert!(live.encounter.active);
        assert_eq!(live.outgoing.total, 250.0);
    }

    #[test]
    fn delayed_prior_phase_death_is_explicit_but_clamped_to_next_phase_start() {
        let mut phase_two = event(0, EventKind::BossStarted, 0.0);
        phase_two.boss = Some("JimBringerPhase2".to_owned());
        let mut phase_two_hit = event(1, EventKind::DamageDealt, 125.0);
        phase_two_hit.boss = Some("JimBringerPhase2".to_owned());

        let mut phase_three = event(10, EventKind::BossStarted, 0.0);
        phase_three.boss = Some("JimBringerPhase3".to_owned());
        let mut delayed_phase_two_death = event(12, EventKind::BossDefeated, 0.0);
        delayed_phase_two_death.boss = Some("JimBringerPhase2".to_owned());
        let mut phase_three_hit = event(13, EventKind::DamageDealt, 250.0);
        phase_three_hit.boss = Some("JimBringerPhase3".to_owned());
        let mut phase_three_death = event(20, EventKind::BossDefeated, 0.0);
        phase_three_death.boss = Some("JimBringerPhase3".to_owned());

        let run = summarize_run(
            "session-42".to_owned(),
            Some(42),
            &[
                phase_two,
                phase_two_hit,
                phase_three.clone(),
                delayed_phase_two_death,
                phase_three_hit,
                phase_three_death,
            ],
        );
        let boss_encounters: Vec<_> = run
            .encounters
            .iter()
            .filter(|encounter| encounter.kind == "boss")
            .collect();
        assert_eq!(boss_encounters.len(), 2);
        assert_eq!(boss_encounters[0].end_reason, "boss_defeated");
        assert_eq!(boss_encounters[0].boundary_confidence, "explicit");
        assert_eq!(boss_encounters[0].ended_at, Some(phase_three.timestamp));
        assert_eq!(boss_encounters[0].duration_seconds, 10.0);
        assert_eq!(
            boss_encounters[0].timeline.last().unwrap().elapsed_seconds,
            Some(10.0)
        );
        assert_eq!(boss_encounters[1].started_at, Some(phase_three.timestamp));
        assert_eq!(boss_encounters[1].duration_seconds, 10.0);

        let summed_duration: f64 = boss_encounters
            .iter()
            .map(|encounter| encounter.duration_seconds)
            .sum();
        assert_eq!(summed_duration, 20.0);
        assert_eq!(run.duration_seconds, summed_duration);
        assert_eq!(run.timeline.first().unwrap().elapsed_seconds, Some(0.0));
        assert_eq!(run.timeline.last().unwrap().elapsed_seconds, Some(20.0));
        assert!(run.timeline.windows(2).all(|points| {
            points[0].elapsed_seconds.unwrap() <= points[1].elapsed_seconds.unwrap()
        }));
    }

    #[test]
    fn encounter_boundaries_fall_back_to_next_start_and_intermission() {
        let mut stage = event(0, EventKind::StageEntered, 0.0);
        stage.boss = None;
        let mut first_boss = event(10, EventKind::BossStarted, 0.0);
        first_boss.boss = Some("FirstBoss".to_owned());
        let mut first_hit = event(11, EventKind::DamageDealt, 75.0);
        first_hit.boss = Some("FirstBoss".to_owned());
        let mut second_boss = event(20, EventKind::BossStarted, 0.0);
        second_boss.boss = Some("SecondBoss".to_owned());
        let mut second_hit = event(21, EventKind::DamageDealt, 125.0);
        second_hit.boss = Some("SecondBoss".to_owned());
        let intermission = event(30, EventKind::Intermission, 0.0);

        let run = summarize_run(
            "session-42".to_owned(),
            Some(42),
            &[
                stage,
                first_boss,
                first_hit,
                second_boss,
                second_hit,
                intermission,
            ],
        );
        assert_eq!(run.encounters.len(), 3);
        assert_eq!(run.encounters[0].end_reason, "boss_started");
        assert_eq!(run.encounters[1].end_reason, "next_boss");
        assert_eq!(run.encounters[2].end_reason, "intermission");
        assert!(run.encounters.iter().all(|encounter| encounter.completed));
        assert!(run
            .encounters
            .iter()
            .all(|encounter| encounter.boundary_confidence == "structural"));
        assert_eq!(run.outgoing.total, 200.0);
    }

    #[test]
    fn named_damage_summary_is_an_explicit_legacy_boss_boundary() {
        let started = event(0, EventKind::BossStarted, 0.0);
        let hit = event(1, EventKind::DamageDealt, 50.0);
        let summary = event(5, EventKind::BossDamageSummary, 0.0);
        let run = summarize_run("session-42".to_owned(), Some(42), &[started, hit, summary]);
        assert_eq!(run.encounters.len(), 1);
        assert_eq!(run.encounters[0].end_reason, "boss_summary");
        assert_eq!(run.encounters[0].boundary_confidence, "explicit");
        assert!(run.encounters[0].completed);
    }

    #[test]
    fn matching_named_death_after_intermission_wins_within_peer_tolerance() {
        let started = event(0, EventKind::BossStarted, 0.0);
        let hit = event(1, EventKind::DamageDealt, 50.0);
        let intermission = event(5, EventKind::Intermission, 0.0);
        let mut peer_hit = event(6, EventKind::DamageDealt, 25.0);
        peer_hit.player = Some("Beta".to_owned());
        let named_death = event(7, EventKind::BossDefeated, 0.0);

        let encounter = summarize_encounter(&[started, hit, intermission, peer_hit, named_death]);
        assert_eq!(encounter.end_reason, "boss_defeated");
        assert_eq!(encounter.boundary_confidence, "explicit");
        assert_eq!(
            encounter.ended_at,
            Some(event(7, EventKind::BossDefeated, 0.0).timestamp)
        );
        assert_eq!(encounter.duration_seconds, 7.0);
        assert_eq!(encounter.outgoing.total, 75.0);
    }

    #[test]
    fn stale_named_deaths_cannot_override_intermission_or_lobby_boundaries() {
        let started = event(0, EventKind::BossStarted, 0.0);
        let hit = event(1, EventKind::DamageDealt, 50.0);
        let intermission = event(5, EventKind::Intermission, 0.0);
        let named_too_late = event(20, EventKind::BossDefeated, 0.0);
        let encounter =
            summarize_encounter(&[started.clone(), hit.clone(), intermission, named_too_late]);
        assert_eq!(encounter.end_reason, "intermission");
        assert_eq!(encounter.boundary_confidence, "structural");
        assert_eq!(encounter.duration_seconds, 5.0);

        let lobby = event(5, EventKind::Lobby, 0.0);
        let named_after_lobby = event(6, EventKind::BossDefeated, 0.0);
        let run = summarize_run(
            "session-42".to_owned(),
            Some(42),
            &[started, hit, lobby, named_after_lobby],
        );
        assert_eq!(run.encounters.len(), 1);
        assert_eq!(run.encounters[0].end_reason, "lobby");
        assert_eq!(run.encounters[0].boundary_confidence, "structural");
        assert_eq!(run.encounters[0].duration_seconds, 5.0);
    }

    #[test]
    fn combined_boss_timeline_concatenates_combat_time_across_long_gaps() {
        let first_started = event(0, EventKind::BossStarted, 0.0);
        let first_hit = event(1, EventKind::DamageDealt, 100.0);
        let first_defeated = event(10, EventKind::BossDefeated, 0.0);

        let mut pre_boss_stage = event(11, EventKind::StageEntered, 0.0);
        pre_boss_stage.boss = None;
        let mut trash_hit = event(20, EventKind::DamageDealt, 500.0);
        trash_hit.boss = None;

        let long_gap = chrono::Duration::minutes(10);
        let mut second_started = event(40, EventKind::BossStarted, 0.0);
        second_started.timestamp += long_gap;
        second_started.boss = Some("SecondBoss".to_owned());
        let mut second_hit = event(41, EventKind::DamageDealt, 200.0);
        second_hit.timestamp += long_gap;
        second_hit.boss = Some("SecondBoss".to_owned());
        let mut second_defeated = event(55, EventKind::BossDefeated, 0.0);
        second_defeated.timestamp += long_gap;
        second_defeated.boss = Some("SecondBoss".to_owned());

        let run = summarize_run(
            "session-42".to_owned(),
            Some(42),
            &[
                first_started,
                first_hit,
                first_defeated,
                pre_boss_stage,
                trash_hit,
                second_started,
                second_hit,
                second_defeated,
            ],
        );
        assert_eq!(run.boss_count, 2);
        assert_eq!(run.duration_seconds, 25.0);
        assert_eq!(run.outgoing.total, 300.0);
        assert_eq!(run.timeline.first().unwrap().elapsed_seconds, Some(0.0));
        assert_eq!(run.timeline.last().unwrap().elapsed_seconds, Some(25.0));
        assert!(
            seconds_between(
                run.timeline.first().unwrap().timestamp,
                run.timeline.last().unwrap().timestamp
            ) > 600.0
        );

        let boss_encounters: Vec<_> = run
            .encounters
            .iter()
            .filter(|encounter| encounter.kind == "boss")
            .collect();
        assert_eq!(
            boss_encounters[0].timeline.last().unwrap().elapsed_seconds,
            Some(10.0)
        );
        assert_eq!(
            boss_encounters[1].timeline.first().unwrap().elapsed_seconds,
            Some(0.0)
        );
        assert_eq!(
            boss_encounters[1].timeline.last().unwrap().elapsed_seconds,
            Some(15.0)
        );
    }

    #[test]
    fn boss_scoped_player_activity_sums_each_fight_without_idle_gap() {
        let first_started = event(0, EventKind::BossStarted, 0.0);
        let mut first_hit = event(1, EventKind::DamageDealt, 100.0);
        first_hit.player = Some("SparsePlayer".to_owned());
        let first_defeated = event(10, EventKind::BossDefeated, 0.0);

        let long_gap = chrono::Duration::minutes(10);
        let mut second_started = event(20, EventKind::BossStarted, 0.0);
        second_started.timestamp += long_gap;
        second_started.boss = Some("SecondBoss".to_owned());
        let mut second_hit = event(21, EventKind::DamageDealt, 100.0);
        second_hit.timestamp += long_gap;
        second_hit.boss = Some("SecondBoss".to_owned());
        second_hit.player = Some("SparsePlayer".to_owned());
        let mut second_defeated = event(30, EventKind::BossDefeated, 0.0);
        second_defeated.timestamp += long_gap;
        second_defeated.boss = Some("SecondBoss".to_owned());

        let run = summarize_run(
            "session-42".to_owned(),
            Some(42),
            &[
                first_started,
                first_hit,
                first_defeated,
                second_started,
                second_hit,
                second_defeated,
            ],
        );
        assert_eq!(run.boss_count, 2);
        assert_eq!(run.duration_seconds, 20.0);

        let player = run
            .players
            .iter()
            .find(|player| player.player == "SparsePlayer")
            .unwrap();
        assert_eq!(player.damage.total, 200.0);
        assert_eq!(player.damage.dps, 10.0);
        assert_eq!(player.active_seconds, 2.0);
        assert_eq!(player.active_seconds / run.duration_seconds, 0.1);
        assert!(player.active_seconds < run.duration_seconds);
    }

    #[test]
    fn saved_session_claims_the_whole_visit_and_empty_visits_are_removed() {
        let empty_entered = visit_event(0, EventKind::WorldEntered, None);
        let empty_saved = visit_event(1, EventKind::SessionSaved, Some(99));
        let empty_exited = visit_event(2, EventKind::WorldExited, Some(99));

        let entered = visit_event(3, EventKind::WorldEntered, None);
        let stale = visit_event(4, EventKind::SessionLoaded, Some(16367));
        let mut invalidated = visit_event(5, EventKind::GameMessage, None);
        invalidated.message = Some("session_invalidated".to_owned());
        let mut stage = visit_event(6, EventKind::StageEntered, None);
        stage.stage = Some("Stage_PreSave".to_owned());
        stage.class_name = Some("Spellhammer".to_owned());
        let saved = visit_event(7, EventKind::SessionSaved, Some(32179));
        let mut damage = visit_event(8, EventKind::DamageDealt, Some(32179));
        damage.amount = Some(125.0);
        damage.damage_type = Some("strike".to_owned());
        let exited = visit_event(9, EventKind::WorldExited, Some(32179));

        let all_events = vec![
            empty_entered,
            empty_saved,
            empty_exited,
            entered,
            stale,
            invalidated,
            stage,
            saved,
            damage,
            exited,
        ];
        let runs = analyze_runs(&all_events);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, "session-32179");
        assert_eq!(runs[0].session_id, Some(32179));
        assert_eq!(runs[0].stages, ["Stage_PreSave"]);
        assert_eq!(runs[0].outgoing.total, 125.0);
        assert_eq!(runs[0].event_count, 7);

        let detail = events_for_run(&all_events, "session-32179");
        assert_eq!(detail.len(), 7);
        assert!(detail
            .iter()
            .any(|event| { event.kind == EventKind::StageEntered && event.session_id.is_none() }));
        assert!(events_for_run(&all_events, "session-16367").is_empty());
        assert!(events_for_run(&all_events, "session-99").is_empty());
    }

    #[test]
    fn rejects_a_stale_loaded_id_even_without_an_invalidation_event() {
        let entered = visit_event(0, EventKind::WorldEntered, None);
        let stale = visit_event(1, EventKind::SessionLoaded, Some(16367));
        let mut stage = visit_event(2, EventKind::StageEntered, None);
        stage.stage = Some("Stage_Actual".to_owned());
        let mut damage = visit_event(3, EventKind::DamageDealt, None);
        damage.amount = Some(50.0);
        damage.damage_type = Some("non-strike".to_owned());
        let exited = visit_event(4, EventKind::WorldExited, None);

        let runs = analyze_runs(&[entered, stale, stage, damage, exited]);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].session_id, None);
        assert!(runs[0].id.starts_with("instance-"));
        assert_ne!(runs[0].id, "session-16367");
    }

    #[test]
    fn accepts_a_loaded_id_that_is_carried_by_gameplay() {
        let entered = visit_event(0, EventKind::WorldEntered, None);
        let loaded = visit_event(1, EventKind::SessionLoaded, Some(42));
        let mut stage = visit_event(2, EventKind::StageEntered, Some(42));
        stage.stage = Some("Stage_Valid".to_owned());
        let mut damage = visit_event(3, EventKind::DamageDealt, Some(42));
        damage.amount = Some(75.0);
        damage.damage_type = Some("strike".to_owned());
        let exited = visit_event(4, EventKind::WorldExited, Some(42));

        let runs = analyze_runs(&[entered, loaded, stage, damage, exited]);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, "session-42");
        assert_eq!(runs[0].session_id, Some(42));
    }

    #[test]
    fn merged_collectors_share_canonical_encounter_windows() {
        let player_event = |second: u32,
                            player: &str,
                            kind: EventKind,
                            amount: Option<f64>,
                            stage: Option<&str>,
                            boss: Option<&str>| {
            let mut value = visit_event(second, kind, Some(77));
            value.player = Some(player.to_owned());
            value.amount = amount;
            value.damage_type = amount.map(|_| "strike".to_owned());
            value.stage = stage.map(str::to_owned);
            value.boss = boss.map(str::to_owned);
            value
        };

        let events = vec![
            player_event(0, "Alpha", EventKind::WorldEntered, None, None, None),
            player_event(1, "Alpha", EventKind::SessionSaved, None, None, None),
            player_event(
                2,
                "Alpha",
                EventKind::StageEntered,
                None,
                Some("Stage_Test"),
                None,
            ),
            player_event(
                3,
                "Alpha",
                EventKind::DamageDealt,
                Some(100.0),
                Some("Stage_Test"),
                None,
            ),
            player_event(
                4,
                "Alpha",
                EventKind::BossStarted,
                None,
                Some("Stage_Test"),
                Some("TestBoss"),
            ),
            player_event(
                5,
                "Alpha",
                EventKind::DamageDealt,
                Some(200.0),
                Some("Stage_Test"),
                Some("TestBoss"),
            ),
            player_event(8, "Alpha", EventKind::Intermission, None, None, None),
            player_event(0, "Beta", EventKind::WorldEntered, None, None, None),
            player_event(1, "Beta", EventKind::SessionSaved, None, None, None),
            player_event(
                3,
                "Beta",
                EventKind::StageEntered,
                None,
                Some("Stage_Test"),
                None,
            ),
            player_event(
                5,
                "Beta",
                EventKind::BossStarted,
                None,
                Some("Stage_Test"),
                Some("TestBoss"),
            ),
            player_event(
                6,
                "Beta",
                EventKind::DamageDealt,
                Some(150.0),
                Some("Stage_Test"),
                Some("TestBoss"),
            ),
            player_event(9, "Beta", EventKind::Intermission, None, None, None),
        ];

        let runs = analyze_runs(&events);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].source_count, 2);
        assert_eq!(runs[0].encounters.len(), 2);
        assert_eq!(runs[0].encounters[0].name, "Stage_Test");
        assert_eq!(runs[0].encounters[0].outgoing.total, 100.0);
        assert_eq!(runs[0].encounters[1].name, "TestBoss");
        assert_eq!(runs[0].encounters[1].players.len(), 2);
        assert_eq!(runs[0].encounters[1].outgoing.total, 350.0);
    }
}
