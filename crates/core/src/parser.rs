use crate::model::{EventKind, GameEvent, ParserCoverage};
use chrono::{NaiveDateTime, ParseError};
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

static PLAYER_LOCAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"Initialized PlayerAPI \"(?P<name>.+)\" is local"#).unwrap());
static PLAYER_JOIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[Behaviour\] OnPlayerJoined (?P<name>.+) \((?P<id>usr_[^)]+)\)").unwrap()
});
static PLAYER_LEFT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[Behaviour\] OnPlayerLeft (?P<name>.+) \((?P<id>usr_[^)]+)\)").unwrap()
});
static SESSION_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"ECLIPTICA (?:loaded|saving) SESSION ID (?P<id>\d+)").unwrap());
static STAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"ECLIPTICA - now in stage: (?P<stage>.+?) on phase: (?P<phase>-?[0-9.]+) as class: (?P<class>.+)$",
    )
    .unwrap()
});
static BOSS_START: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"ECLIPTICA - now fighting boss: (?P<boss>.+?)(?:\(Clone\))? on phase: (?P<phase>-?[0-9.]+)$",
    )
    .unwrap()
});
static DAMAGE_DEALT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Dealing (?P<amount>[0-9]+(?:\.[0-9]+)?) (?P<kind>[A-Za-z_-]+) damage").unwrap()
});
static DAMAGE_TAKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"damage has been taken: (?P<amount>-?[0-9]+(?:\.[0-9]+)?), from source:\s*(?P<source>.*)$",
    )
    .unwrap()
});
static BOSS_DEAD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Boss (?P<boss>.+?) dead, personal damage dealt:").unwrap());
static DAMAGE_SUMMARY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?P<kind>STRIKE|NON-STRIKE) DMG: (?P<amount>[0-9]+(?:\.[0-9]+)?)").unwrap()
});
static ENEMY_SPAWN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Initializing Enemy POOL ID(?P<pool>\d+) as ENEMY ID (?P<enemy>\d+)").unwrap()
});
static ENEMY_RETIRE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Retiring Enemy POOL ID(?P<pool>\d+)").unwrap());
static OWNERSHIP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"ownership of (?P<entity>.+?) transferred to (?P<name>.+)$").unwrap()
});
static WORLD_INSTANCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:Destination (?:requested|fetching|set):|\[Behaviour\] (?:Joining|Rejoining local world:)) (?P<instance>wrld_[^\s]+)").unwrap()
});
static ENEMY_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[Behaviour\] No targets to encode on (?P<entity>.+?)(?:\(Clone\))?$").unwrap()
});
static STAGE_EVENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^Advancing Stage Event: (?P<value>-?\d+)$").unwrap());
static STAGE_PROGRESS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^Advancing Stage Progress to: (?P<value>-?\d+)$").unwrap());
static SPAWN_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^spawn token, (?P<enabled>True|False), (?P<id>-?\d+)$").unwrap());
static ENEMY_DIAGNOSTIC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^ENEMY (?P<entity>.+?) (?P<diagnostic>INTRO SLAP\.|INVALID! REINITIALIZING!!)$")
        .unwrap()
});

#[derive(Debug, Clone)]
struct PendingBossSummary {
    boss: String,
    ttl: u8,
    strike: f64,
    key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedLogLine<'a> {
    pub timestamp: NaiveDateTime,
    pub level: &'a str,
    pub message: &'a str,
}

/// Splits VRChat's standard timestamp/severity/message envelope without allocating.
pub fn parse_log_line(line: &str) -> Result<Option<ParsedLogLine<'_>>, ParseError> {
    if line.len() < 20 || line.as_bytes().get(4).is_none_or(|c| *c != b'.') {
        return Ok(None);
    }
    let timestamp = NaiveDateTime::parse_from_str(&line[..19], "%Y.%m.%d %H:%M:%S")?;
    let remainder = line[19..].trim_start();
    let Some((level, message)) = remainder.split_once("-  ") else {
        return Ok(None);
    };
    Ok(Some(ParsedLogLine {
        timestamp,
        level: level.trim(),
        message: message.trim(),
    }))
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum ParseOutcome {
    Event(GameEvent),
    RelevantButUnknown,
    Ignored,
}

