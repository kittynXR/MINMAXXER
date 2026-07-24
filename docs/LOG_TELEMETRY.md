# Ecliptica log telemetry

This capability audit is based on the VRChat logs from July 21–24, 2026 and the Ecliptica world ID `wrld_0fb88df3-2057-4c2f-8e06-e948864378fd`. The latest exhaustive pass used the 2.74 MB session log which continued through July 24 at 01:43 PDT: 34,347 physical lines, 18,801 timestamped lines, and 17,964 Ecliptica-scoped timestamped lines.

## Exhaustive July 24 event inventory

Counts are parser-accepted semantic records. `Jul 24` covers the post-midnight portion; `Session` covers the whole continued log so stage and encounter context is not cut at midnight.

| Data type | Jul 24 | Session | Sanitized log shape | Reliable payload |
|---|---:|---:|---|---|
| World entered | 0 | 3 | `Entering Room: Ecliptica…` | World boundary |
| World exited | 0 | 2 | `OnLeftRoom` | World boundary |
| World instance | 0 | 4 raw | `Joining wrld_<…>:<instance>` | Instance/visit identity |
| Local player identified | 0 | 3 | `Initialized PlayerAPI "<LOCAL>" is local` | Local perspective |
| Player joined | 1 | 33 | `OnPlayerJoined <PLAYER> (usr_<…>)` | Roster presence |
| Player left | 4 | 13 | `OnPlayerLeft <PLAYER> (usr_<…>)` | Roster presence |
| Session loaded | 0 | 3 | `ECLIPTICA loaded SESSION ID <N>` | Run/session key |
| Session invalidated | 0 | 2 | `session ID does not match…` | Clears a stale session key |
| Session saved | 61 | 209 | `ECLIPTICA saving SESSION ID <N>` | Checkpoint only; no item payload |
| Stage entered | 6 | 23 | `now in stage: <STAGE> on phase: <F> as class: <CLASS>` | Stage, fractional run progress, local class |
| Boss started/form changed | 8 | 29 | `now fighting boss: <BOSS>(Clone) on phase: <F>` | Boss/form start |
| Boss defeated | 9 | 29 | `Boss <BOSS> dead, personal damage dealt:` | Strong boss-end marker |
| Personal boss summary rows | 30 | 106 | `STRIKE DMG: <N>` / `NON-STRIKE DMG: <N>` | Local player's post-fight boss total |
| Intermission | 6 | 20 | `now in intermission` | Structural phase boundary |
| Lobby | 1 | 2 | `now in lobby` | Run boundary |
| Local outgoing hit | 391 | 1,391 | `Dealing <N> STRIKE damage` | Amount and coarse category; no target or ability |
| Local incoming hit | 245 | 852 | `damage has been taken: <N>, from source: <TEXT>` | Amount and raw source; source can be blank |
| Enemy spawned | 306 | 1,249 | `Initializing Enemy POOL ID<P> as ENEMY ID <E>` | Pool slot and opaque enemy ID |
| Enemy named | 304 | 1,237 | `No targets to encode on <ENTITY>(Clone)` | Entity name observed near a spawn |
| Enemy retired | 308 | 1,234 | `Retiring Enemy POOL ID<P>` | Pool cleanup/despawn, not a kill |
| Ownership transferred | 647 | 2,847 | `ownership of <ENTITY> transferred to <PLAYER>` | Network owner; target proxy only |
| Spawn-token diagnostic | 18 | 69 | `spawn token, <True/False>, <ID>` | Undocumented boolean/ID tuple |
| Enemy diagnostic | 32 | 106 | `ENEMY <ENTITY> INTRO SLAP.` | Internal diagnostic only |

The post-midnight combat portion contained 391 outgoing hits for 140,404 raw damage and 245 incoming hits for 3,632 damage. All outgoing lines were `STRIKE`; 59 incoming hits had an empty source. Raw logs repeat buffered starts, summaries, and cleanup messages, so accepted semantic counts are intentionally lower than raw text matches.

High-volume world/VRChat implementation noise is not gameplay telemetry: component-stripping reports, `Backup Active, swapping...`, localization-key misses, Udon stack traces, asset-bundle messages, and aggregate networking statistics reveal object names or runtime behavior but no combat state values.

## Directly available

| Data | Reliability | Notes |
|---|---|---|
| Local outgoing hit | Direct | Amount plus strike/non-strike category. |
| Local incoming hit | Direct | Amount and an opaque raw attack/source label; the source can be blank. |
| Named boss-death header and personal boss summary | Direct, noisy | A matching named death header is the strongest boss-end marker. Repeated summary revisions are canonicalized per boss phase, including zero-damage summaries. |
| Stage, class, boss, phase | Direct/derived | The numeric run-progress value is direct; its named difficulty band is a third-party presentation mapping. |
| Session ID | Direct, guarded | Blank and mismatch messages clear stale state; saves are checkpoints, not endings or shop purchases. |
| Player roster and user ID | Direct | Presence only; it does not imply combat contribution. |
| Enemy pool activity | Direct | A retire record is cleanup/despawn, not a kill. |
| Network ownership | Direct | Useful as an experimental focus hint only, never damage or aggro attribution. |
| Spawn/retire pool diagnostics | Direct, opaque | Pool IDs, enemy IDs, and nearby entity names; no health or authoritative kill meaning. |
| Spawn-token diagnostics | Direct, undocumented | Boolean plus numeric ID; no demonstrated relationship to player gems, inventory, or rewards. |
| Stage progress and timing | Direct/derived | Durations use line timestamps and file order. |

