# Ecliptica log telemetry

This capability audit is based on the VRChat logs from July 21, 2026 and the Ecliptica world ID `wrld_0fb88df3-2057-4c2f-8e06-e948864378fd`.

## Directly available

| Data | Reliability | Notes |
|---|---|---|
| Local outgoing hit | Direct | Amount plus strike/non-strike category. |
| Local incoming hit | Direct | Amount and an opaque raw attack/source label; the source can be blank. |
| Named boss-death header and personal boss summary | Direct, noisy | A matching named death header is the strongest boss-end marker. Repeated summary revisions are canonicalized per boss phase, including zero-damage summaries. |
| Stage, class, boss, phase | Direct | Phase is treated as the world's opaque progress value. |
| Session ID | Direct, guarded | Blank and mismatch messages clear stale state; saves are checkpoints, not endings. |
| Player roster and user ID | Direct | Presence only; it does not imply combat contribution. |
| Enemy pool activity | Direct | A retire record is cleanup/despawn, not a kill. |
| Network ownership | Direct | Useful as an experimental focus hint only, never damage or aggro attribution. |
| Stage progress and timing | Direct/derived | Durations use line timestamps and file order. |

## Derived by MINMAXXER

- Boss-window/run DPS, rolling 5/15/30-second DPS, totals, hit count, average, maximum, and first-to-last combat span. When boss telemetry exists, primary historical run metrics include boss windows only; pre-boss duration, outgoing damage, and incoming damage are reported separately. Runs without boss windows fall back to their observed combat span.
- Strike/non-strike shares, damage taken per second, and incoming totals by source.
- Recent local hit feed, boss/stage segmentation, run timeline, parser coverage, and party totals after per-player logs are merged.
- Boss boundaries prefer a matching named death or personal-summary record. When that marker is absent or arrives out of order, the next boss start, intermission, lobby, world exit, or stage transition provides a lower-confidence structural boundary. Boundary provenance and confidence are retained for analysis.
- Experimental `BOSS TARGET?` from network ownership transferred to a player for an entity whose normalized name exactly matches the current boss. Recent matching incoming-hit activity can corroborate the ownership proxy. The display reports confidence, evidence, and age, and retains its last candidate until the encounter boundary.

## Not present in the audited logs

- Remote-player damage attribution from a single client's log.
- Healing or healer attribution.
- Buff/debuff applied, removed, stack, duration, or ownership state.
- Player or enemy health, shields, absorbs, mitigation, or avoidable-damage flags.
- Critical hits, misses, blocks, revives, player deaths, or downed state.
- Regular-enemy kills; pool retirement cannot be used as a kill event.
- Authoritative hate/aggro target, difficulty, clear/wipe outcome, loot, score, rewards, equipment, or upgrades.

Names such as `BuffBug` and component references such as `SyrupDebuff` are object/asset text, not gameplay effect telemetry.

## Parser safety rules

- Parse the timestamped VRChat envelope first, then anchored Ecliptica message patterns.
- Preserve file byte offset and sequence because timestamps have only one-second resolution.
- Never merge identical hit records by value.
- Suppress duplicate boss-start announcements occurring within the same few seconds.
- Match named boss-death and summary records to the corresponding normalized boss name, so a late phase marker cannot close the following phase.
- Canonicalize repeated boss summaries while keeping their underlying raw records available.
- Use structural boss boundaries only after checking for a matching named marker, and retain the boundary source and confidence instead of presenting an inferred end as authoritative.
- Match boss network-ownership entities by exact normalized name; substring matches can confuse bosses with unrelated pooled objects.
- Clear run/class/phase/boss state at lobby and world boundaries.
- Ignore stale post-lobby boss announcements until a new stage begins.
- Treat imported players as separate local perspectives and merge only by a validated session/visit key.