#[derive(Debug, Clone)]
pub struct EclipticaParser {
    sequence: u64,
    in_world: bool,
    world: Option<String>,
    instance: Option<String>,
    pending_instance: Option<String>,
    local_player: Option<String>,
    session_id: Option<u32>,
    stage: Option<String>,
    class_name: Option<String>,
    boss: Option<String>,
    phase: Option<f64>,
    pending_boss_summary: Option<PendingBossSummary>,
    completed_encounters: HashSet<String>,
    active_enemy_pools: HashSet<u32>,
    last_enemy_spawn: Option<(u32, u32, NaiveDateTime)>,
    last_boss_start: Option<(String, Option<f64>, NaiveDateTime)>,
    suppress_boss_until_stage: bool,
    coverage: ParserCoverage,
}

impl Default for EclipticaParser {
    fn default() -> Self {
        Self::new()
    }
}

impl EclipticaParser {
    pub fn new() -> Self {
        Self {
            sequence: 0,
            in_world: false,
            world: None,
            instance: None,
            pending_instance: None,
            local_player: None,
            session_id: None,
            stage: None,
            class_name: None,
            boss: None,
            phase: None,
            pending_boss_summary: None,
            completed_encounters: HashSet::new(),
            active_enemy_pools: HashSet::new(),
            last_enemy_spawn: None,
            last_boss_start: None,
            suppress_boss_until_stage: false,
            coverage: ParserCoverage::default(),
        }
    }

    pub fn coverage(&self) -> &ParserCoverage {
        &self.coverage
    }

    pub fn local_player(&self) -> Option<&str> {
        self.local_player.as_deref()
    }

    pub fn session_id(&self) -> Option<u32> {
        self.session_id
    }