## Derived by MINMAXXER

- Boss-window/run DPS, rolling 5/15/30-second DPS, totals, hit count, average, maximum, and first-to-last combat span. When boss telemetry exists, primary historical run metrics include boss windows only; pre-boss duration, outgoing damage, and incoming damage are reported separately. Runs without boss windows fall back to their observed combat span.
- Strike/non-strike shares, damage taken per second, and incoming totals by source.
- Phase-scoped local hit feed, boss/stage segmentation, run timeline, parser coverage, and party totals after per-player logs are merged. The newest hits persist until the phase boundary and dim after inactivity.
- Full-phase average DPS (`phase damage / phase elapsed time`) displayed alongside the existing rolling five-second DPS curve.
- Run-progress labels follow the unofficial [EACT five-band reference](https://eact-doc.rtail.dev/?lang=en#phases): `0.0–<0.2` PRIME, `0.2–<0.4` PENUMBRA, `0.4–<0.6` ANTUMBRA, `0.6–<0.8` UMBRA, and `0.8–1.0` ECLIPSE. The logged number is direct, but the names and MINMAXXER's half-open endpoint handling are derived presentation rather than an official world contract. Progress `1.0` is additionally presented as **EYE OF THE ECLIPSE** only when the final Bringer stage/boss marker corroborates the community [boss reference](https://wikiwiki.jp/ecliptica/%E3%83%9C%E3%82%B9).
- The 1-based boss ordinal is exact when stage markers have been counted from a run observed at progress zero, or when the final Bringer marker identifies boss 13. For other mid-run attachments, the nearest audited progress anchor supplies a visibly marked inferred ordinal (`~`/`INFERRED`); repeated stage announcements and boss subphases do not increment it.
- Boss boundaries prefer a matching named death or personal-summary record. When that marker is absent or arrives out of order, the next boss start, intermission, lobby, world exit, or stage transition provides a lower-confidence structural boundary. Boundary provenance and confidence are retained for analysis.
- Experimental `BOSS TARGET?` from network ownership transferred to a player for an entity whose normalized name exactly matches the current boss. Recent matching incoming-hit activity can corroborate the ownership proxy. The display reports confidence, evidence, and age, and retains its last candidate until the encounter boundary.

## Not present in the audited logs

These fields are omitted from default OBS, desktop, and native VR HUDs so they consume no live space. Overlay Studio can opt into a compact browser-only reminder row that labels them `NOT LOGGED`.

- Remote-player damage attribution from a single client's log.
- Outgoing ability/action names. `STRIKE` and `NON-STRIKE` are the only logged local damage categories, not ability names.
- Healing or healer attribution.
- Buff/debuff applied, removed, stack, duration, or ownership state.
- Player or enemy current/max health, health percentage, health-bar text, shields, absorbs, mitigation, or avoidable-damage flags. The named boss-death line is the first exact boss-health state: zero.
- Critical hits, misses, blocks, revives, player deaths, or downed state.
- Regular-enemy kills; pool retirement cannot be used as a kill event.
- Authoritative hate/aggro target, clear/wipe outcome, loot, score, gem balance/spending, or rewards.
- Shop item identities, selected upgrades, equipment, or stack counts. Repeated session save/load markers contain no item payload and must not be interpreted as purchases or a loadout.
- Current song, music title, or an Ecliptica-scoped media identifier.

Names such as `BuffBug` and component references such as `SyrupDebuff` are object/asset text, not gameplay effect telemetry.

### Why boss HP cannot be reconstructed exactly

The complete July 24 audit found zero whole-word `HP`, `health`, `heal`, `currentHealth`, `maxHealth`, health-bar, or UI-text payloads. Udon output exposes setup/errors and aggregate byte counts, not synchronized variable names or values.

The local `Dealing` stream also cannot be treated as boss damage: it has no target field. Across 29 canonical boss encounters, summing all outgoing lines between boss start and death matched the later personal boss summary in 21 encounters but exceeded that boss-only summary in eight, by 19,676 damage total. In one phase the live targetless stream was 14,394 while the boss-only personal summary was 12,453, demonstrating add/other-target contamination.

Even a complete set of party logs would still lack boss maximum HP and per-hit target attribution. Exact live HP seen by players therefore comes from Ecliptica's in-world UI or another telemetry path, not vanilla VRChat `output_log`.

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
- Count a boss ordinal from distinct stage-entry markers, not boss-start lines, because multi-form bosses emit additional starts without advancing the run. Preserve an explicit inferred flag when recovery begins mid-run.
- Ignore stale post-lobby boss announcements until a new stage begins.
- Treat imported players as separate local perspectives and merge only by a validated session/visit key.
