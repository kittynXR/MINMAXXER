use crate::model::{
    AttackStats, DamageTotals, EncounterStats, EventKind, GameEvent, IncomingTotals,
    ParserCoverage, PlayerStats, RunSummary, TimelinePoint,
};
use chrono::{NaiveDateTime, Timelike};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const RECENT_EVENT_LIMIT: usize = 40;
const RECENT_HIT_DISPLAY_SECONDS: f64 = 60.0;
const DAMAGE_WINDOW_SECONDS: i64 = 30;
const MERGED_BOUNDARY_TOLERANCE_SECONDS: i64 = 10;

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
    pub source_note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineSnapshot {
    pub version: u64,
    pub connected: bool,
    pub status: String,
    pub observed_player: Option<String>,
    pub session_id: Option<u32>,
    pub world: Option<String>,
    pub stage: Option<String>,
    pub class_name: Option<String>,
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
            status: "Waiting for VRChat".to_owned(),
            observed_player: None,
            session_id: None,
            world: None,
            stage: None,
            class_name: None,
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
    encounter: LiveEncounter,
    focus: Option<FocusSignal>,
    outgoing: DamageTotals,
    incoming: IncomingTotals,
    roster: BTreeSet<String>,
    damage_by_type: BTreeMap<String, (f64, u64, f64, f64)>,
    damage_window: VecDeque<(NaiveDateTime, f64)>,
    timeline: VecDeque<TimelinePoint>,
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
            encounter: LiveEncounter::default(),
            focus: None,
            outgoing: DamageTotals::default(),
            incoming: IncomingTotals::default(),
            roster: BTreeSet::new(),
            damage_by_type: BTreeMap::new(),
            damage_window: VecDeque::new(),
            timeline: VecDeque::new(),
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
        if event.stage.is_some() {
            self.stage = event.stage.clone();
        }
        if event.class_name.is_some() {
            self.class_name = event.class_name.clone();
        }

        match event.kind {
            EventKind::WorldEntered => {
                self.in_world = true;
                self.reset_encounter();
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
                }
            }
            EventKind::LocalPlayerIdentified => {
                if let Some(player) = event.player.as_ref() {
                    self.observed_player = Some(player.clone());
                    self.roster.insert(player.clone());
                }
            }
            EventKind::StageEntered => {
                self.begin_encounter(
                    event.stage.clone().unwrap_or_else(|| "Stage".to_owned()),
                    "stage",
                    event.timestamp,
                    event.phase,
                );
            }
            EventKind::BossStarted => {
                self.focus = None;
                self.begin_encounter(
                    event.boss.clone().unwrap_or_else(|| "Boss".to_owned()),
                    "boss",
                    event.timestamp,
                    event.phase,
                );
            }
            EventKind::Intermission | EventKind::Lobby | EventKind::BossDefeated => {
                self.update_duration(event.timestamp);
                self.encounter.active = false;
                self.focus = None;
                if event.kind == EventKind::Lobby {
                    self.stage = None;
                    self.class_name = None;
                }
            }
            EventKind::GameMessage if event.message.as_deref() == Some("session_invalidated") => {
                self.session_id = None;
            }
            EventKind::DamageDealt => self.add_outgoing(&event),
            EventKind::DamageTaken => self.add_incoming(&event),
            EventKind::OwnershipTransferred => self.observe_focus(&event),
            _ => {}
        }

        if self.recent_events.len() == RECENT_EVENT_LIMIT {
            self.recent_events.pop_front();
        }
        self.recent_events.push_back(event);
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
                    healing: 0.0,
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
                    outgoing: 0.0,
                    incoming: 0.0,
                    rolling_dps: outgoing.rolling_5s,
                });
            }
        }

        EngineSnapshot {
            version: self.version,
            connected: self.connected,
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
        self.encounter.active
            || self.current_focus_at(now).is_some()
            || self
                .recent_events
                .iter()
                .rev()
                .find(|event| event.kind == EventKind::DamageDealt)
                .is_some_and(|event| {
                    now.map(|now| seconds_between(event.timestamp, now))
                        .unwrap_or_default()
                        <= RECENT_HIT_DISPLAY_SECONDS + 1.0
                })
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

    fn reset_encounter(&mut self) {
        self.encounter = LiveEncounter::default();
        self.focus = None;
        self.outgoing = DamageTotals::default();
        self.incoming = IncomingTotals::default();
        self.damage_by_type.clear();
        self.damage_window.clear();
        self.timeline.clear();
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
        let entry = self
            .damage_by_type
            .entry(damage_type)
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

    fn observe_focus(&mut self, event: &GameEvent) {
        if !self.encounter.active || self.encounter.kind != "boss" {
            return;
        }
        let (Some(entity), Some(player)) = (event.entity.as_ref(), event.target.as_ref()) else {
            return;
        };
        let entity_name_key = entity_key(entity);
        let boss_key = entity_key(&self.encounter.name);
        if entity_name_key.is_empty()
            || boss_key.is_empty()
            || !(entity_name_key.contains(&boss_key) || boss_key.contains(&entity_name_key))
        {
            return;
        }
        self.focus = Some(FocusSignal {
            player: player.clone(),
            entity: entity.clone(),
            observed_at: event.timestamp,
            age_seconds: 0.0,
            confidence: "inferred".to_owned(),
            source_note: "Boss-matching VRChat network ownership; this may not equal hate/aggro."
                .to_owned(),
        });
    }

    fn current_focus_at(&self, now: Option<NaiveDateTime>) -> Option<FocusSignal> {
        let mut focus = self.focus.clone()?;
        focus.age_seconds = self
            .last_event_at
            .max(now)
            .map(|now| seconds_between(focus.observed_at, now))
            .unwrap_or_default();
        (focus.age_seconds <= 15.0).then_some(focus)
    }

    fn recent_hits_at(&self, limit: usize, now: Option<NaiveDateTime>) -> Vec<RecentHit> {
        self.recent_events
            .iter()
            .rev()
            .filter(|event| event.kind == EventKind::DamageDealt)
            .filter(|event| {
                now.map(|now| seconds_between(event.timestamp, now))
                    .unwrap_or_default()
                    <= RECENT_HIT_DISPLAY_SECONDS
            })
            .take(limit)
            .map(|event| RecentHit {
                timestamp: event.timestamp,
                amount: event.amount(),
                damage_type: event
                    .damage_type
                    .clone()
                    .unwrap_or_else(|| "unknown".to_owned()),
                age_seconds: now
                    .map(|now| seconds_between(event.timestamp, now))
                    .unwrap_or_default(),
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
    let started_at = events
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
    let first_completion = started_at.and_then(|started_at| {
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
    let ended_at = first_completion
        .map(|completed_at| {
            events
                .iter()
                .filter(|event| {
                    event.timestamp >= started_at.unwrap_or(event.timestamp)
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
    let duration_seconds = started_at
        .zip(ended_at)
        .map(|(start, end)| seconds_between(start, end))
        .unwrap_or_default();
    let completed = events
        .iter()
        .any(|event| matches!(event.kind, EventKind::Lobby | EventKind::WorldExited));

    let mut classes = BTreeSet::new();
    let mut stages = Vec::new();
    let mut recent_stage_boundaries: BTreeMap<String, NaiveDateTime> = BTreeMap::new();
    let mut player_events: BTreeMap<String, Vec<&GameEvent>> = BTreeMap::new();
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
    let encounters = summarize_encounters(events);

    RunSummary {
        id,
        session_id,
        started_at,
        ended_at,
        duration_seconds,
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
        .find(|event| matches!(event.kind, EventKind::DamageDealt | EventKind::DamageTaken))
        .map(|event| event.timestamp);
    let last_combat = events
        .iter()
        .rev()
        .find(|event| matches!(event.kind, EventKind::DamageDealt | EventKind::DamageTaken))
        .map(|event| event.timestamp);
    let active_seconds = first_combat
        .zip(last_combat)
        .map(|(start, end)| seconds_between(start, end).max(1.0))
        .unwrap_or_default();
    let mut damage = DamageTotals::default();
    let mut incoming = IncomingTotals::default();
    let mut healing = 0.0;
    let mut deaths = 0;
    for event in events {
        match event.kind {
            EventKind::DamageDealt => add_damage_total(&mut damage, event),
            EventKind::DamageTaken => add_incoming_total(&mut incoming, event),
            EventKind::Healing => healing += event.amount(),
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

fn summarize_encounters(events: &[GameEvent]) -> Vec<EncounterStats> {
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
            // Keep the complete boundary-to-boundary window. A second collector can log its
            // final hit just after the first collector logs intermission, so closing on the first
            // completion marker would silently discard valid party damage.
            result.push(summarize_encounter(&events[start..end]));
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

fn summarize_encounter(events: &[GameEvent]) -> EncounterStats {
    let first_completion = events
        .iter()
        .find(|event| {
            matches!(
                event.kind,
                EventKind::BossDefeated | EventKind::Intermission | EventKind::Lobby
            )
        })
        .or_else(|| {
            events
                .iter()
                .find(|event| event.kind == EventKind::WorldExited)
        })
        .map(|event| event.timestamp);
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
    let first = &events[0];
    let name = first
        .boss
        .clone()
        .or_else(|| first.stage.clone())
        .unwrap_or_else(|| "Encounter".to_owned());
    let started_at = Some(first.timestamp);
    let ended_at = first_completion
        .or_else(|| events.last().map(|event| event.timestamp))
        .map(|completed_at| {
            events
                .iter()
                .filter(|event| {
                    matches!(
                        event.kind,
                        EventKind::DamageDealt
                            | EventKind::DamageTaken
                            | EventKind::BossDefeated
                            | EventKind::Intermission
                            | EventKind::Lobby
                            | EventKind::WorldExited
                    )
                })
                .map(|event| event.timestamp)
                .max()
                .unwrap_or(completed_at)
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

    EncounterStats {
        id: format!("{}-{}", first.timestamp.format("%Y%m%d%H%M%S"), slug(&name)),
        name,
        stage: first.stage.clone(),
        phase: first.phase,
        started_at,
        ended_at,
        duration_seconds,
        players,
        outgoing,
        incoming,
        attacks,
        completed: events.iter().any(|event| {
            matches!(
                event.kind,
                EventKind::BossDefeated
                    | EventKind::Intermission
                    | EventKind::Lobby
                    | EventKind::WorldExited
            )
        }),
    }
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
    fn wall_clock_snapshots_decay_windows_and_expire_inferred_focus() {
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
        assert!(snapshot.focus.is_none());
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