    pub fn process_line(&mut self, line: &str) -> ParseOutcome {
        self.coverage.lines_seen += 1;
        let parsed = match parse_log_line(line) {
            Ok(Some(parsed)) => parsed,
            _ => return ParseOutcome::Ignored,
        };
        self.coverage.timestamped_lines += 1;
        let message = parsed.message;

        if let Some(pending) = self.pending_boss_summary.as_mut() {
            if pending.ttl == 0 {
                self.pending_boss_summary = None;
            } else {
                pending.ttl -= 1;
            }
        }

        if let Some(caps) = WORLD_INSTANCE.captures(message) {
            let instance = caps["instance"].to_owned();
            self.pending_instance = Some(instance.clone());
            // VRChat commonly writes `Entering Room` before its more useful `Joining wrld_…`
            // line. In that ordering, update the active visit as well as the next-visit hint so
            // every event after the join carries the concrete instance identifier.
            if self.in_world
                && (message.starts_with("[Behaviour] Joining ")
                    || message.starts_with("[Behaviour] Rejoining local world: "))
            {
                self.instance = Some(instance);
            }
        }

        if let Some(caps) = PLAYER_LOCAL.captures(message) {
            let name = caps["name"].to_owned();
            self.local_player = Some(name.clone());
            // PlayerAPI is initialized in every VRChat world. Preserve the parser context, but
            // only persist an event while Ecliptica is active so global initialization does not
            // manufacture empty run reports.
            if !self.in_world {
                return ParseOutcome::Ignored;
            }
            return self.emit(
                parsed.timestamp,
                EventKind::LocalPlayerIdentified,
                line,
                |event| {
                    event.player = Some(name);
                },
            );
        }

        if let Some(world) = message.strip_prefix("[Behaviour] Entering Room: ") {
            let is_ecliptica = world.to_ascii_lowercase().contains("ecliptica");
            self.in_world = is_ecliptica;
            self.world = is_ecliptica.then(|| world.to_owned());
            self.instance = is_ecliptica
                .then(|| self.pending_instance.clone())
                .flatten();
            self.stage = None;
            self.class_name = None;
            self.boss = None;
            self.phase = None;
            self.session_id = None;
            self.suppress_boss_until_stage = false;
            self.active_enemy_pools.clear();
            self.completed_encounters.clear();
            self.last_boss_start = None;
            if is_ecliptica {
                return self.emit(parsed.timestamp, EventKind::WorldEntered, line, |event| {
                    event.world = Some(world.to_owned());
                    event.message = Some("Ecliptica world entered".to_owned());
                });
            }
            return ParseOutcome::Ignored;
        }

        if message.contains("[Behaviour] OnLeftRoom")
            || message.contains("[Behaviour] Leaving Room")
        {
            if self.in_world {
                let result = self.emit(parsed.timestamp, EventKind::WorldExited, line, |_| {});
                self.in_world = false;
                self.world = None;
                self.instance = None;
                self.stage = None;
                self.class_name = None;
                self.boss = None;
                self.phase = None;
                self.session_id = None;
                self.active_enemy_pools.clear();
                return result;
            }
            return ParseOutcome::Ignored;
        }

        if let Some(caps) = PLAYER_JOIN.captures(message) {
            if self.in_world {
                let name = caps["name"].to_owned();
                let user_id = caps["id"].to_owned();
                return self.emit(parsed.timestamp, EventKind::PlayerJoined, line, |event| {
                    event.target = Some(name);
                    event.user_id = Some(user_id);
                });
            }
            return ParseOutcome::Ignored;
        }

        if let Some(caps) = PLAYER_LEFT.captures(message) {
            if self.in_world {
                let name = caps["name"].to_owned();
                let user_id = caps["id"].to_owned();
                return self.emit(parsed.timestamp, EventKind::PlayerLeft, line, |event| {
                    event.target = Some(name);
                    event.user_id = Some(user_id);
                });
            }
            return ParseOutcome::Ignored;
        }

        // Any explicit ECLIPTICA line is enough to recover parser context when an import starts
        // in the middle of a world visit.
        if message.starts_with("ECLIPTICA") {
            self.in_world = true;
            self.world.get_or_insert_with(|| "Ecliptica".to_owned());
        }

        if message.contains("ECLIPTICA loaded blank session ID")
            || message.contains("ECLIPTICA session ID does not match")
        {
            self.session_id = None;
            // Retain the invalidation in normalized data. It lets historical analysis reject a
            // numeric session that was logged immediately before Ecliptica declared it stale.
            return self.emit(parsed.timestamp, EventKind::GameMessage, line, |event| {
                event.message = Some("session_invalidated".to_owned());
            });
        }

        if let Some(caps) = SESSION_ID.captures(message) {
            let session_id = caps["id"].parse::<u32>().ok();
            self.session_id = session_id;
            let kind = if message.contains("saving") {
                EventKind::SessionSaved
            } else {
                EventKind::SessionLoaded
            };
            return self.emit(parsed.timestamp, kind, line, |_| {});
        }

        if let Some(caps) = STAGE.captures(message) {
            let stage = caps["stage"].to_owned();
            let class_name = caps["class"].to_owned();
            let phase = caps["phase"].parse::<f64>().ok();
            self.stage = Some(stage.clone());
            self.class_name = Some(class_name.clone());
            self.phase = phase;
            self.boss = None;
            self.suppress_boss_until_stage = false;
            self.last_boss_start = None;
            return self.emit(parsed.timestamp, EventKind::StageEntered, line, |event| {
                event.stage = Some(stage);
                event.class_name = Some(class_name);
                event.phase = phase;
            });
        }

        if let Some(caps) = BOSS_START.captures(message) {
            if self.suppress_boss_until_stage {
                return ParseOutcome::Ignored;
            }
            let boss = clean_clone_suffix(&caps["boss"]);
            let phase = caps["phase"].parse::<f64>().ok();
            if self
                .last_boss_start
                .as_ref()
                .is_some_and(|(previous, previous_phase, at)| {
                    previous == &boss
                        && *previous_phase == phase
                        && (parsed.timestamp - *at).num_seconds().unsigned_abs() < 5
                })
            {
                return ParseOutcome::Ignored;
            }
            self.last_boss_start = Some((boss.clone(), phase, parsed.timestamp));
            self.boss = Some(boss.clone());
            self.phase = phase;
            return self.emit(parsed.timestamp, EventKind::BossStarted, line, |event| {
                event.boss = Some(boss);
                event.phase = phase;
            });
        }

        if message == "ECLIPTICA - now in intermission" {
            self.boss = None;
            self.phase = None;
            self.suppress_boss_until_stage = true;
            return self.emit(parsed.timestamp, EventKind::Intermission, line, |_| {});
        }

        if message == "ECLIPTICA - now in lobby" {
            self.boss = None;
            self.stage = None;
            self.class_name = None;
            self.phase = None;
            self.suppress_boss_until_stage = true;
            return self.emit(parsed.timestamp, EventKind::Lobby, line, |_| {});
        }

        if let Some(caps) = DAMAGE_DEALT.captures(message) {
            if self.in_world {
                let amount = caps["amount"].parse::<f64>().ok();
                let damage_type = caps["kind"].to_ascii_lowercase();
                return self.emit(parsed.timestamp, EventKind::DamageDealt, line, |event| {
                    event.amount = amount;
                    event.damage_type = Some(damage_type);
                });
            }
            return ParseOutcome::Ignored;
        }

        if let Some(caps) = DAMAGE_TAKEN.captures(message) {
            if self.in_world {
                let amount = caps["amount"].parse::<f64>().ok();
                // An explicitly empty source is meaningful forensic data (usually a damage-over-
                // time tick), so preserve it instead of inventing an attacker name.
                let source = caps
                    .name("source")
                    .map(|value| value.as_str().trim())
                    .unwrap_or("")
                    .to_owned();
                return self.emit(parsed.timestamp, EventKind::DamageTaken, line, |event| {
                    event.amount = amount;
                    event.source = Some(source);
                });
            }
            return ParseOutcome::Ignored;
        }

        if let Some(caps) = BOSS_DEAD.captures(message) {
            let boss = caps["boss"].to_owned();
            let key = self.encounter_key(&boss);
            self.pending_boss_summary = Some(PendingBossSummary {
                boss,
                ttl: 4,
                strike: 0.0,
                key,
            });
            // The header is heavily duplicated by Ecliptica's buffered/save path. Wait for the
            // following non-zero personal summary before declaring a canonical completion.
            return ParseOutcome::Ignored;
        }

        if let Some(caps) = DAMAGE_SUMMARY.captures(message) {
            if let Some(mut pending) = self.pending_boss_summary.take() {
                let damage_type = caps["kind"].to_ascii_lowercase();
                let amount = caps["amount"].parse::<f64>().unwrap_or_default();
                if damage_type == "strike" {
                    pending.strike = amount;
                    let boss = pending.boss.clone();
                    self.pending_boss_summary = Some(pending);
                    return self.emit(
                        parsed.timestamp,
                        EventKind::BossDamageSummary,
                        line,
                        |event| {
                            event.boss = Some(boss);
                            event.amount = Some(amount);
                            event.damage_type = Some(damage_type);
                        },
                    );
                }
                let total = pending.strike + amount;
                if total > 0.0 && self.completed_encounters.insert(pending.key) {
                    let boss = pending.boss;
                    return self.emit(parsed.timestamp, EventKind::BossDefeated, line, |event| {
                        event.boss = Some(boss);
                        event.amount = Some(total);
                        event.damage_type = Some("personal_total".to_owned());
                        event.message = Some("Canonical non-zero personal boss summary".to_owned());
                    });
                }
                return ParseOutcome::Ignored;
            }
        }

        if let Some(caps) = ENEMY_SPAWN.captures(message) {
            if self.in_world {
                let pool_id = caps["pool"].parse::<u32>().ok();
                let enemy_id = caps["enemy"].parse::<u32>().ok();
                if let Some(pool_id) = pool_id {
                    self.active_enemy_pools.insert(pool_id);
                }
                if let (Some(pool_id), Some(enemy_id)) = (pool_id, enemy_id) {
                    self.last_enemy_spawn = Some((pool_id, enemy_id, parsed.timestamp));
                }
                return self.emit(parsed.timestamp, EventKind::EnemySpawned, line, |event| {
                    event.pool_id = pool_id;
                    event.enemy_id = enemy_id;
                });
            }
        }

        if let Some(caps) = ENEMY_RETIRE.captures(message) {
            if self.in_world {
                let pool_id = caps["pool"].parse::<u32>().ok();
                if !pool_id.is_some_and(|pool_id| self.active_enemy_pools.remove(&pool_id)) {
                    return ParseOutcome::Ignored;
                }
                return self.emit(parsed.timestamp, EventKind::EnemyRetired, line, |event| {
                    event.pool_id = pool_id;
                });
            }
        }

        if let Some(caps) = OWNERSHIP.captures(message) {
            if self.in_world {
                let entity = caps["entity"].to_owned();
                let target = caps["name"].to_owned();
                return self.emit(
                    parsed.timestamp,
                    EventKind::OwnershipTransferred,
                    line,
                    |event| {
                        event.entity = Some(entity);
                        event.target = Some(target);
                    },
                );
            }
        }

        if let Some(caps) = ENEMY_NAME.captures(message) {
            if self.in_world {
                let entity = clean_clone_suffix(&caps["entity"]);
                let linked = self.last_enemy_spawn.filter(|(_, _, timestamp)| {
                    (parsed.timestamp - *timestamp).num_seconds().unsigned_abs() <= 2
                });
                return self.emit(parsed.timestamp, EventKind::EnemyNamed, line, |event| {
                    event.entity = Some(entity);
                    if let Some((pool_id, enemy_id, _)) = linked {
                        event.pool_id = Some(pool_id);
                        event.enemy_id = Some(enemy_id);
                    }
                });
            }
        }

        if let Some(caps) = STAGE_EVENT.captures(message) {
            if self.in_world {
                let value = caps["value"].parse::<f64>().ok();
                return self.emit(
                    parsed.timestamp,
                    EventKind::StageEventAdvanced,
                    line,
                    |event| {
                        event.amount = value;
                    },
                );
            }
        }

        if let Some(caps) = STAGE_PROGRESS.captures(message) {
            if self.in_world {
                let value = caps["value"].parse::<f64>().ok();
                return self.emit(
                    parsed.timestamp,
                    EventKind::StageProgressAdvanced,
                    line,
                    |event| event.amount = value,
                );
            }
        }

        if let Some(caps) = SPAWN_TOKEN.captures(message) {
            if self.in_world {
                let id = caps["id"].to_owned();
                let enabled = caps["enabled"].to_ascii_lowercase();
                return self.emit(parsed.timestamp, EventKind::SpawnToken, line, |event| {
                    event.entity = Some(id);
                    event.message = Some(enabled);
                });
            }
        }

        if let Some(caps) = ENEMY_DIAGNOSTIC.captures(message) {
            if self.in_world {
                let entity = caps["entity"].to_owned();
                let diagnostic = caps["diagnostic"].to_owned();
                return self.emit(
                    parsed.timestamp,
                    EventKind::EnemyDiagnostic,
                    line,
                    |event| {
                        event.entity = Some(entity);
                        event.message = Some(diagnostic);
                    },
                );
            }
        }

        // These messages were present in the audited build but carry no additional combat
        // fields. Classifying them explicitly keeps parser coverage focused on genuinely new
        // telemetry without turning the repeated boss-tracking diagnostic into a fake defeat.
        if is_known_non_telemetry(message) {
            return ParseOutcome::Ignored;
        }

        if self.in_world && is_probably_gameplay_line(message) {
            self.coverage.relevant_unparsed += 1;
            return ParseOutcome::RelevantButUnknown;
        }

        ParseOutcome::Ignored
    }

