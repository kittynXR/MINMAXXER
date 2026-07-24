use crate::config::AppConfig;
use chrono::NaiveDateTime;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const ALERT_COOLDOWN: Duration = Duration::from_millis(2_500);
static BOSS_TARGET_ALERT_WAV: &[u8] = include_bytes!("../assets/boss-target-alert.wav");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BossTargetEvent {
    pub source_file: String,
    pub encounter_name: String,
    pub encounter_started_at: Option<NaiveDateTime>,
    pub target_player: String,
    pub observed_player: Option<String>,
    pub observed_at: NaiveDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BossTargetUpdate {
    Baseline(Option<BossTargetEvent>),
    Live(BossTargetEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EncounterKey {
    source_file: String,
    name: String,
    started_at: Option<NaiveDateTime>,
}

#[derive(Debug, Default)]
struct AlertGate {
    encounter: Option<EncounterKey>,
    targeting_local_player: bool,
    last_alert_at: Option<Instant>,
}

impl AlertGate {
    fn rebaseline(&mut self, event: Option<&BossTargetEvent>) {
        let Some(event) = event else {
            self.encounter = None;
            self.targeting_local_player = false;
            return;
        };
        self.encounter = Some(encounter_key(event));
        self.targeting_local_player =
            event.observed_player.as_deref() == Some(event.target_player.as_str());
    }

    fn should_alert(&mut self, event: &BossTargetEvent, enabled: bool, now: Instant) -> bool {
        let encounter = encounter_key(event);
        if self.encounter.as_ref() != Some(&encounter) {
            self.encounter = Some(encounter);
            self.targeting_local_player = false;
        }

        let targets_local_player =
            event.observed_player.as_deref() == Some(event.target_player.as_str());
        let transitioned_to_local = targets_local_player && !self.targeting_local_player;
        // Consume every edge even while disabled. Re-enabling the option while already targeted
        // should not produce a delayed warning for an ownership event which happened earlier.
        self.targeting_local_player = targets_local_player;
        if !enabled || !transitioned_to_local {
            return false;
        }

        if self
            .last_alert_at
            .is_some_and(|last| now.saturating_duration_since(last) < ALERT_COOLDOWN)
        {
            return false;
        }
        self.last_alert_at = Some(now);
        true
    }
}

fn encounter_key(event: &BossTargetEvent) -> EncounterKey {
    EncounterKey {
        source_file: event.source_file.clone(),
        name: event.encounter_name.clone(),
        started_at: event.encounter_started_at,
    }
}

pub async fn monitor(
    mut events: mpsc::UnboundedReceiver<BossTargetUpdate>,
    config: Arc<RwLock<AppConfig>>,
) {
    let mut gate = AlertGate::default();
    while let Some(update) = events.recv().await {
        let event = match update {
            BossTargetUpdate::Baseline(event) => {
                gate.rebaseline(event.as_ref());
                continue;
            }
            BossTargetUpdate::Live(event) => event,
        };
        let enabled = config
            .read()
            .map(|config| config.boss_target_alert_enabled)
            .unwrap_or(false);
        if gate.should_alert(&event, enabled, Instant::now()) {
            if play_boss_target_alert() {
                tracing::info!(
                    boss = %event.encounter_name,
                    player = %event.target_player,
                    "boss target sound alert played"
                );
            } else {
                tracing::warn!("Windows could not play the boss target sound alert");
            }
        }
    }
}

#[cfg(windows)]
fn play_boss_target_alert() -> bool {
    use windows::core::PCSTR;
    use windows::Win32::Media::Audio::{PlaySoundA, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

    // SND_MEMORY treats the first argument as an in-memory RIFF image. The bytes are static,
    // so they remain valid for the full duration of asynchronous playback.
    unsafe {
        PlaySoundA(
            PCSTR(BOSS_TARGET_ALERT_WAV.as_ptr()),
            None,
            SND_ASYNC | SND_MEMORY | SND_NODEFAULT,
        )
        .as_bool()
    }
}

#[cfg(not(windows))]
fn play_boss_target_alert() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(value: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn event(target: &str, local: Option<&str>, second: u32) -> BossTargetEvent {
        BossTargetEvent {
            source_file: "output_log_live.txt".to_owned(),
            encounter_name: "Astral Sovereign".to_owned(),
            encounter_started_at: Some(at("2026-07-23 20:00:00")),
            target_player: target.to_owned(),
            observed_player: local.map(str::to_owned),
            observed_at: at(&format!("2026-07-23 20:00:{second:02}")),
        }
    }

    #[test]
    fn embedded_alert_is_a_pcm_wave_file() {
        assert!(BOSS_TARGET_ALERT_WAV.len() > 44);
        assert_eq!(&BOSS_TARGET_ALERT_WAV[0..4], b"RIFF");
        assert_eq!(&BOSS_TARGET_ALERT_WAV[8..12], b"WAVE");
        assert_eq!(&BOSS_TARGET_ALERT_WAV[12..16], b"fmt ");
        assert_eq!(
            u16::from_le_bytes([BOSS_TARGET_ALERT_WAV[20], BOSS_TARGET_ALERT_WAV[21]]),
            1
        );
    }

    #[test]
    fn first_discrete_live_event_can_alert_local_player() {
        let now = Instant::now();
        let mut gate = AlertGate::default();

        assert!(gate.should_alert(&event("Local Player", Some("Local Player"), 1), true, now));
    }

    #[test]
    fn rapid_other_local_other_sequence_preserves_the_local_edge() {
        let now = Instant::now();
        let mut gate = AlertGate::default();

        assert!(!gate.should_alert(&event("Other Player", Some("Local Player"), 1), true, now));
        assert!(gate.should_alert(
            &event("Local Player", Some("Local Player"), 2),
            true,
            now + Duration::from_millis(10)
        ));
        assert!(!gate.should_alert(
            &event("Other Player", Some("Local Player"), 3),
            true,
            now + Duration::from_millis(20)
        ));
    }

    #[test]
    fn repeated_local_observations_do_not_replay_the_alert() {
        let now = Instant::now();
        let mut gate = AlertGate::default();

        assert!(gate.should_alert(&event("Local Player", Some("Local Player"), 1), true, now));
        assert!(!gate.should_alert(
            &event("Local Player", Some("Local Player"), 2),
            true,
            now + Duration::from_secs(3)
        ));
    }

    #[test]
    fn retarget_rearms_after_the_cooldown() {
        let now = Instant::now();
        let mut gate = AlertGate::default();

        assert!(gate.should_alert(&event("Local Player", Some("Local Player"), 1), true, now));
        assert!(!gate.should_alert(
            &event("Other Player", Some("Local Player"), 2),
            true,
            now + Duration::from_secs(1)
        ));
        assert!(!gate.should_alert(
            &event("Local Player", Some("Local Player"), 3),
            true,
            now + Duration::from_secs(2)
        ));
        assert!(!gate.should_alert(
            &event("Other Player", Some("Local Player"), 4),
            true,
            now + Duration::from_secs(3)
        ));
        assert!(gate.should_alert(
            &event("Local Player", Some("Local Player"), 5),
            true,
            now + Duration::from_secs(4)
        ));
    }

    #[test]
    fn disabled_alert_consumes_the_target_edge() {
        let now = Instant::now();
        let mut gate = AlertGate::default();
        let local = event("Local Player", Some("Local Player"), 1);

        assert!(!gate.should_alert(&local, false, now));
        assert!(!gate.should_alert(&local, true, now + Duration::from_secs(3)));
    }

    #[test]
    fn late_local_identity_can_complete_a_pending_target_match() {
        let now = Instant::now();
        let mut gate = AlertGate::default();

        assert!(!gate.should_alert(&event("Local Player", None, 1), true, now));
        assert!(gate.should_alert(
            &event("Local Player", Some("Local Player"), 1),
            true,
            now + Duration::from_millis(10)
        ));
    }

    #[test]
    fn a_new_encounter_rearms_even_when_the_target_name_is_unchanged() {
        let now = Instant::now();
        let mut gate = AlertGate::default();
        let first = event("Local Player", Some("Local Player"), 1);
        let mut next = event("Local Player", Some("Local Player"), 5);
        next.encounter_started_at = Some(at("2026-07-23 20:01:00"));

        assert!(gate.should_alert(&first, true, now));
        assert!(gate.should_alert(&next, true, now + Duration::from_secs(60)));
    }

    #[test]
    fn silent_replay_baseline_consumes_an_existing_local_target() {
        let now = Instant::now();
        let mut gate = AlertGate::default();
        let local = event("Local Player", Some("Local Player"), 1);
        gate.rebaseline(Some(&local));

        let refresh = event("Local Player", Some("Local Player"), 2);
        assert!(!gate.should_alert(&refresh, true, now));

        assert!(!gate.should_alert(
            &event("Other Player", Some("Local Player"), 3),
            true,
            now + Duration::from_secs(1)
        ));
        assert!(gate.should_alert(
            &event("Local Player", Some("Local Player"), 4),
            true,
            now + Duration::from_secs(2)
        ));
    }
}
