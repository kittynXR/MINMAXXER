use crate::audio_output;
use crate::config::AppConfig;
use chrono::NaiveDateTime;
use minmaxxer_core::BossTargetObservationCause;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const ALERT_COOLDOWN: Duration = Duration::from_millis(2_500);
static BOSS_TARGET_ALERT_WAV: &[u8] = include_bytes!("../assets/boss-target-alert.wav");
static BOSS_TARGET_RELEASED_WAV: &[u8] = include_bytes!("../assets/boss-target-released.wav");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BossTargetEvent {
    pub source_file: String,
    pub encounter_name: String,
    pub encounter_started_at: Option<NaiveDateTime>,
    pub target_player: String,
    pub observed_player: Option<String>,
    pub observed_at: NaiveDateTime,
    pub cause: BossTargetObservationCause,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AlertCue {
    Targeted,
    Released,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum TargetState {
    #[default]
    Unknown,
    Local,
    Other,
}

#[derive(Debug, Default)]
struct AlertGate {
    encounter: Option<EncounterKey>,
    target_state: TargetState,
    last_targeted_at: Option<Instant>,
    last_released_at: Option<Instant>,
}

impl AlertGate {
    fn rebaseline(&mut self, event: Option<&BossTargetEvent>) {
        let Some(event) = event else {
            self.encounter = None;
            self.target_state = TargetState::Unknown;
            return;
        };
        self.encounter = Some(encounter_key(event));
        self.target_state = target_state(event);
    }

    fn observe(
        &mut self,
        event: &BossTargetEvent,
        enabled: bool,
        now: Instant,
    ) -> Option<AlertCue> {
        let encounter = encounter_key(event);
        if self.encounter.as_ref() != Some(&encounter) {
            self.encounter = Some(encounter);
            self.target_state = TargetState::Unknown;
        }

        let previous = self.target_state;
        let current = target_state(event);
        // Consume every edge even while disabled. Re-enabling the option while already targeted
        // should not produce a delayed warning for an ownership event which happened earlier.
        self.target_state = current;
        if !enabled {
            return None;
        }

        let cue = match (previous, current, event.cause) {
            (TargetState::Unknown | TargetState::Other, TargetState::Local, _) => {
                AlertCue::Targeted
            }
            (
                TargetState::Local,
                TargetState::Other,
                BossTargetObservationCause::OwnershipTransfer,
            ) => AlertCue::Released,
            _ => return None,
        };
        let last_alert = match cue {
            AlertCue::Targeted => &mut self.last_targeted_at,
            AlertCue::Released => &mut self.last_released_at,
        };
        if last_alert.is_some_and(|last| now.saturating_duration_since(last) < ALERT_COOLDOWN) {
            return None;
        }
        *last_alert = Some(now);
        Some(cue)
    }
}

fn target_state(event: &BossTargetEvent) -> TargetState {
    let Some(local_player) = event.observed_player.as_deref() else {
        return TargetState::Unknown;
    };
    if event.target_player.trim().is_empty() {
        return TargetState::Unknown;
    }
    if local_player == event.target_player {
        TargetState::Local
    } else {
        TargetState::Other
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
        let (enabled, audio_output_device_id) = config
            .read()
            .map(|config| {
                (
                    config.boss_target_alert_enabled,
                    config.audio_output_device_id.clone(),
                )
            })
            .unwrap_or_default();
        let Some(cue) = gate.observe(&event, enabled, Instant::now()) else {
            continue;
        };
        let (sound, description) = match cue {
            AlertCue::Targeted => (BOSS_TARGET_ALERT_WAV, "boss target warning"),
            AlertCue::Released => (BOSS_TARGET_RELEASED_WAV, "boss target all-clear"),
        };
        let boss = event.encounter_name.clone();
        let player = event.target_player.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("minmaxxer-audio-cue".to_owned())
            .spawn(
                move || match audio_output::play(sound, &audio_output_device_id) {
                    Ok(()) => tracing::info!(
                        %boss,
                        %player,
                        cue = description,
                        "boss target audio cue played"
                    ),
                    Err(error) => tracing::warn!(
                        %error,
                        cue = description,
                        "Windows could not play boss target audio cue"
                    ),
                },
            )
        {
            tracing::warn!(%error, cue = description, "could not start audio-cue playback");
        }
    }
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
            cause: BossTargetObservationCause::OwnershipTransfer,
        }
    }

    fn assert_pcm_wave(sound: &[u8]) {
        assert!(sound.len() > 44);
        assert_eq!(&sound[0..4], b"RIFF");
        assert_eq!(&sound[8..12], b"WAVE");
        assert_eq!(&sound[12..16], b"fmt ");
        assert_eq!(u16::from_le_bytes([sound[20], sound[21]]), 1);
        assert_eq!(u16::from_le_bytes([sound[22], sound[23]]), 1);
        assert_eq!(
            u32::from_le_bytes([sound[24], sound[25], sound[26], sound[27]]),
            44_100
        );
        assert_eq!(u16::from_le_bytes([sound[34], sound[35]]), 16);
    }

    #[test]
    fn embedded_alerts_are_pcm_wave_files() {
        assert_pcm_wave(BOSS_TARGET_ALERT_WAV);
        assert_pcm_wave(BOSS_TARGET_RELEASED_WAV);
        assert_ne!(BOSS_TARGET_ALERT_WAV, BOSS_TARGET_RELEASED_WAV);
    }

    #[test]
    fn first_discrete_live_event_can_alert_local_player() {
        let now = Instant::now();
        let mut gate = AlertGate::default();

        assert_eq!(
            gate.observe(&event("Local Player", Some("Local Player"), 1), true, now),
            Some(AlertCue::Targeted)
        );
    }

    #[test]
    fn rapid_other_local_other_sequence_preserves_both_cues() {
        let now = Instant::now();
        let mut gate = AlertGate::default();

        assert_eq!(
            gate.observe(&event("Other Player", Some("Local Player"), 1), true, now),
            None
        );
        assert_eq!(
            gate.observe(
                &event("Local Player", Some("Local Player"), 2),
                true,
                now + Duration::from_millis(10)
            ),
            Some(AlertCue::Targeted)
        );
        assert_eq!(
            gate.observe(
                &event("Other Player", Some("Local Player"), 3),
                true,
                now + Duration::from_millis(20)
            ),
            Some(AlertCue::Released)
        );
    }

    #[test]
    fn repeated_local_observations_do_not_replay_the_alert() {
        let now = Instant::now();
        let mut gate = AlertGate::default();

        assert_eq!(
            gate.observe(&event("Local Player", Some("Local Player"), 1), true, now),
            Some(AlertCue::Targeted)
        );
        assert_eq!(
            gate.observe(
                &event("Local Player", Some("Local Player"), 2),
                true,
                now + Duration::from_secs(3)
            ),
            None
        );
    }

    #[test]
    fn retarget_rearms_after_the_cooldown() {
        let now = Instant::now();
        let mut gate = AlertGate::default();

        assert_eq!(
            gate.observe(&event("Local Player", Some("Local Player"), 1), true, now),
            Some(AlertCue::Targeted)
        );
        assert_eq!(
            gate.observe(
                &event("Other Player", Some("Local Player"), 2),
                true,
                now + Duration::from_secs(1)
            ),
            Some(AlertCue::Released)
        );
        assert_eq!(
            gate.observe(
                &event("Local Player", Some("Local Player"), 3),
                true,
                now + Duration::from_secs(2)
            ),
            None
        );
        assert_eq!(
            gate.observe(
                &event("Other Player", Some("Local Player"), 4),
                true,
                now + Duration::from_secs(3)
            ),
            None
        );
        assert_eq!(
            gate.observe(
                &event("Local Player", Some("Local Player"), 5),
                true,
                now + Duration::from_secs(4)
            ),
            Some(AlertCue::Targeted)
        );
    }

    #[test]
    fn disabled_alert_consumes_the_target_edge() {
        let now = Instant::now();
        let mut gate = AlertGate::default();
        let local = event("Local Player", Some("Local Player"), 1);

        assert_eq!(gate.observe(&local, false, now), None);
        assert_eq!(
            gate.observe(&local, true, now + Duration::from_secs(3)),
            None
        );
    }

    #[test]
    fn late_local_identity_can_complete_a_pending_target_match() {
        let now = Instant::now();
        let mut gate = AlertGate::default();

        assert_eq!(
            gate.observe(&event("Local Player", None, 1), true, now),
            None
        );
        let mut identified = event("Local Player", Some("Local Player"), 1);
        identified.cause = BossTargetObservationCause::LocalIdentity;
        assert_eq!(
            gate.observe(&identified, true, now + Duration::from_millis(10)),
            Some(AlertCue::Targeted)
        );
    }

    #[test]
    fn a_new_encounter_rearms_even_when_the_target_name_is_unchanged() {
        let now = Instant::now();
        let mut gate = AlertGate::default();
        let first = event("Local Player", Some("Local Player"), 1);
        let mut next = event("Local Player", Some("Local Player"), 5);
        next.encounter_started_at = Some(at("2026-07-23 20:01:00"));

        assert_eq!(gate.observe(&first, true, now), Some(AlertCue::Targeted));
        assert_eq!(
            gate.observe(&next, true, now + Duration::from_secs(60)),
            Some(AlertCue::Targeted)
        );
    }

    #[test]
    fn silent_replay_baseline_consumes_an_existing_local_target() {
        let now = Instant::now();
        let mut gate = AlertGate::default();
        let local = event("Local Player", Some("Local Player"), 1);
        gate.rebaseline(Some(&local));

        let refresh = event("Local Player", Some("Local Player"), 2);
        assert_eq!(gate.observe(&refresh, true, now), None);

        assert_eq!(
            gate.observe(
                &event("Other Player", Some("Local Player"), 3),
                true,
                now + Duration::from_secs(1)
            ),
            Some(AlertCue::Released)
        );
        assert_eq!(
            gate.observe(
                &event("Local Player", Some("Local Player"), 4),
                true,
                now + Duration::from_secs(2)
            ),
            Some(AlertCue::Targeted)
        );
    }

    #[test]
    fn identity_correction_does_not_sound_like_a_target_release() {
        let now = Instant::now();
        let mut gate = AlertGate::default();
        let originally_local = event("Old Local", Some("Old Local"), 1);
        gate.rebaseline(Some(&originally_local));

        let mut corrected = event("Old Local", Some("New Local"), 2);
        corrected.cause = BossTargetObservationCause::LocalIdentity;
        assert_eq!(gate.observe(&corrected, true, now), None);
    }

    #[test]
    fn encounter_change_and_unknown_identity_do_not_release() {
        let now = Instant::now();
        let mut gate = AlertGate::default();
        let local = event("Local Player", Some("Local Player"), 1);
        gate.rebaseline(Some(&local));

        let mut next_encounter = event("Other Player", Some("Local Player"), 2);
        next_encounter.encounter_started_at = Some(at("2026-07-23 20:01:00"));
        assert_eq!(gate.observe(&next_encounter, true, now), None);

        gate.rebaseline(Some(&local));
        assert_eq!(
            gate.observe(
                &event("Other Player", None, 3),
                true,
                now + Duration::from_secs(1)
            ),
            None
        );
    }

    #[test]
    fn disabled_release_is_consumed_without_a_delayed_cue() {
        let now = Instant::now();
        let mut gate = AlertGate::default();
        let local = event("Local Player", Some("Local Player"), 1);
        gate.rebaseline(Some(&local));
        let other = event("Other Player", Some("Local Player"), 2);

        assert_eq!(gate.observe(&other, false, now), None);
        assert_eq!(
            gate.observe(&other, true, now + Duration::from_secs(3)),
            None
        );
    }
}