    fn emit<F>(
        &mut self,
        timestamp: NaiveDateTime,
        kind: EventKind,
        raw: &str,
        customize: F,
    ) -> ParseOutcome
    where
        F: FnOnce(&mut GameEvent),
    {
        self.sequence += 1;
        let mut event = GameEvent {
            sequence: self.sequence,
            timestamp,
            kind,
            player: self.local_player.clone(),
            session_id: self.session_id,
            world: self.world.clone(),
            instance: self
                .instance
                .clone()
                .or_else(|| self.pending_instance.clone()),
            stage: self.stage.clone(),
            class_name: self.class_name.clone(),
            boss: self.boss.clone(),
            phase: self.phase,
            amount: None,
            damage_type: None,
            source: None,
            target: None,
            entity: None,
            pool_id: None,
            enemy_id: None,
            user_id: None,
            message: None,
            raw: raw.to_owned(),
        };
        customize(&mut event);
        self.coverage.events_emitted += 1;
        ParseOutcome::Event(event)
    }

    fn encounter_key(&self, boss: &str) -> String {
        format!(
            "{}|{}|{}|{}|{}",
            self.session_id
                .map(|value| value.to_string())
                .unwrap_or_default(),
            self.instance.as_deref().unwrap_or_default(),
            self.stage.as_deref().unwrap_or_default(),
            boss,
            self.phase
                .map(|value| value.to_string())
                .unwrap_or_default()
        )
    }
}

fn clean_clone_suffix(value: &str) -> String {
    value.trim().trim_end_matches("(Clone)").to_owned()
}

fn is_probably_gameplay_line(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("ecliptica")
        || lower.contains(" enemy ")
        || lower.starts_with("enemy ")
        || lower.contains("boss ")
        || lower.contains("damage")
        || lower.contains("debuff")
        || lower.contains("buff")
        || lower.contains("heal")
        || lower.contains("advancing stage")
        || lower.contains("spawn token")
        || lower.contains("ownership of")
        || lower.contains("pool id")
        || lower.contains("no targets to encode")
}

fn is_known_non_telemetry(message: &str) -> bool {
    matches!(
        message,
        "[Behaviour] Requesting buffered events"
            | "[Behaviour] Executing Buffered Events"
            | "ECLIPTICA Loading Settings..."
            | "ECLIPTICA successfully loaded SESSION data."
            | "Tracking boss as defeated in-run."
    ) || message.starts_with("[Behaviour] Joining or Creating Room: Ecliptica")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parser_in_world() -> EclipticaParser {
        let mut parser = EclipticaParser::new();
        parser.process_line(
            "2026.07.21 20:17:48 Debug      -  [Behaviour] Entering Room: Ecliptica - Demo Playtest",
        );
        parser.process_line(
            "2026.07.21 20:18:01 Debug      -  [Behaviour] Initialized PlayerAPI \"PlayerOne\" is local",
        );
        parser
    }

    #[test]
    fn parses_live_outgoing_and_incoming_damage() {
        let mut parser = parser_in_world();
        let ParseOutcome::Event(outgoing) =
            parser.process_line("2026.07.21 20:55:49 Debug      -  Dealing 138 NON-STRIKE damage")
        else {
            panic!("expected outgoing event");
        };
        assert_eq!(outgoing.kind, EventKind::DamageDealt);
        assert_eq!(outgoing.amount, Some(138.0));
        assert_eq!(outgoing.damage_type.as_deref(), Some("non-strike"));

        let ParseOutcome::Event(incoming) = parser.process_line(
            "2026.07.21 20:56:05 Debug      -  damage has been taken: 28, from source: attack_icicle",
        ) else {
            panic!("expected incoming event");
        };
        assert_eq!(incoming.kind, EventKind::DamageTaken);
        assert_eq!(incoming.source.as_deref(), Some("attack_icicle"));
    }

    #[test]
    fn associates_multiline_boss_summary() {
        let mut parser = parser_in_world();
        parser.process_line(
            "2026.07.21 20:58:07 Debug      -  Boss Nan dead, personal damage dealt: ",
        );
        let ParseOutcome::Event(summary) =
            parser.process_line("2026.07.21 20:58:07 Debug      -  STRIKE DMG: 2569")
        else {
            panic!("expected summary event");
        };
        assert_eq!(summary.kind, EventKind::BossDamageSummary);
        assert_eq!(summary.boss.as_deref(), Some("Nan"));
        assert_eq!(summary.amount, Some(2569.0));
    }

    #[test]
    fn parses_stage_class_and_fractional_phase() {
        let mut parser = parser_in_world();
        let ParseOutcome::Event(event) = parser.process_line(
            "2026.07.21 21:10:54 Debug      -  ECLIPTICA - now in stage: Stage_CalamityPalace on phase: 0.1178404 as class: Spellhammer",
        ) else {
            panic!("expected stage event");
        };
        assert_eq!(event.stage.as_deref(), Some("Stage_CalamityPalace"));
        assert_eq!(event.class_name.as_deref(), Some("Spellhammer"));
        assert_eq!(event.phase, Some(0.1178404));
    }

    #[test]
    fn joining_after_room_entry_updates_active_instance() {
        let mut parser = parser_in_world();
        assert!(matches!(
            parser.process_line(
                "2026.07.21 20:18:02 Debug      -  [Behaviour] Joining wrld_0fb88df3-2057-4c2f-8e06-e948864378fd:12345~region(us)"
            ),
            ParseOutcome::Ignored
        ));

        let ParseOutcome::Event(event) =
            parser.process_line("2026.07.21 20:18:03 Debug      -  Dealing 50 STRIKE damage")
        else {
            panic!("expected damage event");
        };
        assert_eq!(
            event.instance.as_deref(),
            Some("wrld_0fb88df3-2057-4c2f-8e06-e948864378fd:12345~region(us)")
        );
    }

    #[test]
    fn global_player_initialization_updates_context_without_emitting_event() {
        let mut parser = EclipticaParser::new();
        let outcome = parser.process_line(
            "2026.07.21 20:00:00 Debug      -  [Behaviour] Initialized PlayerAPI \"PlayerOne\" is local",
        );
        assert_eq!(outcome, ParseOutcome::Ignored);
        assert_eq!(parser.local_player(), Some("PlayerOne"));
        assert_eq!(parser.coverage().events_emitted, 0);

        let ParseOutcome::Event(entered) = parser.process_line(
            "2026.07.21 20:01:00 Debug      -  [Behaviour] Entering Room: Ecliptica - Demo Playtest",
        ) else {
            panic!("expected world event");
        };
        assert_eq!(entered.player.as_deref(), Some("PlayerOne"));
    }

    #[test]
    fn records_session_invalidation_for_historical_segmentation() {
        let mut parser = parser_in_world();
        let ParseOutcome::Event(loaded) = parser
            .process_line("2026.07.21 20:18:03 Debug      -  ECLIPTICA loaded SESSION ID 16367")
        else {
            panic!("expected session event");
        };
        assert_eq!(loaded.session_id, Some(16367));

        let ParseOutcome::Event(invalidated) = parser
            .process_line("2026.07.21 20:18:04 Debug      -  ECLIPTICA session ID does not match")
        else {
            panic!("expected invalidation event");
        };
        assert_eq!(invalidated.kind, EventKind::GameMessage);
        assert_eq!(invalidated.session_id, None);
        assert_eq!(invalidated.message.as_deref(), Some("session_invalidated"));
    }
}
