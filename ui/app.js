(() => {
  "use strict";

  const DEFAULT_ORIGIN = location.protocol === "file:" || location.protocol === "about:" ? "http://127.0.0.1:49321" : location.origin;
  const $ = (selector, root = document) => root.querySelector(selector);
  const $$ = (selector, root = document) => Array.from(root.querySelectorAll(selector));
  const clamp = (value, min, max) => Math.min(max, Math.max(min, value));
  const number = (value, fallback = 0) => Number.isFinite(Number(value)) ? Number(value) : fallback;
  const escapeHtml = (value) => String(value ?? "").replace(/[&<>'"]/g, (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;" })[char]);
  const safeId = (value) => encodeURIComponent(String(value ?? ""));
  const nowSeconds = () => Date.now() / 1000;

  const COLORS = ["#54e6ff", "#ad8cff", "#65f2b8", "#ffc45c", "#ff719a", "#70a7ff", "#eaa1ff", "#8de46b"];
  const ROLE_COLORS = { striker: "#54e6ff", support: "#65f2b8", tank: "#ad8cff", flex: "#ffc45c", unknown: "#8491a3" };
  const EVENT_COLORS = { damage: "#54e6ff", heal: "#65f2b8", buff: "#ad8cff", debuff: "#ff719a", death: "#ff5c70", system: "#ffc45c" };
  const ACCENTS = {
    cyan: ["#54e6ff", "84,230,255"], mint: ["#65f2b8", "101,242,184"], violet: ["#ad8cff", "173,140,255"],
    amber: ["#ffc45c", "255,196,92"], rose: ["#ff719a", "255,113,154"]
  };
  const titles = { live: "LIVE ENCOUNTER", runs: "RUN HISTORY", compare: "COMPARE RUNS", analysis: "DEEP ANALYSIS", events: "EVENT EXPLORER", overlay: "OVERLAY STUDIO" };
  const OVERLAY_SETTINGS_VERSION = 2;

  const state = {
    live: null,
    runs: [],
    events: [],
    settings: {},
    apiOnline: false,
    streamOnline: false,
    usingMock: true,
    frozen: false,
    selectedRunId: null,
    runFilter: "all",
    partyMetric: "dps",
    analysisTab: "players",
    analysisEncounterByRun: {},
    analysisPlayerByRun: {},
    eventLimit: 60,
    eventRate: 0,
    eventCounter: 0,
    overlay: null,
    vrStatus: null,
    timers: [],
    charts: {},
    profileSaveTimer: null,
    profileSaveInFlight: false,
    profileSavePendingOptions: null,
    studioBackendPending: false,
    lastLiveAt: 0,
    archiveRefreshTimer: null,
    archiveBootstrapRetryTimer: null,
    archiveStreamPrimed: false,
    archiveMeaningfulPrimed: false,
    archiveOpenPrimed: false,
    overlayServiceState: "ready",
    overlayStreamLossTimer: null
  };
  const overlayHitMemory = new WeakMap();

  function seededWave(index, base, amplitude, phase = 0) {
    const wave = Math.sin(index * .31 + phase) * .56 + Math.sin(index * .127 + phase * 1.7) * .24 + Math.sin(index * .73) * .12;
    const burst = index % 17 > 12 ? .25 * Math.sin(((index % 17) - 12) / 5 * Math.PI) : 0;
    return Math.max(0, base * (1 + amplitude * wave + burst));
  }

  function makeMockLive() {
    const players = [
      { name: "Aster", role: "Striker", className: "Void Dancer", dps: 48240, hps: 820, incoming: 1480, damage: 19357000, healing: 328000, crit: 38.4, active: 98.7, deaths: 0, color: COLORS[0], you: true },
      { name: "Riven", role: "Striker", className: "Starbreaker", dps: 42980, hps: 360, incoming: 1720, damage: 17252000, healing: 144000, crit: 34.1, active: 97.9, deaths: 0, color: COLORS[1] },
      { name: "Hexadecimal", role: "Support", className: "Lumen Sage", dps: 35810, hps: 15940, incoming: 1190, damage: 14360000, healing: 6391000, crit: 26.2, active: 96.4, deaths: 0, color: COLORS[2] },
      { name: "Mirai", role: "Tank", className: "Gravity Warden", dps: 31220, hps: 2310, incoming: 3520, damage: 12520000, healing: 926000, crit: 23.8, active: 99.2, deaths: 0, color: COLORS[3] },
      { name: "Comet", role: "Flex", className: "Solar Weaver", dps: 25970, hps: 2360, incoming: 550, damage: 10414000, healing: 947000, crit: 29.6, active: 91.5, deaths: 1, color: COLORS[4] }
    ];
    const attacks = mockAttacks();
    players.forEach((player, index) => {
      player.strike = player.damage * (.61 + index * .025);
      player.nonStrike = player.damage - player.strike;
      player.attacks = attacks.filter((attack) => attack.source === player.name);
    });
    const duration = 402.8;
    const timeline = Array.from({ length: 72 }, (_, i) => ({
      t: i * duration / 71,
      total: seededWave(i, 184200, .62, .2),
      hps: seededWave(i, 21800, .78, 2.1),
      incoming: seededWave(i, 8460, 1.05, 4.3),
      players: players.map((player, p) => seededWave(i, player.dps, .72, p * 1.3 + .3))
    }));
    return {
      version: 1,
      connected: true,
      inWorld: true,
      status: "watching",
      observedPlayer: "Aster",
      sessionId: "ECL-7B42-0482",
      world: "Ecliptica",
      stage: "Umbra Citadel · Depth 4",
      className: "Void Dancer",
      sourceFile: "output_log_demo.txt",
      runContext: { progress: null, phaseName: "", bossNumber: null, bossNumberInferred: false, bossSubphase: null },
      loadout: { available: false, items: [], sourceNote: "Ecliptica does not expose shop item names or stack counts in the audited VRChat log." },
      encounter: { name: "Astral Sovereign", kind: "Boss", phase: "Phase 3 · Eclipse", duration, active: true, hpPercent: 27.4, hpCurrent: 8310000, hpMax: 30280000, shieldPercent: 9 },
      focus: { player: "Mirai", entity: "Astral Sovereign", ageSeconds: 1.2, confidence: "likely", evidence: "boss_owner_plus_local_incoming", corroboratingHits: 2, corroboratedAt: new Date().toISOString(), sourceNote: "Exact boss ownership plus immediate local incoming damage; still not an authoritative hate table." },
      outgoing: { total: 73903000, strike: 52600000, nonStrike: 21303000, hits: 1831, biggestHit: 388420, dps: 184220, rolling5s: 216300, rolling15s: 196100, rolling30s: 188700 },
      incoming: { total: 3400000, hits: 94, biggestHit: 244100, dps: 8460, avoidablePercent: 14.2, bySource: [] },
      partyHps: 21790,
      critRate: 31.6,
      debuffUptime: 94.7,
      players,
      roster: players.map((p) => p.name),
      attacks,
      timeline,
      effects: [
        { name: "Fractured Reality", source: "Astral Sovereign", kind: "debuff", icon: "FR", remaining: 3.8, color: "#ff719a" },
        { name: "Lumen Chorus", source: "Hexadecimal", kind: "buff", icon: "LC", remaining: 11.4, color: "#65f2b8" },
        { name: "Eclipse Brand", source: "Aster", kind: "debuff", icon: "EB", stacks: 5, color: "#ad8cff" },
        { name: "Solar Alignment", source: "Comet", kind: "buff", icon: "SA", remaining: 6.2, color: "#ffc45c" },
        { name: "Gravity Anchor", source: "Mirai", kind: "buff", icon: "GA", remaining: 8.6, color: "#54e6ff" }
      ],
      recentEvents: mockEvents().slice(0, 14),
      recentHits: [
        { id: "hit-1", time: "06:42.8", age: .2, type: "damage", source: "Aster", action: "Umbra Cascade", target: "Astral Sovereign", amount: 388420, flags: ["CRIT", "STRIKE"], direction: "dealt" },
        { id: "hit-2", time: "06:42.4", age: .6, type: "damage", source: "Aster", action: "Astral Echo", target: "Astral Sovereign", amount: 62110, flags: ["NON-STRIKE"], direction: "dealt" },
        { id: "hit-3", time: "06:41.9", age: 1.1, type: "damage", source: "Aster", action: "Singularity", target: "Astral Sovereign", amount: 377610, flags: ["CRIT", "STRIKE"], direction: "dealt" },
        { id: "hit-4", time: "06:41.5", age: 1.5, type: "damage", source: "Aster", action: "Void Needle", target: "Astral Sovereign", amount: 48190, flags: ["NON-STRIKE"], direction: "dealt" },
        { id: "hit-5", time: "06:40.9", age: 2.1, type: "damage", source: "Aster", action: "Umbra Cascade", target: "Astral Sovereign", amount: 241880, flags: ["STRIKE"], direction: "dealt" }
      ],
      parserCoverage: { damage: "direct", healing: "direct", buffs: "partial", absorbs: "unavailable", positions: "unavailable" },
      capabilityNote: "Only events written by Ecliptica to the VRChat log can be measured."
    };
  }

  function mockAttacks() {
    return [
      { name: "Umbra Cascade", source: "Aster", damage: 7340000, hits: 84, crit: 47.6, max: 388420, dps: 18304 },
      { name: "Nova Sever", source: "Riven", damage: 6810000, hits: 52, crit: 42.3, max: 351800, dps: 16987 },
      { name: "Singularity", source: "Aster", damage: 5290000, hits: 19, crit: 36.8, max: 377610, dps: 13198 },
      { name: "Lumen Spear", source: "Hexadecimal", damage: 4920000, hits: 91, crit: 28.6, max: 142200, dps: 12276 },
      { name: "Event Horizon", source: "Mirai", damage: 4060000, hits: 112, crit: 21.4, max: 96740, dps: 10130 },
      { name: "Solar Flare", source: "Comet", damage: 3840000, hits: 47, crit: 31.9, max: 186300, dps: 9579 },
      { name: "Void Needle", source: "Riven", damage: 3380000, hits: 149, crit: 33.6, max: 88210, dps: 8433 },
      { name: "Astral Echo", source: "Aster", damage: 2950000, hits: 233, crit: 38.2, max: 62100, dps: 7359 }
    ];
  }

  function mockEvents() {
    const rows = [
      ["06:42.8", "damage", "Aster", "Umbra Cascade", "Astral Sovereign", 388420, ["CRIT", "STRIKE"]],
      ["06:42.5", "debuff", "Astral Sovereign", "Fractured Reality", "Comet", 0, ["3.8s"]],
      ["06:42.1", "heal", "Hexadecimal", "Lumen Chorus", "Mirai", 74260, ["CRIT"]],
      ["06:41.7", "damage", "Riven", "Nova Sever", "Astral Sovereign", 351810, ["CRIT"]],
      ["06:41.2", "buff", "Comet", "Solar Alignment", "Party", 0, ["6.2s"]],
      ["06:40.9", "damage", "Astral Sovereign", "Eclipse Pulse", "Mirai", 244100, ["INCOMING"]],
      ["06:40.4", "damage", "Hexadecimal", "Lumen Spear", "Astral Sovereign", 142220, []],
      ["06:39.9", "debuff", "Aster", "Eclipse Brand", "Astral Sovereign", 0, ["×5"]],
      ["06:39.3", "heal", "Comet", "Photosphere", "Aster", 51940, []],
      ["06:38.8", "damage", "Mirai", "Event Horizon", "Astral Sovereign", 96740, ["STRIKE"]],
      ["06:38.1", "system", "Encounter", "Phase transition", "Eclipse", 0, ["P3"]],
      ["06:37.8", "damage", "Comet", "Solar Flare", "Astral Sovereign", 186300, ["CRIT"]],
      ["06:36.9", "buff", "Mirai", "Gravity Anchor", "Mirai", 0, ["8.6s"]],
      ["06:35.4", "damage", "Astral Sovereign", "Dark Matter", "Comet", 118440, ["AVOIDABLE"]],
      ["06:34.1", "death", "Comet", "Defeated", "Comet", 0, ["KO"]]
    ];
    return Array.from({ length: 9 }, (_, block) => rows.map((row, i) => {
      const seconds = 402.8 - (block * rows.length + i) * .47;
      return { id: `ev-${block}-${i}`, time: formatDuration(Math.max(0, seconds), true), type: row[1], source: row[2], action: row[3], target: row[4], amount: Math.max(0, row[5] * (1 - block * .018)), flags: row[6], raw: `[Ecliptica] ${row[2]} ${row[3]} ${row[4]} ${row[5]}` };
    })).flat();
  }

  function makeMockRuns(live) {
    const specs = [
      ["482", "Astral Sovereign", "closed", 402.8, 184220, 73903000, 31.6, "Today · 21:47"],
      ["481", "Astral Sovereign", "ongoing", 287.4, 171380, 49257000, 29.8, "Today · 21:34"],
      ["480", "The Parallax Engine", "closed", 361.2, 192740, 69610000, 33.1, "Today · 21:19"],
      ["479", "Astral Sovereign", "closed", 417.6, 177540, 74133000, 30.4, "Yesterday · 23:08"],
      ["478", "Hollow Regent", "closed", 298.5, 165930, 49530000, 27.9, "Yesterday · 22:49"],
      ["477", "The Parallax Engine", "ongoing", 198.2, 153210, 30369000, 28.7, "Yesterday · 22:31"],
      ["476", "Hollow Regent", "closed", 312.7, 160480, 50177000, 29.2, "Yesterday · 22:12"],
      ["475", "Astral Sovereign", "closed", 431.9, 169840, 73353000, 29.7, "Mon · 20:18"]
    ];
    return specs.map((r, idx) => {
      const factor = r[4] / live.outgoing.dps;
      const players = live.players.map((p, pidx) => ({ ...p, dps: Math.round(p.dps * factor * (1 + (pidx - 2) * .013 * (idx % 3 - 1))), damage: Math.round(p.damage * factor) }));
      const preBossDuration = 48 + (idx % 4) * 7;
      const preBossDamage = Math.round(r[5] * (.075 + (idx % 3) * .012));
      const bossEncounter = {
        id: `${r[0]}-boss-1`, name: r[1], stage: idx > 3 ? "Obsidian Meridian" : "Umbra Citadel · Depth 4", kind: "boss",
        duration: r[3], duration_seconds: r[3], dps: r[4], totalDamage: r[5], hits: Math.round(1831 * factor), biggestHit: Math.round(388420 * factor),
        outgoing: { total: r[5], dps: r[4], hits: Math.round(1831 * factor), biggest_hit: Math.round(388420 * factor) },
        incoming: { total: Math.round(live.incoming.total * factor), damage_per_second: Math.round(live.incoming.dps * (.88 + (idx % 3) * .11)), by_source: [] },
        incomingDps: Math.round(live.incoming.dps * (.88 + (idx % 3) * .11)), players, attacks: live.attacks, timeline: live.timeline,
        endReason: r[2] === "closed" ? "boss_defeated" : "open", boundaryConfidence: r[2] === "closed" ? "explicit" : "open", completed: r[2] === "closed"
      };
      const preBossEncounter = {
        id: `${r[0]}-pre-1`, name: "Approach combat", stage: bossEncounter.stage, kind: "pre_boss", duration: preBossDuration, duration_seconds: preBossDuration,
        dps: preBossDamage / preBossDuration, totalDamage: preBossDamage, outgoing: { total: preBossDamage, dps: preBossDamage / preBossDuration },
        incoming: { total: Math.round(preBossDamage * .08), damage_per_second: preBossDamage * .08 / preBossDuration }, endReason: "boss_started", boundaryConfidence: "structural", completed: true,
        players: [], attacks: [], timeline: []
      };
      return {
        id: r[0], number: r[0], encounter: r[1], stage: idx > 3 ? "Obsidian Meridian" : "Umbra Citadel · Depth 4", result: r[2], duration: r[3], dps: r[4], totalDamage: r[5], critRate: r[6], when: r[7], hps: Math.round(live.partyHps * (.9 + (idx % 4) * .035)), incomingDps: Math.round(live.incoming.dps * (.88 + (idx % 3) * .11)), debuffUptime: 88.4 + (idx % 4) * 2.1, deaths: idx % 3 === 1 ? 1 : 0, players, attacks: live.attacks, timeline: live.timeline.map((point, i) => ({ ...point, total: point.total * factor * (1 + Math.sin(i * .2 + idx) * .06) })),
        metricsScope: "boss", observedDuration: r[3] + preBossDuration, preBossDuration, preBossOutgoing: preBossEncounter.outgoing, preBossIncoming: preBossEncounter.incoming, bossCount: 1, encounters: [preBossEncounter, bossEncounter],
        outgoing: bossEncounter.outgoing, incoming: bossEncounter.incoming, hits: bossEncounter.hits, biggestHit: bossEncounter.biggestHit
      };
    });
  }

  function formatCompact(value, digits = 1) {
    const n = number(value);
    const abs = Math.abs(n);
    if (abs >= 1e9) return `${(n / 1e9).toFixed(digits)}b`;
    if (abs >= 1e6) return `${(n / 1e6).toFixed(digits)}m`;
    if (abs >= 1e3) return `${(n / 1e3).toFixed(digits)}k`;
    return Math.round(n).toLocaleString();
  }

  function formatNumber(value) { return Math.round(number(value)).toLocaleString(); }
  function formatPercent(value, digits = 1) { return `${number(value).toFixed(digits)}%`; }
  function formatDuration(seconds, tenths = false) {
    const total = Math.max(0, number(seconds));
    const min = Math.floor(total / 60);
    const sec = total - min * 60;
    return `${String(min).padStart(2, "0")}:${sec.toFixed(tenths ? 1 : 0).padStart(tenths ? 4 : 2, "0")}`;
  }
  function initials(name) { return String(name || "?").split(/\s+/).map((part) => part[0]).join("").slice(0, 2).toUpperCase(); }

  function normalizeAttack(attack, duration) {
    const damage = number(attack.damage ?? attack.total);
    const damageType = String(attack.damage_type ?? attack.damageType ?? "");
    const category = String(attack.name ?? damageType.replaceAll("_", " ") ?? "").trim() || "Unclassified damage";
    const sourceValue = attack.source ?? attack.actor;
    const source = sourceValue === undefined || sourceValue === null ? "" : String(sourceValue).trim();
    return {
      ...attack,
      name: category,
      damageType,
      source,
      sourceAvailable: source.length > 0,
      damage,
      hits: number(attack.hits),
      min: number(attack.min),
      max: number(attack.max ?? attack.biggest_hit),
      average: number(attack.average, number(attack.hits) ? damage / number(attack.hits) : 0),
      share: number(attack.share),
      crit: number(attack.crit ?? attack.crit_rate),
      critAvailable: attack.crit !== undefined || attack.crit_rate !== undefined,
      dps: number(attack.dps, duration ? damage / duration : 0)
    };
  }

  function normalizeTimeline(rawTimeline) {
    if (!Array.isArray(rawTimeline)) return [];
    let firstTimestamp = null;
    return rawTimeline.map((point, index) => {
      const timestamp = point.timestamp ?? point.occurred_at ?? null;
      const parsedTimestamp = timestamp ? Date.parse(timestamp) : NaN;
      if (firstTimestamp === null && Number.isFinite(parsedTimestamp)) firstTimestamp = parsedTimestamp;
      const explicitTime = point.elapsed_seconds ?? point.elapsedSeconds ?? point.t ?? point.time ?? point.second ?? point.elapsed;
      const elapsed = explicitTime === undefined || explicitTime === null
        ? Number.isFinite(parsedTimestamp) && firstTimestamp !== null ? Math.max(0, (parsedTimestamp - firstTimestamp) / 1000) : index
        : number(explicitTime, index);
      return {
        ...point,
        t: elapsed,
        timestamp,
        total: number(point.rolling_dps ?? point.total ?? point.total_dps ?? point.party_dps ?? point.dps),
        outgoing: number(point.outgoing),
        incoming: number(point.incoming ?? point.incoming_dps),
        hps: number(point.hps ?? point.healing_per_second),
        players: Array.isArray(point.players) ? point.players.map(number) : []
      };
    });
  }

  function normalizePlayer(player, index, duration, observed) {
    const outgoing = player.outgoing || (player.damage && typeof player.damage === "object" ? player.damage : {});
    const incoming = player.incoming || {};
    const healingStats = player.healing && typeof player.healing === "object" ? player.healing : {};
    const damage = number((typeof player.damage === "number" ? player.damage : undefined) ?? player.total_damage ?? player.total ?? outgoing.total);
    const dps = number(player.dps ?? player.damage_per_second ?? outgoing.dps, duration ? damage / duration : 0);
    const healing = number((typeof player.healing === "number" ? player.healing : undefined) ?? player.total_healing ?? player.heal_total ?? healingStats.total);
    const hps = number(player.hps ?? player.healing_per_second ?? healingStats.dps ?? healingStats.hps, duration ? healing / duration : 0);
    const hits = number(player.hits ?? outgoing.hits);
    const crits = number(player.crits ?? player.critical_hits);
    const name = player.name ?? player.player ?? player.display_name ?? player.actor ?? `Player ${index + 1}`;
    const role = player.role ?? player.combat_role ?? "Unknown";
    const attacksRaw = Array.isArray(player.attacks) ? player.attacks : [];
    return {
      ...player,
      name: String(name), role: String(role), className: String(player.className ?? player.class_name ?? player.class ?? role),
      damage, dps, healing, hps, hits, biggestHit: number(player.biggest_hit ?? outgoing.biggest_hit), strike: number(player.strike ?? outgoing.strike), nonStrike: number(player.non_strike ?? player.nonStrike ?? outgoing.non_strike ?? outgoing.nonStrike),
      incoming: number(player.damage_in ?? player.incoming_damage ?? incoming.total ?? player.incoming),
      incomingDps: number(player.incoming_dps ?? incoming.damage_per_second),
      crit: number(player.crit ?? player.crit_rate ?? player.critical_rate, hits ? crits / hits * 100 : 0),
      critAvailable: player.crit !== undefined || player.crit_rate !== undefined || player.critical_rate !== undefined || crits > 0,
      active: number(player.active ?? player.active_time_percent ?? player.uptime, duration && player.active_seconds !== undefined ? number(player.active_seconds) / duration * 100 : 100), deaths: number(player.deaths ?? player.knockouts),
      attacks: attacksRaw.map((attack) => normalizeAttack(attack, duration)),
      color: COLORS[index % COLORS.length], you: Boolean(player.you ?? player.is_self ?? String(name) === String(observed || ""))
    };
  }

  function normalizeEvent(event, index = 0) {
    const typeRaw = String(event.type ?? event.kind ?? event.event_type ?? "system").toLowerCase();
    const type = Object.keys(EVENT_COLORS).find((key) => typeRaw.includes(key)) || (typeRaw.includes("hit") ? "damage" : "system");
    const timeValue = event.time ?? event.elapsed ?? event.timestamp_offset;
    const timestamp = event.timestamp ?? event.occurred_at ?? null;
    const timestampClock = String(timestamp ?? "").match(/(?:T|\s)(\d{2}:\d{2}:\d{2})(?:\.\d+)?/)?.[1] ?? "00:00:00";
    return {
      ...event,
      id: event.id ?? `${Date.now()}-${index}`,
      time: timeValue === undefined || timeValue === null ? timestampClock : typeof timeValue === "string" && timeValue.includes(":") ? timeValue : formatDuration(number(timeValue), true),
      type,
      source: String(event.source ?? event.player ?? event.actor ?? event.source_name ?? "Unknown"),
      action: String(event.action ?? event.attack ?? event.effect ?? event.ability ?? event.name ?? event.damage_type ?? event.message ?? typeRaw),
      target: String(event.target ?? event.target_name ?? event.boss ?? event.entity ?? "—"),
      amount: number(event.amount ?? event.value ?? event.damage ?? event.healing),
      flags: Array.isArray(event.flags) ? event.flags.map(String) : [event.critical || event.crit ? "CRIT" : "", event.strike ? "STRIKE" : "", event.damage_type ? String(event.damage_type).toUpperCase().replace("_", "-") : ""].filter(Boolean),
      raw: String(event.raw ?? event.raw_line ?? event.line ?? ""), rawType: typeRaw,
      direction: String(event.direction ?? (typeRaw.includes("dealt") || typeRaw.includes("outgoing") ? "dealt" : typeRaw.includes("taken") || typeRaw.includes("incoming") ? "taken" : "")),
      age: number(event.age ?? event.age_seconds ?? event.seconds_ago, NaN), timestamp
    };
  }

  function phaseNameFromProgress(value) {
    if (value === null || value === undefined || value === "") return "";
    const progress = Number(value);
    if (!Number.isFinite(progress) || progress < 0 || progress > 1.000001) return "";
    if (progress < .2) return "PRIME";
    if (progress < .4) return "PENUMBRA";
    if (progress < .6) return "ANTUMBRA";
    if (progress < .8) return "UMBRA";
    return "ECLIPSE";
  }

  function normalizeLive(payload, fallback = makeMockLive()) {
    if (!payload || typeof payload !== "object") return fallback;
    const src = payload.live ?? payload.snapshot ?? payload.data ?? payload;
    const isRealSnapshot = src.version !== undefined;
    const encounterRaw = src.encounter ?? src.current_encounter ?? {};
    const duration = number(encounterRaw.duration_seconds ?? encounterRaw.duration ?? src.duration_seconds ?? src.elapsed, isRealSnapshot ? 0 : fallback.encounter.duration);
    const outgoingRaw = src.outgoing ?? src.damage ?? {};
    const incomingRaw = src.incoming ?? {};
    const observed = src.observed_player ?? src.observedPlayer ?? src.player_name ?? (src.version !== undefined ? "" : fallback.observedPlayer);
    let playersRaw = src.players ?? src.combatants ?? src.party ?? [];
    if (!Array.isArray(playersRaw)) playersRaw = Object.entries(playersRaw).map(([name, data]) => ({ name, ...(data || {}) }));
    const explicitPlayers = isRealSnapshot || src.players !== undefined || src.combatants !== undefined || src.party !== undefined;
    const players = playersRaw.length ? playersRaw.map((p, i) => normalizePlayer(p, i, duration, observed)) : explicitPlayers ? ((observed || number(outgoingRaw.total) > 0) ? [normalizePlayer({ player: observed || "Local player", damage: outgoingRaw, incoming: incomingRaw, class_name: src.class_name }, 0, duration, observed)] : []) : fallback.players;
    const totalDps = number(outgoingRaw.dps ?? src.party_dps ?? src.dps, players.reduce((sum, p) => sum + p.dps, 0));
    const partyHps = number(src.party_hps ?? src.hps, players.reduce((sum, p) => sum + p.hps, 0));
    const incomingDps = number(incomingRaw.damage_per_second ?? incomingRaw.dps ?? src.incoming_dps, players.reduce((sum, p) => sum + (p.incomingDps || (duration ? p.incoming / duration : 0)), 0));
    const rawTimeline = Array.isArray(src.timeline) ? src.timeline : [];
    let timeline = normalizeTimeline(rawTimeline);
    if ((timeline.length < 2 || !timeline.some((point) => point.total > 0)) && src.version === undefined) {
      timeline = Array.from({ length: 60 }, (_, i) => ({ t: i * duration / 59, total: seededWave(i, totalDps, .55, .4), hps: seededWave(i, partyHps, .7, 2), incoming: seededWave(i, incomingDps, .9, 4), players: players.map((p, j) => seededWave(i, p.dps, .62, j + .5)) }));
    }
    const attacksRaw = Array.isArray(src.attacks) ? src.attacks : isRealSnapshot ? [] : fallback.attacks;
    const effectsRaw = src.effects ?? src.active_effects ?? src.buffs ?? (isRealSnapshot ? [] : fallback.effects);
    const eventsRaw = src.recent_events ?? src.recentEvents ?? (isRealSnapshot ? [] : fallback.recentEvents);
    const hitsRaw = src.recent_hits ?? src.recentHits ?? [];
    const runContextRaw = src.run_context ?? src.runContext ?? {};
    const progressCandidate = runContextRaw.progress ?? encounterRaw.phase ?? src.phase;
    const parsedProgress = Number(progressCandidate);
    const runProgress = progressCandidate !== null && progressCandidate !== undefined && progressCandidate !== "" && Number.isFinite(parsedProgress) && parsedProgress >= 0 && parsedProgress <= 1.000001 ? clamp(parsedProgress, 0, 1) : null;
    const loadoutRaw = src.loadout ?? {};
    const loadoutItems = Array.isArray(loadoutRaw.items) ? loadoutRaw.items.map((item) => ({ name: String(item.name ?? item.display_name ?? item.id ?? "Unknown item"), stacks: Math.max(1, Math.round(number(item.stacks, 1))) })) : [];
    return {
      ...fallback, ...src,
      version: src.version ?? fallback.version,
      connected: Boolean(src.connected ?? true), inWorld: Boolean(src.in_world ?? src.inWorld ?? (src.version === undefined ? fallback.inWorld : false)), status: String(src.status ?? "watching"), observedPlayer: String(observed ?? ""),
      sessionId: String(src.session_id ?? src.sessionId ?? (src.version !== undefined ? "—" : fallback.sessionId)), world: String(src.world ?? (src.version !== undefined ? "Waiting for Ecliptica" : fallback.world)), stage: String(src.stage ?? (src.version !== undefined ? "No stage detected" : fallback.stage)),
      className: String(src.class_name ?? src.className ?? (src.version !== undefined ? "Unknown class" : fallback.className)), sourceFile: String(src.source_file ?? src.sourceFile ?? (src.version !== undefined ? "No active log file" : fallback.sourceFile)),
      runContext: {
        progress: runProgress,
        phaseName: String(runContextRaw.phase_name ?? runContextRaw.phaseName ?? phaseNameFromProgress(runProgress) ?? ""),
        bossNumber: Number.isFinite(Number(runContextRaw.boss_number ?? runContextRaw.bossNumber)) ? Number(runContextRaw.boss_number ?? runContextRaw.bossNumber) : null,
        bossNumberInferred: Boolean(runContextRaw.boss_number_inferred ?? runContextRaw.bossNumberInferred),
        bossSubphase: Number.isFinite(Number(runContextRaw.boss_subphase ?? runContextRaw.bossSubphase)) ? Number(runContextRaw.boss_subphase ?? runContextRaw.bossSubphase) : null
      },
      loadout: { available: Boolean(loadoutRaw.available), items: loadoutItems, sourceNote: String(loadoutRaw.source_note ?? loadoutRaw.sourceNote ?? "Ecliptica does not expose shop item names or stack counts in the audited VRChat log.") },
      encounter: {
        ...fallback.encounter, ...encounterRaw,
        name: String(encounterRaw.name || (src.version !== undefined ? "No active encounter" : fallback.encounter.name)), kind: String(encounterRaw.kind || (src.version !== undefined ? "Waiting" : fallback.encounter.kind)),
        phase: String(encounterRaw.phase ?? src.phase ?? (src.version !== undefined ? "Awaiting phase" : fallback.encounter.phase)), duration, active: Boolean(encounterRaw.active ?? src.active ?? (src.version === undefined)),
        hpPercent: number(encounterRaw.hp_percent ?? encounterRaw.health_percent ?? encounterRaw.hpPercent, src.version !== undefined ? 0 : fallback.encounter.hpPercent),
        hpCurrent: number(encounterRaw.hp_current ?? encounterRaw.health ?? encounterRaw.hpCurrent, src.version !== undefined ? 0 : fallback.encounter.hpCurrent),
        hpMax: number(encounterRaw.hp_max ?? encounterRaw.max_health ?? encounterRaw.hpMax, src.version !== undefined ? 0 : fallback.encounter.hpMax),
        shieldPercent: number(encounterRaw.shield_percent ?? encounterRaw.shieldPercent, src.version !== undefined ? 0 : fallback.encounter.shieldPercent)
      },
      focus: src.focus ? {
        player: String(src.focus.player ?? src.focus.owner ?? "Unknown"), entity: String(src.focus.entity ?? encounterRaw.name ?? "Boss"),
        observedAt: src.focus.observed_at ?? src.focus.observedAt ?? null, ageSeconds: number(src.focus.age_seconds ?? src.focus.ageSeconds),
        confidence: String(src.focus.confidence ?? "possible"), evidence: String(src.focus.evidence ?? "boss_network_ownership"),
        corroboratingHits: number(src.focus.corroborating_hits ?? src.focus.corroboratingHits), corroboratedAt: src.focus.corroborated_at ?? src.focus.corroboratedAt ?? null,
        sourceNote: String(src.focus.source_note ?? src.focus.sourceNote ?? "Inferred from network ownership; not authoritative hate.")
      } : null,
      outgoing: {
        ...fallback.outgoing, ...outgoingRaw, total: number(outgoingRaw.total, isRealSnapshot ? 0 : fallback.outgoing.total), dps: totalDps,
        strike: number(outgoingRaw.strike, isRealSnapshot ? 0 : fallback.outgoing.strike), nonStrike: number(outgoingRaw.non_strike ?? outgoingRaw.nonStrike, isRealSnapshot ? 0 : fallback.outgoing.nonStrike),
        hits: number(outgoingRaw.hits, isRealSnapshot ? 0 : fallback.outgoing.hits), biggestHit: number(outgoingRaw.biggest_hit ?? outgoingRaw.biggestHit, isRealSnapshot ? 0 : fallback.outgoing.biggestHit),
        rolling5s: number(outgoingRaw.rolling_5s ?? outgoingRaw.rolling5s, totalDps), rolling15s: number(outgoingRaw.rolling_15s ?? outgoingRaw.rolling15s, totalDps), rolling30s: number(outgoingRaw.rolling_30s ?? outgoingRaw.rolling30s, totalDps)
      },
      incoming: {
        ...fallback.incoming, ...incomingRaw, total: number(incomingRaw.total, isRealSnapshot ? 0 : fallback.incoming.total), hits: number(incomingRaw.hits, isRealSnapshot ? 0 : fallback.incoming.hits),
        biggestHit: number(incomingRaw.biggest_hit ?? incomingRaw.biggestHit, isRealSnapshot ? 0 : fallback.incoming.biggestHit), dps: incomingDps,
        avoidablePercent: number(incomingRaw.avoidable_percent ?? incomingRaw.avoidablePercent, src.version !== undefined ? 0 : fallback.incoming.avoidablePercent), bySource: (() => { const sources = incomingRaw.by_source ?? incomingRaw.bySource ?? (src.version !== undefined ? {} : fallback.incoming.bySource); return Array.isArray(sources) ? sources : Object.entries(sources || {}).map(([name, damage]) => ({ name: name || "(empty source)", source: name, rawSource: name, damage: number(damage), total: number(damage), hits: 0 })); })()
      },
      partyHps, critRate: number(src.crit_rate ?? src.critical_rate, weightedAverage(players, "crit", "damage")), critAvailable: src.crit_rate !== undefined || src.critical_rate !== undefined || players.some((player) => player.critAvailable), debuffUptime: number(src.debuff_uptime, src.version !== undefined ? 0 : fallback.debuffUptime), debuffAvailable: src.debuff_uptime !== undefined || src.version === undefined,
      players, roster: Array.isArray(src.roster) ? src.roster : players.map((p) => p.name),
      attacks: attacksRaw.map((attack) => normalizeAttack(attack, duration)),
      timeline,
      effects: Array.isArray(effectsRaw) ? effectsRaw.map((effect, i) => ({ ...effect, name: String(effect.name ?? effect.effect ?? "Effect"), source: String(effect.source ?? effect.actor ?? "Unknown"), kind: String(effect.kind ?? effect.type ?? "buff"), icon: String(effect.icon ?? initials(effect.name ?? effect.effect)), remaining: number(effect.remaining ?? effect.duration_remaining), stacks: number(effect.stacks), color: effect.color ?? COLORS[(i + 2) % COLORS.length] })) : fallback.effects,
      recentEvents: Array.isArray(eventsRaw) ? eventsRaw.map(normalizeEvent) : fallback.recentEvents,
      recentHits: Array.isArray(hitsRaw) ? hitsRaw.map((hit, index) => normalizeEvent({ ...hit, id: hit.id ?? `recent-hit-${index}-${hit.timestamp ?? ""}`, kind: "damage_dealt", source: observed || "Local player", action: hit.action ?? String(hit.damage_type ?? "outgoing hit").replaceAll("_", " "), target: hit.target ?? "Target not logged", flags: hit.flags ?? [String(hit.damage_type ?? "unknown").toUpperCase().replace("_", "-")], age: hit.age_seconds ?? hit.age })) : [],
      parserCoverage: src.parser_coverage ?? src.parserCoverage ?? fallback.parserCoverage,
      capabilityNote: String(src.capability_note ?? src.capabilityNote ?? fallback.capabilityNote),
      lastEventAt: src.last_event_at ?? src.lastEventAt ?? null
    };
  }

  function makeOverlayWaitingLive(serviceAvailable = false) {
    return normalizeLive({
      version: 2,
      connected: false,
      in_world: false,
      status: serviceAvailable ? "Waiting for a VRChat log" : "Local service unavailable",
      session_id: "—",
      world: "Waiting for Ecliptica",
      stage: "No stage detected",
      source_file: "No active log file",
      encounter: { name: "No active encounter", kind: "waiting", phase: "Awaiting phase", duration_seconds: 0, active: false },
      players: [], attacks: [], timeline: [], effects: [], recent_events: [], recent_hits: [], outgoing: {}, incoming: {}
    }, makeMockLive());
  }

  function weightedAverage(items, valueKey, weightKey) {
    const weight = items.reduce((sum, item) => sum + number(item[weightKey]), 0);
    return weight ? items.reduce((sum, item) => sum + number(item[valueKey]) * number(item[weightKey]), 0) / weight : 0;
  }

  function normalizedEncounterKind(value) {
    const kind = String(value ?? "").toLowerCase().replaceAll("-", "_").replaceAll(" ", "_");
    if (kind.includes("pre") && kind.includes("boss")) return "pre_boss";
    return kind.includes("boss") ? "boss" : kind || "pre_boss";
  }

  function normalizeEncounterStats(encounter, index, observed) {
    const raw = encounter && typeof encounter === "object" ? encounter : { name: encounter };
    const outgoing = raw.outgoing ?? {};
    const incoming = raw.incoming ?? {};
    const duration = number(raw.duration ?? raw.duration_seconds);
    let playersRaw = raw.players ?? raw.combatants ?? [];
    if (!Array.isArray(playersRaw)) playersRaw = Object.entries(playersRaw).map(([name, value]) => ({ name, ...(value || {}) }));
    const players = playersRaw.map((player, playerIndex) => normalizePlayer(typeof player === "string" ? { player } : player, playerIndex, duration, observed));
    const attacksRaw = Array.isArray(raw.attacks) ? raw.attacks : [];
    const totalDamage = number(raw.totalDamage ?? raw.total_damage ?? outgoing.total);
    const dps = number(raw.dps ?? outgoing.dps, duration ? totalDamage / duration : 0);
    const incomingTotal = number(incoming.total ?? raw.incoming_damage);
    const incomingDps = number(raw.incomingDps ?? raw.incoming_dps ?? incoming.damage_per_second ?? incoming.dps, duration ? incomingTotal / duration : 0);
    const endReason = String(raw.endReason ?? raw.end_reason ?? (raw.completed ? "boss_summary" : "open")).toLowerCase();
    const boundaryConfidence = String(raw.boundaryConfidence ?? raw.boundary_confidence ?? (endReason === "open" ? "open" : "structural")).toLowerCase();
    return {
      ...raw,
      id: String(raw.id ?? raw.encounter_id ?? `encounter-${index + 1}`),
      name: String(raw.name ?? raw.encounter ?? "Unnamed encounter"), stage: String(raw.stage ?? "Stage not logged"),
      kind: normalizedEncounterKind(raw.kind ?? raw.encounter_kind), duration, duration_seconds: duration,
      dps, totalDamage, hits: number(raw.hits ?? outgoing.hits), biggestHit: number(raw.biggestHit ?? raw.biggest_hit ?? outgoing.biggest_hit),
      outgoing: { ...outgoing, total: totalDamage, dps, hits: number(raw.hits ?? outgoing.hits), biggest_hit: number(raw.biggest_hit ?? outgoing.biggest_hit) },
      incoming: { ...incoming, total: incomingTotal, damage_per_second: incomingDps }, incomingDps,
      players, attacks: attacksRaw.map((attack) => normalizeAttack(attack, duration)), timeline: normalizeTimeline(raw.timeline),
      endReason, boundaryConfidence, completed: endReason !== "open"
    };
  }

  function normalizeRun(run, index, live) {
    const encounter = run.encounter && typeof run.encounter === "object" ? run.encounter : {};
    const duration = number(run.duration ?? run.duration_seconds ?? encounter.duration_seconds, live.encounter.duration);
    const runOutgoing = run.outgoing ?? {};
    const runIncoming = run.incoming ?? {};
    const encounterList = Array.isArray(run.encounters) ? run.encounters : [];
    const normalizedEncounters = encounterList.map((item, encounterIndex) => normalizeEncounterStats(item, encounterIndex, live.observedPlayer));
    const encounterEntry = [...normalizedEncounters].reverse().find((item) => item.kind === "boss") ?? normalizedEncounters.at(-1);
    const encounterFromList = encounterEntry?.name ?? encounterEntry?.encounter;
    const stageList = Array.isArray(run.stages) ? run.stages : [];
    const stageEntry = stageList.at(-1);
    const stageFromList = typeof stageEntry === "string" ? stageEntry : stageEntry?.name ?? stageEntry?.stage;
    let players = run.players ?? run.combatants ?? [];
    if (!Array.isArray(players)) players = Object.entries(players).map(([name, value]) => ({ name, ...(value || {}) }));
    players = players.map((player) => typeof player === "string" ? { player } : player);
    const normalizedPlayers = players.length ? players.map((player, playerIndex) => normalizePlayer(player, playerIndex, duration, live.observedPlayer)) : [];
    const runAttacks = Array.isArray(run.attacks) ? run.attacks : state.usingMock ? live.attacks : [];
    const runTimeline = Array.isArray(run.timeline) && run.timeline.length ? run.timeline : state.usingMock ? live.timeline : [];
    const derivedPreBossDuration = normalizedEncounters.filter((item) => item.kind === "pre_boss").reduce((sum, item) => sum + item.duration, 0);
    const preBossDuration = Math.max(number(run.pre_boss_duration_seconds ?? run.preBossDurationSeconds ?? run.pre_boss_duration ?? run.preBossDuration), derivedPreBossDuration);
    const derivedBossCount = normalizedEncounters.filter((item) => item.kind === "boss").length;
    const bossCount = Math.max(number(run.boss_count ?? run.bossCount), derivedBossCount);
    const metricsScope = String(run.metrics_scope ?? run.metricsScope ?? "").trim() || (bossCount ? "boss" : "observed_combat");
    const observedDuration = number(run.observed_duration_seconds ?? run.observedDurationSeconds ?? run.observed_duration ?? run.observedDuration) || duration + preBossDuration;
    const preBossOutgoing = run.pre_boss_outgoing ?? run.preBossOutgoing ?? {};
    const preBossIncoming = run.pre_boss_incoming ?? run.preBossIncoming ?? {};
    const totalDamage = number(run.total_damage ?? (typeof run.damage === "number" ? run.damage : undefined) ?? runOutgoing.total);
    const dps = number(run.dps ?? run.party_dps ?? runOutgoing.dps, duration ? totalDamage / duration : 0);
    const incomingDps = number(run.incoming_dps ?? run.damage_taken_per_second ?? runIncoming.damage_per_second, duration ? number(runIncoming.total) / duration : 0);
    return {
      ...run,
      id: String(run.id ?? run.run_id ?? run.number ?? index + 1), number: String(run.number ?? run.id ?? run.run_id ?? index + 1),
      encounter: String(run.encounter_name ?? encounter.name ?? (typeof run.encounter === "string" ? run.encounter : encounterFromList ?? (state.usingMock ? live.encounter.name : "No encounter name logged"))),
      stage: String(run.stage ?? run.world ?? stageFromList ?? (state.usingMock ? live.stage : "No stage logged")), result: String(run.result ?? run.outcome ?? (run.completed === false ? "ongoing" : "closed")).toLowerCase(),
      duration, dps, totalDamage, critRate: number(run.crit_rate ?? run.critical_rate),
      hps: number(run.hps ?? run.party_hps, normalizedPlayers.reduce((sum, player) => sum + player.hps, 0)), hits: number(run.hits ?? runOutgoing.hits), biggestHit: number(run.biggest_hit ?? runOutgoing.biggest_hit), incomingDps, debuffUptime: number(run.debuff_uptime),
      sourceCount: number(run.source_count ?? run.sourceCount, normalizedPlayers.length ? 1 : 0),
      deaths: number(run.deaths ?? run.knockouts), when: String(run.when ?? run.started_at_label ?? formatDate(run.started_at ?? run.timestamp)),
      players: normalizedPlayers.length ? normalizedPlayers : state.usingMock ? live.players : [],
      attacks: runAttacks.map((attack) => normalizeAttack(attack, duration)),
      timeline: normalizeTimeline(runTimeline), encounters: normalizedEncounters,
      metricsScope, observedDuration, preBossDuration, preBossOutgoing, preBossIncoming, bossCount,
      outgoing: { ...runOutgoing, total: totalDamage, dps, hits: number(run.hits ?? runOutgoing.hits), biggest_hit: number(run.biggest_hit ?? runOutgoing.biggest_hit) },
      incoming: { ...runIncoming, total: number(runIncoming.total), damage_per_second: incomingDps }
    };
  }

  function formatDate(value) {
    if (!value) return "Recorded run";
    const date = new Date(value);
    if (Number.isNaN(date.valueOf())) return String(value);
    return new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" }).format(date);
  }

  async function api(path, options = {}, timeout = 1800) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), timeout);
    try {
      const method = String(options.method || "GET").toUpperCase(); const headers = { "Accept": "application/json", ...(options.headers || {}) };
      if (options.body && !(options.body instanceof FormData)) headers["Content-Type"] = "application/json";
      const token = state.settings?.stream_token ?? state.settings?.streamToken;
      if (method !== "GET" && token) headers["X-MINMAXXER-Token"] = token;
      const response = await fetch(path, { ...options, method, signal: controller.signal, headers });
      if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
      const contentType = response.headers.get("content-type") || "";
      return contentType.includes("json") ? await response.json() : await response.text();
    } finally { clearTimeout(timer); }
  }

  async function loadInitialData() {
    const requests = await Promise.allSettled([api("/api/live"), api("/api/runs"), api("/api/settings"), api("/api/health", {}, 1000), api("/api/vr-status", {}, 1000)]);
    const livePayload = requests[0].status === "fulfilled" ? requests[0].value : null;
    state.apiOnline = requests[3].status === "fulfilled" || requests[0].status === "fulfilled";
    state.live = livePayload ? normalizeLive(livePayload, makeOverlayWaitingLive(true)) : makeOverlayWaitingLive(state.apiOnline);
    state.lastLiveAt = livePayload ? Date.now() : 0;
    // The main LIVE page never substitutes demo combat for missing telemetry. Demo factories are
    // retained only for static design fixtures; runtime state is always real or explicitly idle.
    state.usingMock = false;
    const runsPayload = requests[1].status === "fulfilled" ? requests[1].value : null;
    const rawRuns = Array.isArray(runsPayload) ? runsPayload : runsPayload?.runs ?? runsPayload?.items ?? [];
    state.runs = rawRuns.map((run, i) => normalizeRun(run, i, state.live));
    state.settings = requests[2].status === "fulfilled" && requests[2].value && typeof requests[2].value === "object" ? requests[2].value : {};
    state.vrStatus = requests[4].status === "fulfilled" ? requests[4].value : null;
    state.events = state.live.recentEvents.length ? state.live.recentEvents : [];
    state.selectedRunId = state.runs[0]?.id ?? null;
    updateConnectionUI();
  }

  function updateConnectionUI() {
    const source = $("#sourceState");
    const detail = $("#sourceDetail");
    const health = $("#healthStatus");
    if (!source || !detail || !health) return;
    const online = state.apiOnline || state.streamOnline;
    const parserConnected = online && !state.usingMock && Boolean(state.live?.connected);
    source.classList.toggle("offline", !parserConnected);
    health.classList.toggle("offline", !online);
    if (state.usingMock) {
      detail.textContent = online ? "Live snapshot unavailable; showing an offline visual demo" : "Showing a visual demo until the parser starts";
      source.querySelector("strong").textContent = "Demo data";
      health.lastChild.textContent = online ? " API" : " DEMO";
    } else if (online) {
      detail.textContent = parserConnected && state.live?.sourceFile ? tailPath(state.live.sourceFile) : "Local service ready; waiting for a VRChat log";
      source.querySelector("strong").textContent = parserConnected ? "Watching VRChat" : state.live?.status || "Waiting for VRChat";
      health.lastChild.textContent = " API";
    } else { detail.textContent = "Local service disconnected"; source.querySelector("strong").textContent = "Offline"; health.lastChild.textContent = " OFFLINE"; }
  }

  function tailPath(path) { return String(path).split(/[\\/]/).pop(); }

  function isMeaningfulArchiveSnapshot(live) {
    const source = String(live?.sourceFile ?? "").trim().toLowerCase();
    return Boolean(live?.connected) && Boolean(source) && source !== "no active log file" && !source.includes("output_log_demo");
  }

  function archiveEncounterIdentity(live) {
    const encounter = live?.encounter ?? {};
    return [String(encounter.name ?? ""), String(encounter.kind ?? ""), String(encounter.started_at ?? encounter.startedAt ?? ""), String(encounter.phase ?? "")].join("|");
  }

  function archiveRefreshReason(previous, next, bootstrap = false) {
    if (bootstrap) return "stream-bootstrap";
    if (!previous || !next) return "";
    const previousBossActive = Boolean(previous.encounter?.active) && normalizedEncounterKind(previous.encounter?.kind) === "boss";
    const nextActive = Boolean(next.encounter?.active);
    if (previousBossActive && !nextActive) return "boss-closed";
    if (previous.encounter?.active && nextActive && archiveEncounterIdentity(previous) !== archiveEncounterIdentity(next)) return "encounter-boundary";
    const previousSession = String(previous.sessionId ?? "");
    const nextSession = String(next.sessionId ?? "");
    if (previousSession !== nextSession && ((previousSession && previousSession !== "—") || (nextSession && nextSession !== "—"))) return "session-changed";
    if (previous.connected && !next.connected) return "world-exit";
    return "";
  }

  function scheduleArchiveRefresh() {
    clearTimeout(state.archiveRefreshTimer);
    state.archiveRefreshTimer = setTimeout(() => {
      state.archiveRefreshTimer = null;
      refreshRuns();
    }, 500);
  }

  function scheduleArchiveBootstrapRetry() {
    if (state.archiveBootstrapRetryTimer !== null) return;
    state.archiveBootstrapRetryTimer = setTimeout(() => {
      state.archiveBootstrapRetryTimer = null;
      refreshRuns();
    }, 4000);
  }

  function connectStream(onLive, { refreshArchive = false, onStreamState = null, clearLiveOnError = false } = {}) {
    if (!("EventSource" in window)) return;
    const stream = new EventSource("/api/stream");
    stream.addEventListener("open", () => {
      state.streamOnline = true; state.apiOnline = true; updateConnectionUI();
      if (typeof onStreamState === "function") onStreamState("open");
      if (refreshArchive && !state.archiveOpenPrimed) {
        state.archiveOpenPrimed = true;
        scheduleArchiveRefresh();
        scheduleArchiveBootstrapRetry();
      }
    });
    stream.addEventListener("error", () => {
      state.streamOnline = false;
      if (clearLiveOnError) {
        state.apiOnline = false;
        state.live = makeOverlayWaitingLive(false);
        state.lastLiveAt = 0;
        if (!state.frozen) onLive();
      }
      updateConnectionUI();
      if (typeof onStreamState === "function") onStreamState("error");
    });
    stream.addEventListener("message", (message) => {
      try {
        const payload = JSON.parse(message.data);
        if (typeof onStreamState === "function") onStreamState("message");
        state.eventCounter += 1;
        const kind = String(payload.type ?? payload.kind ?? "").toLowerCase();
        const data = payload.data ?? payload.snapshot ?? payload;
        if (kind.includes("event") && !data.players && !data.encounter) {
          const event = normalizeEvent(data);
          state.events.unshift(event);
          state.events = state.events.slice(0, 1000);
          if (state.live) state.live.recentEvents = [event, ...state.live.recentEvents].slice(0, 10);
        } else if (refreshArchive && kind.includes("run") && (kind.includes("end") || kind.includes("complete"))) {
          scheduleArchiveRefresh();
        } else {
          const leavingDemo = state.usingMock && data && typeof data === "object" && (data.version !== undefined || data.encounter !== undefined || data.players !== undefined);
          if (leavingDemo) state.usingMock = false;
          const previousLive = state.live;
          const nextLive = normalizeLive(data, state.live || makeMockLive());
          const firstStreamSnapshot = !state.archiveStreamPrimed;
          if (firstStreamSnapshot) state.archiveStreamPrimed = true;
          const firstMeaningful = !state.archiveMeaningfulPrimed && isMeaningfulArchiveSnapshot(nextLive);
          if (firstMeaningful) state.archiveMeaningfulPrimed = true;
          if (refreshArchive && archiveRefreshReason(previousLive, nextLive, firstStreamSnapshot || firstMeaningful)) scheduleArchiveRefresh();
          state.live = nextLive;
          state.lastLiveAt = Date.now();
          updateConnectionUI();
        }
        if (!state.frozen) onLive();
      } catch (_) { /* A malformed stream line must not interrupt the overlay. */ }
    });
    state.timers.push(setInterval(() => {
      state.eventRate = state.eventCounter;
      state.eventCounter = 0;
      const output = $("#eventRate");
      if (output && !document.hidden) output.textContent = `${state.eventRate} evt/s`;
    }, 1000));
  }

  async function refreshRuns() {
    try {
      const payload = await api("/api/runs");
      const raw = Array.isArray(payload) ? payload : payload.runs ?? payload.items ?? [];
      state.runs = raw.map((run, i) => normalizeRun(run, i, state.live));
      if (!state.runs.some((run) => run.id === state.selectedRunId)) state.selectedRunId = state.runs[0]?.id ?? null;
      if (!document.hidden) { renderRuns(); populateRunSelects(); }
    } catch (_) { /* Current history stays visible. */ }
  }

  function setPage(page, updateUrl = true) {
    if (!titles[page]) page = "live";
    $$(".app-view").forEach((view) => view.classList.toggle("active", view.dataset.page === page));
    $$(".nav-item").forEach((item) => {
      const active = item.dataset.view === page;
      item.classList.toggle("active", active);
      if (active) item.setAttribute("aria-current", "page"); else item.removeAttribute("aria-current");
    });
    $("#routeTitle").textContent = titles[page];
    document.title = `${titles[page]} // MINMAXXER`;
    $("#sidebar")?.classList.remove("open");
    $("#menuButton")?.setAttribute("aria-expanded", "false");
    if (updateUrl) {
      const url = new URL(location.href);
      if (page === "live") url.searchParams.delete("view"); else url.searchParams.set("view", page);
      history.replaceState({ page }, "", `${url.pathname}${url.search}${url.hash}`);
    }
    requestAnimationFrame(() => {
      if (page === "live") drawLiveChart();
      if (page === "compare") { renderComparison(); drawCompareChart(); }
      if (page === "analysis") renderAnalysis();
      if (page === "events") loadEventsForRun();
      if (page === "overlay") renderStudioPreview();
    });
  }

  function renderSpark(id, values, count = 28) {
    const root = $(id); if (!root) return;
    const points = values?.length ? values.slice(-count) : [];
    if (!points.length) { root.innerHTML = ""; return; }
    const max = Math.max(...points, 1);
    root.innerHTML = points.map((value) => `<i style="height:${clamp(value / max * 100, 7, 100).toFixed(1)}%"></i>`).join("");
  }

  function liveCombatScope(live = state.live) {
    const active = Boolean(live?.encounter?.active);
    const boss = normalizedEncounterKind(live?.encounter?.kind) === "boss";
    if (active && boss) return { key: "boss", label: "BOSS WINDOW", metric: "BOSS", excluded: false };
    if (active) return { key: "pre-boss", label: "PRE-BOSS · EXCLUDED", metric: "PRE-BOSS", excluded: true };
    return { key: "waiting", label: "WAITING FOR BOSS", metric: "BOSS", excluded: false };
  }

  function focusPresentation(live = state.live) {
    const scope = liveCombatScope(live);
    if (live?.focus && scope.key === "boss") {
      const rawConfidence = String(live.focus.confidence || "possible").toLowerCase();
      const confidence = ["likely", "possible", "aging", "stale"].includes(rawConfidence) ? rawConfidence : "possible";
      const hits = number(live.focus.corroboratingHits ?? live.focus.corroborating_hits);
      const evidence = String(live.focus.evidence || "boss_network_ownership").replaceAll("_", " ");
      const corroboration = hits ? ` · ${formatNumber(hits)} corroborating hit${hits === 1 ? "" : "s"}` : "";
      const corroborated = live.focus.corroboratedAt ?? live.focus.corroborated_at;
      return {
        player: live.focus.player || "ACQUIRING", confidence,
        detail: `${number(live.focus.ageSeconds).toFixed(1)}s ago · ${confidence.toUpperCase()} PROXY`,
        badge: confidence.toUpperCase(),
        note: `${live.focus.sourceNote || "Inferred boss target; not authoritative hate."} Evidence: ${evidence}${corroboration}${corroborated ? ` · corroborated ${formatDate(corroborated)}` : ""}.`
      };
    }
    if (scope.key === "boss") return { player: "ACQUIRING", confidence: "acquiring", detail: "watching boss activity · proxy", badge: "PROXY", note: "No target evidence has been observed yet." };
    return { player: "NO ACTIVE BOSS", confidence: "idle", detail: scope.key === "pre-boss" ? "pre-boss combat excluded" : "waiting for boss", badge: "IDLE", note: "Target inference begins when a boss window is active." };
  }

  function runPhasePresentation(live = state.live) {
    const progress = live?.runContext?.progress;
    const phaseName = live?.runContext?.phaseName || phaseNameFromProgress(progress) || "UNKNOWN";
    const percent = Number.isFinite(progress) ? `${(progress * 100).toFixed(progress === 0 || progress === 1 ? 0 : 1)}%` : "PROGRESS UNKNOWN";
    const finalEye = Number.isFinite(progress) && progress >= .999 && /bringer/i.test(`${live?.stage || ""} ${live?.encounter?.name || ""}`);
    return { name: phaseName, percent, detail: finalEye ? `${phaseName} · EYE OF THE ECLIPSE` : `${phaseName} · ${percent}`, finalEye };
  }

  function bossNumberPresentation(live = state.live) {
    const numberValue = live?.runContext?.bossNumber;
    if (!Number.isFinite(numberValue) || numberValue < 1) return { short: "—", detail: "BOSS UNKNOWN", inferred: false, known: false };
    const inferred = Boolean(live.runContext.bossNumberInferred);
    const subphase = live.runContext.bossSubphase;
    const short = `${inferred ? "~" : ""}#${String(Math.round(numberValue)).padStart(2, "0")}`;
    return { short, detail: `BOSS ${short}${Number.isFinite(subphase) && subphase > 1 ? ` · FORM ${Math.round(subphase)}` : ""}`, inferred, known: true };
  }

  function renderLive() {
    const live = state.live; if (!live) return;
    const runPhase = runPhasePresentation(live);
    const bossNumber = bossNumberPresentation(live);
    text("#encounterZone", `${live.stage || live.world}`);
    text("#encounterName", live.encounter.name);
    text("#encounterPhase", `${runPhase.detail} · ${bossNumber.detail}`);
    text("#encounterTimer", formatDuration(live.encounter.duration, true));
    text("#bossHpText", live.encounter.hpMax ? formatPercent(live.encounter.hpPercent) : "NOT LOGGED");
    text("#bossHpValue", live.encounter.hpMax ? `${formatCompact(live.encounter.hpCurrent, 2)} / ${formatCompact(live.encounter.hpMax, 2)}` : "Health not exposed by log");
    $("#bossHpBar").style.width = `${clamp(live.encounter.hpPercent, 0, 100)}%`;
    $("#bossShieldBar").style.width = `${clamp(live.encounter.shieldPercent, 0, 100)}%`;
    text("#runPhase", runPhase.finalEye ? "ECLIPSE · EYE" : `${runPhase.name} · ${runPhase.percent}`);
    text("#bossNumber", `${bossNumber.short}${bossNumber.inferred ? " · INFERRED" : ""}`);
    const loadoutItems = live.loadout?.items || [];
    text("#loadoutStatus", live.loadout?.available ? (loadoutItems.length ? `${loadoutItems.length} ITEM${loadoutItems.length === 1 ? "" : "S"}` : "EMPTY") : "NOT EXPOSED");
    if ($("#loadoutStatus")) $("#loadoutStatus").title = live.loadout?.available ? loadoutItems.map((item) => `${item.name} ×${item.stacks}`).join(" · ") || "Observed empty loadout" : live.loadout?.sourceNote || "Not available from the log.";
    const scope = liveCombatScope(live);
    text("#liveCombatScope", scope.label);
    $("#liveCombatScope")?.classList.toggle("excluded", scope.excluded);
    text("#partyDpsLabel", `LOCAL ${scope.metric} DPS${scope.excluded ? " · EXCLUDED" : ""}`);
    const focus = $("#focusWidget");
    if (focus) {
      const presentation = focusPresentation(live);
      $("strong", focus).textContent = presentation.player;
      $("em", focus).textContent = presentation.detail;
      focus.title = presentation.note;
      focus.dataset.confidence = presentation.confidence;
      focus.classList.toggle("idle", !live.focus || scope.key !== "boss");
    }
    text("#partyDps", formatCompact(live.outgoing.dps));
    if (state.usingMock) {
      $("#partyDpsDetail").innerHTML = `<b id="partyDpsDelta">▲ 8.3%</b> demo comparison`;
      $("#healingDetail").innerHTML = `<b>96.1%</b> demo effective healing`;
      $("#damageInDetail").innerHTML = `<b id="avoidableDamage">14.2%</b> demo avoidable`;
      $("#critDetail").innerHTML = `<b>${formatCompact(live.outgoing.strike,2)}</b> demo strike · ${formatCompact(live.outgoing.nonStrike,2)} non-strike`;
      $("#debuffDetail").innerHTML = `<b>2.8s</b> demo longest gap`;
    } else $("#partyDpsDetail").innerHTML = `${scope.excluded ? "<b>Excluded</b> from archived boss metrics · " : ""}${live.outgoing.hits ? `<b>${formatNumber(live.outgoing.hits)}</b> outgoing hits logged` : `<b>0</b> outgoing hits logged`}`;
    text("#partyHps", state.usingMock ? formatCompact(live.partyHps) : "N/A");
    text("#damageIn", formatCompact(live.incoming.dps));
    if (!state.usingMock) {
      $("#healingDetail").innerHTML = `<b>Not logged</b> unavailable in audited Ecliptica output`;
      $("#damageInDetail").innerHTML = `<b>${formatNumber(live.incoming.hits)}</b> incoming hits · avoidability not logged`;
      $("#critDetail").innerHTML = `<b>${formatCompact(live.outgoing.strike, 2)}</b> strike · ${formatCompact(live.outgoing.nonStrike, 2)} non-strike`;
      $("#debuffDetail").innerHTML = `<b>Not logged</b> effect uptime unavailable`;
    }
    text("#avoidableDamage", formatPercent(live.incoming.avoidablePercent));
    const strikeShare = live.outgoing.total ? live.outgoing.strike / live.outgoing.total * 100 : 0;
    text("#critRate", live.outgoing.total ? formatPercent(strikeShare) : "N/A");
    text("#debuffUptime", live.debuffAvailable === false ? "N/A" : formatPercent(live.debuffUptime));
    renderSpark("#dpsSpark", live.timeline.map((p) => p.total));
    renderSpark("#hpsSpark", state.usingMock ? live.timeline.map((p) => p.hps) : []);
    renderSpark("#inSpark", live.timeline.map((p) => p.incoming));
    renderSpark("#critSpark", state.usingMock ? live.timeline.map((_, i) => seededWave(i, strikeShare, .3, 1.7)) : []);
    renderSpark("#debuffSpark", state.usingMock ? live.timeline.map((_, i) => seededWave(i, live.debuffUptime, .09, 3)) : []);
    renderEffects(); renderParty(); renderFeed(); renderTimelineLegend(); drawLiveChart();
  }

  function text(selector, value) { const element = $(selector); if (element) element.textContent = value; }

  function renderTimelineLegend() {
    const root = $("#timelineLegend"); if (!root) return;
    const top = state.usingMock ? [...state.live.players].sort((a, b) => b.dps - a.dps).slice(0, 4) : [];
    root.innerHTML = `<span><i style="--legend:#eef7ff"></i>${state.usingMock ? "Party total" : "Local total"}</span>${top.map((p) => `<span><i style="--legend:${p.color}"></i>${escapeHtml(p.name)}</span>`).join("")}`;
  }

  function renderEffects() {
    const effects = state.live.effects.slice(0, 6);
    text("#effectCount", `${effects.length} ACTIVE`);
    $("#effectList").innerHTML = effects.length ? effects.map((effect) => `<div class="effect-row" style="--effect:${effect.color}"><span class="effect-icon">${escapeHtml(effect.icon)}</span><div><strong>${escapeHtml(effect.name)}</strong><small>${escapeHtml(effect.kind)} · ${escapeHtml(effect.source)}</small></div>${effect.stacks ? `<span class="effect-time stacks">×${formatNumber(effect.stacks)}</span>` : `<span class="effect-time">${number(effect.remaining).toFixed(1)}s</span>`}</div>`).join("") : `<div class="empty-state"><p>No effects are visible in the current log window.</p></div>`;
  }

  function partyMetricValue(player, metric) {
    if (metric === "damage") return player.damage;
    if (metric === "incoming") return player.incomingDps || (state.live.encounter.duration ? player.incoming / state.live.encounter.duration : 0);
    return player.dps;
  }

  function renderParty() {
    const players = [...state.live.players].sort((a, b) => partyMetricValue(b, state.partyMetric) - partyMetricValue(a, state.partyMetric));
    const total = players.reduce((sum, player) => sum + partyMetricValue(player, state.partyMetric), 0) || 1;
    $("#partyTableBody").innerHTML = players.length ? players.map((player, index) => {
      const value = partyMetricValue(player, state.partyMetric);
      const roleKey = player.role.toLowerCase();
      return `<tr><td class="rank-cell">${String(index + 1).padStart(2, "0")}</td><td><div class="player-cell"><span class="player-avatar" style="--player-color:${player.color}">${escapeHtml(initials(player.name))}</span>${escapeHtml(player.name)}${player.you ? '<span class="you-pill">YOU</span>' : ""}</div></td><td><span class="role-pill" style="--role-color:${ROLE_COLORS[roleKey] || ROLE_COLORS.unknown}">${escapeHtml(player.role)}</span></td><td class="numeric ${index === 0 ? "positive" : ""}">${formatCompact(player.dps)}</td><td class="numeric">${formatCompact(player.damage, 2)}</td><td class="numeric">${player.damage ? formatPercent(player.strike / player.damage * 100) : "N/A"}</td><td class="numeric">${formatPercent(player.active)}</td><td class="share-cell"><div class="share-track" style="--player-color:${player.color}"><i style="width:${value / total * 100}%"></i></div><small>${formatPercent(value / total * 100)}</small></td></tr>`;
    }).join("") : `<tr><td colspan="8"><div class="empty-state"><div><strong>Waiting for player data</strong><p>The local player row appears when Ecliptica writes attributable combat events.</p></div></div></td></tr>`;
  }

  function parseElapsedLabel(value) {
    const match = String(value ?? "").match(/(\d+):(\d+(?:\.\d+)?)/);
    return match ? number(match[1]) * 60 + number(match[2]) : NaN;
  }

  function recentHits(live = state.live, limit = 8) {
    if (!live) return [];
    const observed = String(live.observedPlayer || "").toLowerCase();
    // The backend feed is already a canonical, local-only projection. Mixing it with the raw
    // event windows duplicates the same hit under representation-specific IDs and can admit a
    // non-hit damage summary. Demo mode has no backend, so it retains the illustrative sources.
    const hitSource = state.usingMock
      ? [...(live.recentHits || []), ...(state.events || []), ...(live.recentEvents || [])]
      : (live.recentHits || []);
    const combined = hitSource.map(normalizeEvent);
    const seen = new Set();
    const sinceSnapshot = !state.usingMock && state.lastLiveAt ? Math.max(0, (Date.now() - state.lastLiveAt) / 1000) : 0;
    return combined.filter((event) => {
      const dealt = event.direction === "dealt" || event.rawType?.includes("damage_dealt") || (event.type === "damage" && String(event.source).toLowerCase() === observed && !event.flags.includes("INCOMING"));
      const key = `${event.id}|${event.time}|${event.action}|${event.amount}`;
      if (!dealt || !event.amount || seen.has(key)) return false;
      seen.add(key); return true;
    }).map((event, index) => {
      const elapsed = parseElapsedLabel(event.time);
      const timestampAge = event.timestamp ? Math.max(0, (Date.now() - Date.parse(event.timestamp)) / 1000) : NaN;
      const age = Number.isFinite(event.age) ? event.age + sinceSnapshot : Number.isFinite(timestampAge) ? timestampAge : (Number.isFinite(elapsed) ? Math.max(0, live.encounter.duration - elapsed) : index * .5) + sinceSnapshot;
      const strike = event.flags.some((flag) => String(flag).toUpperCase() === "STRIKE") && !event.flags.some((flag) => String(flag).toUpperCase() === "NON-STRIKE");
      return { ...event, age, strike };
    }).sort((a, b) => a.age - b.age).slice(0, limit);
  }

  function renderFeed() {
    const root = $("#eventFeed"); if (!root) return;
    const hits = recentHits(state.live, 7);
    const newestKey = hits[0] ? `${hits[0].id}|${hits[0].amount}|${hits[0].time}` : ""; const changed = Boolean(newestKey && state.feedHitKey !== newestKey); state.feedHitKey = newestKey;
    root.innerHTML = hits.length ? hits.map((hit, index) => `<div class="feed-event hit-feed-row ${index === 0 && changed ? "newest" : ""}" style="--event-color:${hit.strike ? "var(--cyan)" : "var(--violet)"}"><span class="feed-time">${hit.age < 10 ? `${hit.age.toFixed(1)}s` : `${Math.round(hit.age)}s`}</span><span class="feed-type">${hit.strike ? "S" : "N"}</span><span class="feed-copy"><b>${escapeHtml(hit.action)}</b> → ${escapeHtml(hit.target)}</span><span class="feed-amount">${formatCompact(hit.amount)}${hit.flags.includes("CRIT") ? " ✦" : ""}</span></div>`).join("") : `<div class="empty-state"><div><strong>Waiting for outgoing hits</strong><p>The next local damage event written to the log will appear here.</p></div></div>`;
  }

  function canvasSetup(canvas) {
    if (!canvas) return null;
    const rect = canvas.getBoundingClientRect();
    if (rect.width < 10 || rect.height < 10) return null;
    const ratio = Math.min(window.devicePixelRatio || 1, 2);
    canvas.width = Math.round(rect.width * ratio); canvas.height = Math.round(rect.height * ratio);
    const ctx = canvas.getContext("2d"); ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
    return { ctx, width: rect.width, height: rect.height };
  }

  function drawGrid(ctx, width, height, padding, max, xLabels = true) {
    const innerW = width - padding.left - padding.right, innerH = height - padding.top - padding.bottom;
    ctx.font = '8px "Cascadia Code", monospace'; ctx.textBaseline = "middle";
    for (let i = 0; i <= 4; i++) {
      const y = padding.top + innerH * i / 4;
      ctx.strokeStyle = "rgba(145,170,198,.095)"; ctx.lineWidth = 1; ctx.beginPath(); ctx.moveTo(padding.left, y + .5); ctx.lineTo(width - padding.right, y + .5); ctx.stroke();
      ctx.fillStyle = "rgba(106,122,141,.72)"; ctx.textAlign = "right"; ctx.fillText(formatCompact(max * (1 - i / 4), 0), padding.left - 7, y);
    }
    if (xLabels) for (let i = 0; i <= 5; i++) { const x = padding.left + innerW * i / 5; ctx.fillStyle = "rgba(92,106,123,.65)"; ctx.textAlign = "center"; ctx.fillText(`${i * 20}%`, x, height - 7); }
  }

  function drawSeries(ctx, values, width, height, padding, max, color, lineWidth = 1.5, fill = false, xValues = null, xMin = null, xMax = null) {
    if (!values.length) return;
    const innerW = width - padding.left - padding.right, innerH = height - padding.top - padding.bottom;
    const domainMin = xMin ?? (xValues?.[0] ?? 0), domainMax = xMax ?? (xValues?.at(-1) ?? Math.max(1, values.length - 1));
    const points = values.map((value, i) => {
      const xRatio = xValues ? (number(xValues[i]) - domainMin) / Math.max(.001, domainMax - domainMin) : i / Math.max(1, values.length - 1);
      return [padding.left + clamp(xRatio, 0, 1) * innerW, padding.top + (1 - number(value) / max) * innerH];
    });
    ctx.beginPath(); points.forEach(([x, y], index) => index ? ctx.lineTo(x, y) : ctx.moveTo(x, y));
    if (fill) { ctx.lineTo(points.at(-1)[0], height - padding.bottom); ctx.lineTo(points[0][0], height - padding.bottom); ctx.closePath(); const gradient = ctx.createLinearGradient(0, padding.top, 0, height - padding.bottom); gradient.addColorStop(0, `${color}24`); gradient.addColorStop(1, `${color}00`); ctx.fillStyle = gradient; ctx.fill(); ctx.beginPath(); points.forEach(([x, y], index) => index ? ctx.lineTo(x, y) : ctx.moveTo(x, y)); }
    ctx.strokeStyle = color; ctx.lineWidth = lineWidth; ctx.lineJoin = "round"; ctx.lineCap = "round"; ctx.stroke();
  }

  function drawLiveChart() {
    const canvas = $("#liveChart"); const setup = canvasSetup(canvas); if (!setup || !state.live) return;
    const { ctx, width, height } = setup, padding = { top: 10, right: 8, bottom: 20, left: 42 };
    let timeline = state.live.timeline;
    const activeRange = $(".chart-range .active")?.dataset.range;
    if (activeRange === "30") timeline = timeline.filter((point) => point.t >= state.live.encounter.duration - 30);
    const totals = timeline.map((point) => point.total); const max = Math.max(...totals, 1) * 1.12;
    const times = timeline.map((point) => point.t);
    const timeStart = activeRange === "30" ? Math.max(0, state.live.encounter.duration - 30) : 0;
    const timeEnd = Math.max(timeStart + 1, state.live.encounter.duration);
    ctx.clearRect(0, 0, width, height); drawGrid(ctx, width, height, padding, max, false);
    drawSeries(ctx, totals, width, height, padding, max, "#dfeaf4", 1.8, true, times, timeStart, timeEnd);
    if (state.usingMock) {
      const topPlayers = [...state.live.players].sort((a, b) => b.dps - a.dps).slice(0, 4);
      topPlayers.forEach((player) => { const originalIndex = state.live.players.indexOf(player); const values = timeline.map((point) => point.players?.[originalIndex] || 0); drawSeries(ctx, values, width, height, padding, max, player.color, 1, false, times, timeStart, timeEnd); });
    }
    ctx.fillStyle = "rgba(92,106,123,.65)"; ctx.font = '8px "Cascadia Code", monospace'; ctx.textAlign = "center";
    const innerW = width - padding.left - padding.right; for (let i = 0; i <= 5; i++) { const elapsed = activeRange === "30" ? Math.max(0, state.live.encounter.duration - 30) + i * 6 : state.live.encounter.duration * i / 5; ctx.fillText(formatDuration(elapsed), padding.left + innerW * i / 5, height - 7); }
    if (!timeline.length) { ctx.fillStyle = "rgba(132,145,163,.8)"; ctx.font = '10px "Segoe UI", sans-serif'; ctx.textAlign = "center"; ctx.fillText("Timeline samples have not been logged yet.", width / 2, height / 2); }
    state.charts.live = { timeline, padding, width, height, max, timeStart, timeEnd };
  }

  function bindChartTooltip() {
    const canvas = $("#liveChart"), tip = $("#chartTooltip"); if (!canvas || !tip) return;
    canvas.addEventListener("pointermove", (event) => {
      const chart = state.charts.live; if (!chart) return;
      const rect = canvas.getBoundingClientRect(); const x = event.clientX - rect.left;
      const ratio = clamp((x - chart.padding.left) / (chart.width - chart.padding.left - chart.padding.right), 0, 1);
      const targetTime = chart.timeStart + ratio * (chart.timeEnd - chart.timeStart);
      const point = chart.timeline.reduce((nearest, candidate) => !nearest || Math.abs(candidate.t - targetTime) < Math.abs(nearest.t - targetTime) ? candidate : nearest, null); if (!point) return;
      tip.hidden = false; tip.style.left = `${x}px`; tip.style.top = `${event.clientY - rect.top}px`; tip.innerHTML = `<strong>${formatCompact(point.total)} DPS</strong>${formatDuration(point.t)} elapsed`;
    });
    canvas.addEventListener("pointerleave", () => { tip.hidden = true; });
  }

  function renderRunSummary() {
    const bossRuns = state.runs.filter(isBossScoped);
    const fallbackRuns = state.runs.filter((run) => !isBossScoped(run));
    const best = Math.max(...bossRuns.map((run) => run.dps), 0);
    const average = bossRuns.length ? bossRuns.reduce((sum, run) => sum + run.dps, 0) / bossRuns.length : 0;
    const bossCount = bossRuns.reduce((sum, run) => sum + number(run.bossCount), 0);
    const preBossSeconds = bossRuns.reduce((sum, run) => sum + number(run.preBossDuration), 0);
    $("#runSummaryStrip").innerHTML = [
      ["Boss-scoped runs", bossRuns.length, `${fallbackRuns.length} observed-combat fallback${fallbackRuns.length === 1 ? "" : "s"}`], ["Boss fights indexed", bossCount, `${formatDuration(preBossSeconds)} pre-boss excluded`],
      ["Best boss DPS", bossRuns.length ? formatCompact(best) : "—", "Fallbacks excluded"], ["Average boss DPS", bossRuns.length ? formatCompact(average) : "—", "Boss windows only"]
    ].map(([label, value, sub]) => `<div class="summary-item"><span>${label}</span><strong>${value}</strong><small>${sub}</small></div>`).join("");
  }

  function filteredRuns() {
    const query = $("#runSearch")?.value.trim().toLowerCase() || ""; const sort = $("#runSort")?.value || "recent";
    let runs = state.runs.filter((run) => (state.runFilter === "all" || run.result === state.runFilter) && (!query || `${run.encounter} ${run.stage} ${run.players.map((p) => p.name).join(" ")}`.toLowerCase().includes(query)));
    if (sort === "dps") runs.sort((a, b) => Number(isBossScoped(b)) - Number(isBossScoped(a)) || b.dps - a.dps); else if (sort === "duration") runs.sort((a, b) => Number(isBossScoped(b)) - Number(isBossScoped(a)) || a.duration - b.duration);
    return runs;
  }

  function isBossScoped(run) { return String(run?.metricsScope ?? run?.metrics_scope ?? "").toLowerCase() === "boss" && number(run?.bossCount ?? run?.boss_count) > 0; }
  function runMetricLabels(run) {
    const party = number(run?.sourceCount ?? run?.source_count) > 1;
    return isBossScoped(run)
      ? { scope: "boss", dps: `BOSS ${party ? "PARTY" : "LOCAL"} DPS`, damage: "BOSS DAMAGE", time: "BOSS TIME", short: "BOSS" }
      : { scope: "observed", dps: `OBSERVED ${party ? "PARTY" : "LOCAL"} DPS`, damage: "OBSERVED DAMAGE", time: "OBSERVED TIME", short: "OBS" };
  }
  function runDpsLabel(run) { return runMetricLabels(run).dps; }
  function preBossTotal(run, direction) {
    const stats = direction === "incoming" ? run?.preBossIncoming : run?.preBossOutgoing;
    return number(stats?.total ?? stats?.damage ?? stats);
  }

  function renderRuns() {
    if (!$("#runList")) return;
    renderRunSummary(); const runs = filteredRuns();
    $("#runList").innerHTML = runs.length ? runs.map((run) => { const labels = runMetricLabels(run); const bossScoped = isBossScoped(run); return `<button class="run-row ${run.id === state.selectedRunId ? "active" : ""}" data-run-id="${escapeHtml(run.id)}"><span class="run-result ${run.result}">${run.result === "ongoing" ? "LIVE" : bossScoped ? `B${formatNumber(run.bossCount)}` : "OBS"}</span><span class="run-name"><strong>${escapeHtml(run.encounter)}</strong><span>#${escapeHtml(run.number)} · ${bossScoped ? `${formatDuration(run.preBossDuration)} pre-boss excluded` : "observed-combat fallback · no boss boundary"}</span></span><span class="run-stat"><span>${labels.dps}</span><strong>${formatCompact(run.dps)}</strong></span><span class="run-stat"><span>${labels.damage}</span><strong>${formatCompact(run.totalDamage, 2)}</strong></span><span class="run-stat"><span>${labels.time}</span><strong>${formatDuration(run.duration, true)}</strong></span><span class="run-arrow">›</span></button>`; }).join("") : `<div class="empty-state"><div><strong>No matching runs</strong><p>Try a different filter or import an earlier VRChat log.</p></div></div>`;
    $$(".run-row", $("#runList")).forEach((row) => row.addEventListener("click", () => selectRun(row.dataset.runId)));
    renderRunInspector();
  }

  async function selectRun(id) {
    state.selectedRunId = id; renderRuns();
    if (!state.usingMock) {
      try {
        const detail = await api(`/api/runs/${safeId(id)}`);
        const index = state.runs.findIndex((run) => run.id === id);
        if (index >= 0) state.runs[index] = normalizeRun(detail.run ?? detail, index, state.live);
        renderRunInspector(); populateRunSelects();
      } catch (_) { /* Summary data remains usable. */ }
    }
  }

  function renderRunInspector() {
    const root = $("#runInspector"); if (!root) return;
    const run = state.runs.find((item) => item.id === state.selectedRunId) ?? state.runs[0];
    if (!run) { root.innerHTML = `<div class="empty-state"><p>Select a run to inspect it.</p></div>`; return; }
    const top = [...run.players].sort((a, b) => b.dps - a.dps).slice(0, 5); const max = top[0]?.dps || 1;
    const labels = runMetricLabels(run), bossScoped = isBossScoped(run);
    root.innerHTML = `<div class="inspector-cover"><span class="result-badge ${run.result}">${bossScoped ? "BOSS METRICS · PRE-BOSS EXCLUDED" : "OBSERVED COMBAT FALLBACK · NO BOSS BOUNDARY"}</span><h2>${escapeHtml(run.encounter)}</h2><p>${escapeHtml(run.stage)} · Run #${escapeHtml(run.number)} · ${bossScoped ? `${formatNumber(run.bossCount)} boss fight${run.bossCount === 1 ? "" : "s"}` : "partial import or no BossStarted event"}</p></div><div class="inspector-body"><div class="inspector-metrics"><div class="inspector-metric"><span>${labels.time}</span><strong>${formatDuration(run.duration, true)}</strong></div><div class="inspector-metric"><span>${labels.dps}</span><strong>${formatCompact(run.dps)}</strong></div><div class="inspector-metric"><span>${labels.damage}</span><strong>${formatCompact(run.totalDamage, 2)}</strong></div><div class="inspector-metric"><span>${bossScoped ? "PRE-BOSS EXCLUDED" : "BOSS FIGHTS"}</span><strong>${bossScoped ? formatDuration(run.preBossDuration, true) : "0"}</strong></div></div><div class="scope-note compact">${bossScoped ? `${formatCompact(preBossTotal(run, "outgoing"), 2)} pre-boss outgoing is retained separately and does not affect the boss DPS above.` : "No boss boundary was detected. These observed-combat values are excluded from boss rankings and must not be interpreted as boss performance."}</div><div class="inspector-section"><h3>${bossScoped ? run.sourceCount > 1 ? "Boss player contribution" : "Boss observed player" : run.sourceCount > 1 ? "Observed player contribution" : "Observed player"}</h3>${top.map((p) => `<div class="mini-player" style="--player-color:${p.color};--width:${p.dps / max * 100}%"><span>${escapeHtml(p.name)}</span><b>${formatCompact(p.dps)}</b><i></i></div>`).join("")}</div><div class="inspector-actions"><button class="ghost-button" data-compare-run="${escapeHtml(run.id)}">Compare</button><button class="primary-button" data-analyze-run="${escapeHtml(run.id)}">${bossScoped ? "Analyze bosses" : "Analyze fallback"}</button></div></div>`;
    $("[data-compare-run]", root)?.addEventListener("click", () => { $("#compareA").value = run.id; setPage("compare"); });
    $("[data-analyze-run]", root)?.addEventListener("click", () => { $("#analysisRunSelect").value = run.id; setPage("analysis"); });
  }

  function populateRunSelects() {
    ["#compareA", "#compareB", "#analysisRunSelect", "#eventRunSelect"].forEach((selector, selectorIndex) => {
      const select = $(selector); if (!select) return; const current = select.value;
      select.innerHTML = state.runs.map((run) => `<option value="${escapeHtml(run.id)}">#${escapeHtml(run.number)} · ${escapeHtml(run.encounter)} · ${formatCompact(run.dps)} ${isBossScoped(run) ? "boss DPS" : "observed DPS fallback"}</option>`).join("");
      if (current && state.runs.some((run) => run.id === current)) select.value = current; else if (selectorIndex === 1 && state.runs[1]) select.value = state.runs[1].id; else if (state.selectedRunId) select.value = state.selectedRunId;
    });
  }

  function getCompareRuns() { return [state.runs.find((run) => run.id === $("#compareA")?.value) ?? state.runs[0], state.runs.find((run) => run.id === $("#compareB")?.value) ?? state.runs[1] ?? state.runs[0]]; }
  function delta(a, b, inverse = false) { const value = b - a; const good = inverse ? value < 0 : value > 0; return { value, good }; }

  function renderComparison() {
    if (!$("#compareSummary")) return; const [a, b] = getCompareRuns(); if (!a || !b) return;
    const bossA = isBossScoped(a), bossB = isBossScoped(b);
    const comparisonScope = bossA && bossB ? "boss" : !bossA && !bossB ? "observed" : "mixed";
    const partyLabel = a.sourceCount > 1 && b.sourceCount > 1 ? "party" : a.sourceCount <= 1 && b.sourceCount <= 1 ? "local" : "logged";
    const prefix = comparisonScope === "boss" ? "Boss" : comparisonScope === "observed" ? "Observed" : "Mixed-scope";
    text("#compareScopeNote", comparisonScope === "boss"
      ? `Boss-fight windows only · ${formatDuration(a.preBossDuration)} / ${formatDuration(b.preBossDuration)} pre-boss combat excluded from runs A / B`
      : comparisonScope === "observed"
        ? "Observed-combat fallback comparison · neither run has a detected boss boundary · excluded from boss rankings"
        : "Scope mismatch · one run is boss-only and the other is an observed-combat fallback · deltas are informational, not a boss comparison");
    const chartPanel = $(".compare-layout .chart-panel", $('[data-page="compare"]'));
    if (chartPanel) {
      $(".eyebrow", chartPanel).textContent = comparisonScope === "boss" ? "BOSS WINDOWS" : comparisonScope === "observed" ? "OBSERVED FALLBACK WINDOWS" : "MIXED SCOPES";
      $("h2", chartPanel).textContent = `${prefix} damage curve`;
      $(".unit-label", chartPanel).textContent = `${prefix.toUpperCase()} DPS / elapsed %`;
    }
    const mixedNeutral = comparisonScope === "mixed";
    const metrics = [
      [`${prefix} ${partyLabel} DPS`, a.dps, b.dps, false, (v) => formatCompact(Math.abs(v)), mixedNeutral], [`${prefix} time`, a.duration, b.duration, false, (v) => `${Math.abs(v).toFixed(1)}s`, true],
      [`${prefix} damage`, a.totalDamage, b.totalDamage, false, (v) => formatCompact(Math.abs(v)), true], [`${prefix} incoming DPS`, a.incomingDps, b.incomingDps, true, (v) => formatCompact(Math.abs(v)), mixedNeutral]
    ];
    $("#compareSummary").innerHTML = metrics.map(([label, av, bv, inverse, formatter, neutral]) => { const d = delta(av, bv, inverse); const direction = d.value >= 0 ? "Higher" : "Lower"; return `<div class="delta-card" style="--delta-color:${neutral ? "var(--cyan)" : d.good ? "var(--mint)" : "var(--rose)"}"><span>${label}</span><strong>${d.value >= 0 ? "+" : "−"}${formatter(d.value)}</strong><small>${neutral ? `${direction} in comparison` : `${d.good ? "Improved" : "Regressed"} in comparison`}</small></div>`; }).join("");
    $("#compareLegend").innerHTML = `<span><i style="--legend:#54e6ff"></i>#${escapeHtml(a.number)} ${escapeHtml(a.encounter)}</span><span><i style="--legend:#ad8cff"></i>#${escapeHtml(b.number)} ${escapeHtml(b.encounter)}</span>`;
    const extraMetrics = state.usingMock ? [["Critical rate", a.critRate, b.critRate, false, (v) => `${Math.abs(v).toFixed(1)} pt`, mixedNeutral], ["Demo HPS", a.hps, b.hps, false, (v) => formatCompact(Math.abs(v)), mixedNeutral]] : [[`${prefix} outgoing hits`, a.hits, b.hits, false, (v) => formatNumber(Math.abs(v)), true], [`Largest ${prefix.toLowerCase()} hit`, a.biggestHit, b.biggestHit, false, (v) => formatCompact(Math.abs(v)), true]];
    $("#compareTable").innerHTML = metrics.concat(extraMetrics).map(([label, av, bv, inverse, formatter, neutral]) => { const d = delta(av, bv, inverse); const lower = label.toLowerCase(); return `<div class="metric-delta-row"><span>${label}</span><b>${lower.includes("time") || lower.includes("duration") ? formatDuration(bv, true) : lower.includes("rate") || lower.includes("uptime") ? formatPercent(bv) : formatCompact(bv)}</b><em class="${neutral ? "" : d.good ? "positive" : "negative"}">${d.value >= 0 ? "+" : "−"}${formatter(d.value)}</em></div>`; }).join("");
    const names = [...new Set([...a.players, ...b.players].map((p) => p.name))];
    $("#playerDeltaBody").innerHTML = names.map((name) => { const pa = a.players.find((p) => p.name === name); const pb = b.players.find((p) => p.name === name); const av = pa?.dps || 0, bv = pb?.dps || 0, change = bv - av; return `<tr><td><div class="player-cell"><span class="player-avatar" style="--player-color:${pb?.color || pa?.color || COLORS[0]}">${escapeHtml(initials(name))}</span>${escapeHtml(name)}</div></td><td class="numeric">${formatCompact(av)}</td><td class="numeric">${formatCompact(bv)}</td><td class="numeric ${mixedNeutral ? "" : change >= 0 ? "positive" : "negative"}">${change >= 0 ? "+" : "−"}${formatCompact(Math.abs(change))}</td><td><div class="impact-track"><i style="--impact:${Math.min(48,Math.abs(change)/400)}%;--offset:${change < 0 ? "-" : ""}${Math.min(48,Math.abs(change)/400)}%;--impact-color:${mixedNeutral ? "var(--cyan)" : change >= 0 ? "var(--mint)" : "var(--rose)"}"></i></div></td></tr>`; }).join("");
    drawCompareChart();
  }

  function drawCompareChart() {
    const canvas = $("#compareChart"); const setup = canvasSetup(canvas); if (!setup) return; const [a, b] = getCompareRuns(); if (!a || !b) return;
    const { ctx, width, height } = setup, padding = { top: 12, right: 10, bottom: 24, left: 42 };
    const valuesA = (a.timeline || []).map((point) => number(point.total ?? point.dps)); const valuesB = (b.timeline || []).map((point) => number(point.total ?? point.dps)); const max = Math.max(...valuesA, ...valuesB, 1) * 1.12;
    const timesA = (a.timeline || []).map((point) => number(point.t) / Math.max(1, a.duration)); const timesB = (b.timeline || []).map((point) => number(point.t) / Math.max(1, b.duration));
    ctx.clearRect(0, 0, width, height); drawGrid(ctx, width, height, padding, max, true); drawSeries(ctx, valuesA, width, height, padding, max, "#54e6ff", 2, true, timesA, 0, 1); drawSeries(ctx, valuesB, width, height, padding, max, "#ad8cff", 2, false, timesB, 0, 1);
    if (!valuesA.length && !valuesB.length) { ctx.fillStyle = "rgba(132,145,163,.8)"; ctx.font = '10px "Segoe UI", sans-serif'; ctx.textAlign = "center"; ctx.fillText("Timeline samples are not available for these archived runs.", width / 2, height / 2); }
  }

  function selectedAnalysisRun() { return state.runs.find((run) => run.id === $("#analysisRunSelect")?.value) ?? state.runs[0]; }

  function bossEncounters(run) { return (run?.encounters || []).filter((encounter) => normalizedEncounterKind(encounter.kind) === "boss"); }

  function populateAnalysisEncounterSelect(run) {
    const select = $("#analysisEncounterSelect"); if (!select || !run) return;
    const bosses = bossEncounters(run);
    const remembered = state.analysisEncounterByRun[run.id] ?? "all-bosses";
    select.innerHTML = isBossScoped(run)
      ? `<option value="all-bosses">All boss fights · ${bosses.length || run.bossCount || 0}</option>${bosses.map((boss, index) => `<option value="${escapeHtml(boss.id)}">Boss ${index + 1} · ${escapeHtml(boss.name)} · ${formatDuration(boss.duration, true)}</option>`).join("")}`
      : `<option value="all-bosses">Observed combat fallback · no boss boundary</option>`;
    select.value = bosses.some((boss) => boss.id === remembered) ? remembered : "all-bosses";
    state.analysisEncounterByRun[run.id] = select.value;
  }

  function selectedAnalysisContext(run) {
    const selected = state.analysisEncounterByRun[run?.id] ?? $("#analysisEncounterSelect")?.value ?? "all-bosses";
    const boss = bossEncounters(run).find((encounter) => encounter.id === selected);
    return boss ? { ...boss, number: run.number, sourceCount: run.sourceCount, parentRun: run, metricsScope: "boss", bossCount: 1 } : run;
  }

  function analysisScopeLabel(run, context) {
    if (!isBossScoped(run)) return `Observed combat fallback · ${formatDuration(run.duration, true)} captured · no BossStarted boundary · excluded from boss rankings`;
    if (context !== run && context?.id) return `${context.name} · ${formatDuration(context.duration, true)} boss window · pre-boss excluded`;
    return `All ${run.bossCount || bossEncounters(run).length} boss fight${(run.bossCount || bossEncounters(run).length) === 1 ? "" : "s"} · ${formatDuration(run.preBossDuration)} pre-boss excluded`;
  }

  function analysisMetricLabels(context) {
    const boss = isBossScoped(context);
    return boss
      ? { boss, adjective: "boss", upper: "BOSS", dps: "BOSS DPS", damage: "BOSS DAMAGE", hits: "BOSS HITS", time: "BOSS TIME", selection: "selected boss window", events: "boss-window events" }
      : { boss, adjective: "observed", upper: "OBSERVED", dps: "OBSERVED DPS", damage: "OBSERVED DAMAGE", hits: "OBSERVED HITS", time: "OBSERVED TIME", selection: "observed-combat fallback", events: "observed combat events · no boss boundary" };
  }

  function populateAnalysisPlayerSelect(run, context = run) {
    const control = $("#analysisPlayerControl"), select = $("#analysisPlayerSelect");
    if (!control || !select) return;
    const players = [...(context?.players || [])].sort((a, b) => b.damage - a.damage);
    control.hidden = state.analysisTab !== "players" || !players.length;
    const memoryKey = run ? `${run.id}:${context?.id ?? "all-bosses"}` : null;
    const remembered = memoryKey ? state.analysisPlayerByRun[memoryKey] : null;
    const current = players.some((player) => player.name === remembered) ? remembered : players[0]?.name;
    select.innerHTML = players.map((player) => `<option value="${escapeHtml(player.name)}">${escapeHtml(player.name)} · ${escapeHtml(player.className || "Class not logged")}</option>`).join("");
    if (current) {
      select.value = current;
      state.analysisPlayerByRun[memoryKey] = current;
    }
  }

  function renderAnalysis() {
    const root = $("#analysisContent"); if (!root) return; const run = selectedAnalysisRun(); if (!run) return;
    populateAnalysisEncounterSelect(run);
    const encounterControl = $("#analysisEncounterControl");
    if (encounterControl) encounterControl.hidden = state.analysisTab === "encounter";
    const context = state.analysisTab === "encounter" ? run : selectedAnalysisContext(run);
    populateAnalysisPlayerSelect(run, context);
    if (state.analysisTab === "players") renderPlayerAnalysis(root, context);
    else if (state.analysisTab === "encounter") renderEncounterAnalysis(root, run);
    else if (state.analysisTab === "attacks") renderAttackAnalysis(root, context);
    else renderIncomingAnalysis(root, context);
    root.insertAdjacentHTML("afterbegin", `<div class="scope-note analysis-scope"><strong>${isBossScoped(run) ? "BOSS-ONLY ANALYSIS" : "OBSERVED COMBAT FALLBACK"}</strong><span>${escapeHtml(analysisScopeLabel(run, context))}</span></div>`);
    root.insertAdjacentHTML("beforeend", recentHitsAnalysisMarkup());
  }

  function recentHitsAnalysisMarkup() {
    const hits = recentHits(state.live, 8);
    return `<article class="panel analytics-hit-panel"><header class="panel-header"><div><span class="eyebrow">LIVE HIT MEMORY</span><h2>Previous local outgoing hits</h2></div><span class="unit-label">STRIKE / NON-STRIKE FROM LOG</span></header>${hits.length ? `<div class="data-table-wrap"><table class="data-table"><thead><tr><th>Age</th><th>Attack</th><th>Target</th><th>Kind</th><th class="numeric">Amount</th><th>Flags</th></tr></thead><tbody>${hits.map((hit) => `<tr><td class="rank-cell">${hit.age < 10 ? hit.age.toFixed(1) : Math.round(hit.age)}s</td><td>${escapeHtml(hit.action)}</td><td>${escapeHtml(hit.target)}</td><td><span class="hit-kind ${hit.strike ? "strike" : "non-strike"}">${hit.strike ? "STRIKE" : "NON-STRIKE"}</span></td><td class="numeric">${formatNumber(hit.amount)}</td><td>${hit.flags.filter((flag) => !["STRIKE", "NON-STRIKE"].includes(flag)).map((flag) => `<span class="flag" style="--flag-color:var(--amber)">${escapeHtml(flag)}</span>`).join("") || "—"}</td></tr>`).join("")}</tbody></table></div>` : `<div class="empty-state"><p>No local outgoing hit events are available in the current log window.</p></div>`}</article>`;
  }

  function renderPlayerAnalysis(root, run) {
    const selectedName = $("#analysisPlayerSelect")?.value;
    const player = run.players.find((candidate) => candidate.name === selectedName) ?? [...run.players].sort((a, b) => b.damage - a.damage)[0] ?? (state.usingMock ? state.live.players[0] : null);
    if (!player) { root.innerHTML = `<div class="panel empty-state"><div><strong>No attributed player data</strong><p>This run does not contain player-attributed events in the imported logs.</p></div></div>`; return; }
    const attacks = Array.isArray(player.attacks) ? [...player.attacks].sort((a, b) => b.damage - a.damage).slice(0, 7) : []; const maxAttack = Math.max(...attacks.map((a) => a.damage), 1);
    const scope = analysisMetricLabels(run);
    if (!state.usingMock) {
      root.innerHTML = `<div class="analysis-grid"><article class="panel"><div class="player-analysis-hero"><span class="large-avatar">${escapeHtml(initials(player.name))}</span><div><h2>${escapeHtml(player.name)} ${player.you ? '<span class="you-pill">YOU</span>' : ""}</h2><p>${escapeHtml(player.className)} · ${scope.events} · Run #${escapeHtml(run.number)}</p></div><div class="analysis-score"><strong>${formatCompact(player.dps)}</strong><span>${scope.dps}</span></div></div><div class="analysis-metrics"><div class="analysis-metric"><span>${scope.damage}</span><strong>${formatCompact(player.damage,2)}</strong><small>${run.totalDamage ? formatPercent(player.damage/run.totalDamage*100) : "Not logged"} ${run.sourceCount > 1 ? `${scope.adjective} party share` : `of ${scope.adjective} total`}</small></div><div class="analysis-metric"><span>${scope.hits}</span><strong>${formatNumber(player.hits)}</strong><small>Logged during ${scope.selection}</small></div><div class="analysis-metric"><span>${scope.upper} COMBAT SPAN</span><strong>${formatPercent(player.active)}</strong><small>First-to-last ${scope.adjective} event; not uptime</small></div><div class="analysis-metric"><span>LARGEST ${scope.upper} HIT</span><strong>${formatCompact(player.biggestHit)}</strong><small>${player.damage ? `${formatPercent(player.strike/player.damage*100)} strike share` : "Not logged"}</small></div></div><div class="breakdown-list">${attacks.length ? attacks.map((attack, index) => `<div class="breakdown-row"><span class="name"><strong>${escapeHtml(attack.name)}</strong><small>${formatNumber(attack.hits)} hits · largest ${formatCompact(attack.max)}</small></span><span class="breakdown-bar" style="--bar-color:${COLORS[index%COLORS.length]}"><i style="width:${attack.damage/maxAttack*100}%"></i></span><span class="breakdown-value">${formatCompact(attack.damage,2)}</span><span class="breakdown-share">${player.damage ? formatPercent(attack.damage/player.damage*100) : "—"}</span></div>`).join("") : `<div class="empty-state"><p>No per-player damage-category breakdown was logged for this ${scope.selection}.</p></div>`}</div></article><article class="panel"><header class="panel-header"><div><span class="eyebrow">LOG COVERAGE</span><h2>What this view can prove</h2></div></header><div class="callout-list"><div class="callout" style="--callout:var(--mint)"><span>✓</span><div><strong>${scope.boss ? "Boss damage and hit counts are direct" : "Observed damage and hit counts are direct"}</strong><small>${scope.boss ? "Pre-boss events are excluded from every amount above." : "No boss boundary was detected; these values are observed combat, not boss performance."}</small></div></div><div class="callout" style="--callout:var(--amber)"><span>—</span><div><strong>Rotation order is not logged</strong><small>Cooldown alignment and ability sequencing cannot be reconstructed.</small></div></div><div class="callout" style="--callout:var(--amber)"><span>—</span><div><strong>Critical hits are not logged</strong><small>Strike and non-strike are damage categories, not inferred criticals.</small></div></div></div></article></div>`;
      return;
    }
    root.innerHTML = `<div class="analysis-grid"><article class="panel"><div class="player-analysis-hero"><span class="large-avatar">${escapeHtml(initials(player.name))}</span><div><h2>${escapeHtml(player.name)} ${player.you ? '<span class="you-pill">YOU</span>' : ""}</h2><p>${escapeHtml(player.className)} · ${escapeHtml(player.role)} · Run #${escapeHtml(run.number)}</p></div><div class="analysis-score"><strong>${formatCompact(player.dps)}</strong><span>${scope.dps}</span></div></div><div class="analysis-metrics"><div class="analysis-metric"><span>${scope.damage}</span><strong>${formatCompact(player.damage,2)}</strong><small>${formatPercent(player.damage/run.totalDamage*100)} ${scope.adjective} party share</small></div><div class="analysis-metric"><span>CRITICAL RATE</span><strong>${formatPercent(player.crit)}</strong><small>Across ${scope.adjective} hits</small></div><div class="analysis-metric"><span>${scope.upper} ACTIVE TIME</span><strong>${formatPercent(player.active)}</strong><small>${formatDuration(run.duration*player.active/100)} effective</small></div><div class="analysis-metric"><span>${scope.upper} DOWNTIME</span><strong>${(run.duration*(100-player.active)/100).toFixed(1)}s</strong><small>Movement + mechanics</small></div></div><div class="breakdown-list">${attacks.map((attack, index) => `<div class="breakdown-row"><span class="name"><strong>${escapeHtml(attack.name)}</strong><small>${formatNumber(attack.hits)} hits · ${formatPercent(attack.crit)} crit</small></span><span class="breakdown-bar" style="--bar-color:${COLORS[index%COLORS.length]}"><i style="width:${attack.damage/maxAttack*100}%"></i></span><span class="breakdown-value">${formatCompact(attack.damage,2)}</span><span class="breakdown-share">${formatPercent(attack.damage/player.damage*100)}</span></div>`).join("")}</div></article><article class="panel"><header class="panel-header"><div><span class="eyebrow">ROTATION QUALITY</span><h2>${scope.boss ? "Boss priority uptime" : "Observed activity"}</h2></div></header><div class="uptime-ring" style="--value:${player.active}"><div><strong>${formatPercent(player.active)}</strong><span>${scope.upper} ACTIVE TIME</span></div></div><div class="callout-list"><div class="callout" style="--callout:var(--mint)"><span>↑</span><div><strong>Strong burst alignment</strong><small>Peak output overlaps Solar Alignment in 5 of 6 windows.</small></div></div><div class="callout" style="--callout:var(--amber)"><span>!</span><div><strong>2.8 second debuff gap</strong><small>Eclipse Brand dropped shortly before the third burst.</small></div></div><div class="callout" style="--callout:var(--cyan)"><span>◇</span><div><strong>Movement loss is low</strong><small>Only ${(run.duration*(100-player.active)/100).toFixed(1)} seconds without logged actions.</small></div></div></div></article></div>`;
  }

  function endReasonPresentation(encounter) {
    const labels = {
      boss_defeated: ["Boss defeated", "explicit"], boss_summary: ["Boss summary", "explicit"], next_boss: ["Next boss began", "structural"],
      boss_started: ["Boss began", "structural"], next_stage: ["Next stage began", "structural"], intermission: ["Intermission", "structural"],
      lobby: ["Lobby return", "structural"], world_exit: ["World exit", "structural"], open: ["Still active", "open"]
    };
    const reason = String(encounter.endReason ?? encounter.end_reason ?? "open").toLowerCase();
    const [label, fallbackConfidence] = labels[reason] ?? [reason.replaceAll("_", " ") || "Boundary observed", "structural"];
    const confidence = String(encounter.boundaryConfidence ?? encounter.boundary_confidence ?? fallbackConfidence).toLowerCase();
    return { label, confidence };
  }

  function encounterRows(encounters, kind) {
    if (!encounters.length) return `<tr><td colspan="8"><div class="empty-state"><p>${kind === "boss" ? "No boss windows were detected in this run." : "No pre-boss combat was logged."}</p></div></td></tr>`;
    return encounters.map((encounter) => {
      const end = endReasonPresentation(encounter);
      return `<tr><td>${escapeHtml(encounter.name || (kind === "boss" ? "Unnamed boss" : "Approach combat"))}</td><td>${escapeHtml(encounter.stage || "Not logged")}</td><td class="numeric">${formatDuration(encounter.duration ?? encounter.duration_seconds, true)}</td><td class="numeric">${formatCompact(encounter.outgoing?.total ?? encounter.totalDamage, 2)}</td><td class="numeric">${formatCompact(encounter.outgoing?.dps ?? encounter.dps)}</td><td class="numeric">${formatCompact(encounter.incoming?.total, 2)}</td><td><span class="boundary-badge ${end.confidence}">${escapeHtml(end.label)}</span></td><td><span class="confidence-label ${end.confidence}">${escapeHtml(end.confidence)}</span></td></tr>`;
    }).join("");
  }

  function renderEncounterAnalysis(root, run) {
    if (!isBossScoped(run)) {
      const observedEncounters = Array.isArray(run.encounters) ? run.encounters : [];
      root.innerHTML = `<div class="analysis-grid encounter-analysis"><article class="panel wide-panel encounter-group"><header class="panel-header"><div><span class="eyebrow">FALLBACK SCOPE</span><h2>Observed combat · no boss boundary</h2></div><span class="unit-label">NOT BOSS-RANKED</span></header><p class="section-explainer">This partial import contains combat, but no reliable BossStarted boundary. Nothing below is labeled boss or pre-boss, and this run is excluded from boss rankings.</p><div class="analysis-metrics"><div class="analysis-metric"><span>OBSERVED TIME</span><strong>${formatDuration(run.duration,true)}</strong><small>Fallback analysis denominator</small></div><div class="analysis-metric"><span>OBSERVED DAMAGE</span><strong>${formatCompact(run.totalDamage,2)}</strong><small>Not boss damage</small></div><div class="analysis-metric"><span>OBSERVED DPS</span><strong>${formatCompact(run.dps)}</strong><small>Not boss DPS</small></div><div class="analysis-metric"><span>OBSERVED SEGMENTS</span><strong>${formatNumber(observedEncounters.length)}</strong><small>No boss classification</small></div></div></article><article class="panel wide-panel"><header class="panel-header"><div><span class="eyebrow">WHY FALLBACK</span><h2>Boundary unavailable</h2></div></header><div class="callout-list"><div class="callout" style="--callout:var(--amber)"><span>—</span><div><strong>No BossStarted event was captured</strong><small>This can happen with a partial log import or when observation begins after the boss boundary.</small></div></div><div class="callout" style="--callout:var(--cyan)"><span>i</span><div><strong>Values remain inspectable</strong><small>Players, damage categories, and incoming tabs use observed-combat labels and never enter boss leaderboards.</small></div></div></div></article></div>`;
      return;
    }
    let encounters = Array.isArray(run.encounters) ? run.encounters : [];
    if (!encounters.length && run.bossCount) encounters = [normalizeEncounterStats({ id: `${run.id}-boss-summary`, name: run.encounter, stage: run.stage, kind: "boss", duration_seconds: run.duration, outgoing: run.outgoing, incoming: run.incoming, end_reason: run.result === "ongoing" ? "open" : "boss_summary", boundary_confidence: run.result === "ongoing" ? "open" : "structural" }, 0, state.live?.observedPlayer)];
    const bosses = encounters.filter((encounter) => normalizedEncounterKind(encounter.kind) === "boss");
    const preBoss = encounters.filter((encounter) => normalizedEncounterKind(encounter.kind) === "pre_boss");
    root.innerHTML = `<div class="analysis-grid encounter-analysis"><article class="panel wide-panel encounter-group boss-group"><header class="panel-header"><div><span class="eyebrow">MIN-MAX SCOPE</span><h2>Boss fights</h2></div><span class="unit-label">${bosses.length} BOSS WINDOW${bosses.length === 1 ? "" : "S"}</span></header><p class="section-explainer">Only these windows contribute to boss DPS, boss damage, player rankings, and comparisons.</p><div class="data-table-wrap"><table class="data-table"><thead><tr><th>Boss</th><th>Stage</th><th class="numeric">Boss time</th><th class="numeric">Boss damage</th><th class="numeric">Boss DPS</th><th class="numeric">Incoming</th><th>Window ended by</th><th>Boundary</th></tr></thead><tbody>${encounterRows(bosses, "boss")}</tbody></table></div></article><article class="panel wide-panel encounter-group preboss-group"><header class="panel-header"><div><span class="eyebrow">EXCLUDED SCOPE</span><h2>Pre-boss · excluded</h2></div><span class="unit-label">${formatDuration(run.preBossDuration, true)} EXCLUDED</span></header><p class="section-explainer">Trash mobs and approach combat remain available for auditing, but never dilute boss DPS.</p><div class="data-table-wrap"><table class="data-table"><thead><tr><th>Segment</th><th>Stage</th><th class="numeric">Excluded time</th><th class="numeric">Outgoing</th><th class="numeric">DPS</th><th class="numeric">Incoming</th><th>Segment ended by</th><th>Boundary</th></tr></thead><tbody>${encounterRows(preBoss, "pre_boss")}</tbody></table></div></article><article class="panel"><header class="panel-header"><div><span class="eyebrow">SCOPE TOTALS</span><h2>Boss vs. observed run</h2></div></header><div class="analysis-metrics" style="grid-template-columns:repeat(2,1fr)"><div class="analysis-metric"><span>BOSS TIME</span><strong>${formatDuration(run.duration,true)}</strong><small>Primary analysis denominator</small></div><div class="analysis-metric"><span>BOSS DAMAGE</span><strong>${formatCompact(run.totalDamage,2)}</strong><small>Primary outgoing total</small></div><div class="analysis-metric excluded"><span>PRE-BOSS TIME</span><strong>${formatDuration(run.preBossDuration,true)}</strong><small>Excluded from boss metrics</small></div><div class="analysis-metric excluded"><span>OBSERVED TIME</span><strong>${formatDuration(run.observedDuration,true)}</strong><small>Full visit wall-clock, including transitions</small></div></div></article><article class="panel"><header class="panel-header"><div><span class="eyebrow">BOUNDARY LOGIC</span><h2>How windows close</h2></div></header><div class="callout-list"><div class="callout" style="--callout:var(--mint)"><span>✓</span><div><strong>Explicit boss evidence</strong><small>Boss defeat and boss-summary lines provide the strongest endings.</small></div></div><div class="callout" style="--callout:var(--cyan)"><span>↦</span><div><strong>Structural endings are valid</strong><small>A next boss, stage, intermission, lobby, or world exit closes the prior window without needing a matching end marker.</small></div></div><div class="callout" style="--callout:var(--amber)"><span>●</span><div><strong>Open means currently accumulating</strong><small>It no longer implies that an alternating end marker was missed.</small></div></div></div></article></div>`;
  }

  function renderAttackAnalysis(root, run) {
    const attacks = run.attacks?.length ? run.attacks : state.usingMock ? state.live.attacks : [];
    const max = Math.max(...attacks.map((attack) => number(attack.damage)), 1);
    const hasSources = attacks.some((attack) => attack.sourceAvailable);
    const scope = analysisMetricLabels(run);
    root.innerHTML = `<div class="analysis-grid"><article class="panel wide-panel"><header class="panel-header"><div><span class="eyebrow">${scope.upper} OUTGOING DAMAGE</span><h2>${scope.boss ? "Boss" : "Observed"} damage category breakdown</h2></div><span class="unit-label">${attacks.length} ${scope.upper} CATEGORIES</span></header><div class="data-table-wrap"><table class="data-table"><thead><tr><th>#</th><th>Category</th>${hasSources ? "<th>Logged source</th>" : ""}<th class="numeric">${scope.adjective} damage</th><th class="numeric">${scope.adjective} DPS</th><th class="numeric">${scope.adjective} hits</th><th class="numeric">Critical</th><th class="numeric">Largest</th><th>${scope.boss ? "Boss" : "Observed"} share</th></tr></thead><tbody>${attacks.map((attack,i)=>`<tr><td class="rank-cell">${String(i+1).padStart(2,"0")}</td><td>${escapeHtml(attack.name)}</td>${hasSources ? `<td>${attack.sourceAvailable ? escapeHtml(attack.source) : "—"}</td>` : ""}<td class="numeric">${formatCompact(attack.damage,2)}</td><td class="numeric">${formatCompact(attack.dps)}</td><td class="numeric">${formatNumber(attack.hits)}</td><td class="numeric">${attack.critAvailable ? formatPercent(attack.crit) : "N/A"}</td><td class="numeric">${formatCompact(attack.max)}</td><td class="share-cell"><div class="share-track" style="--player-color:${COLORS[i%COLORS.length]}"><i style="width:${attack.damage/max*100}%"></i></div><small>${run.totalDamage ? formatPercent(attack.damage/run.totalDamage*100) : "—"}</small></td></tr>`).join("")}</tbody></table></div>${attacks.length ? "" : `<div class="empty-state"><p>No damage-category breakdown was logged for this ${scope.selection}.</p></div>`}${state.usingMock ? "" : `<div class="callout" style="--callout:var(--amber);margin-top:12px"><span>—</span><div><strong>Categories are not ability names</strong><small>These ${scope.boss ? "boss-only" : "observed fallback"} labels come from logged damage types; the log does not identify skills or rotation order.</small></div></div>`}</article></div>`;
  }

  function renderIncomingAnalysis(root, run) {
    const scope = analysisMetricLabels(run);
    const rawSources = run.incoming?.by_source ?? run.incoming?.bySource ?? null;
    let sources = Array.isArray(rawSources) ? rawSources : rawSources && typeof rawSources === "object" ? Object.entries(rawSources).map(([name, damage]) => ({ name: name || "(empty source)", source: name, rawSource: name, damage: number(damage), hits: 0 })) : [];
    sources = sources.map((source) => { const rawName = String(source.name ?? source.source ?? "Unknown"); return rawName.length ? source : { ...source, name: "(empty source)", rawSource: rawName }; });
    if (state.usingMock && !sources.length) sources = [
      { name: "Eclipse Pulse", damage: 1240000, hits: 19 }, { name: "Dark Matter", damage: 718400, hits: 8, avoidable: true },
      { name: "Starfall Barrage", damage: 603100, hits: 24 }, { name: "Fractured Reality", damage: 421900, hits: 31 }, { name: "Event Horizon", damage: 286600, hits: 4, avoidable: true }
    ];
    const max = Math.max(...sources.map((source) => number(source.damage ?? source.total)),1); const incomingTotal = number(run.incoming?.total, run.incomingDps * run.duration);
    root.innerHTML = `<div class="analysis-grid"><article class="panel"><header class="panel-header"><div><span class="eyebrow">${scope.upper} PRESSURE</span><h2>${scope.boss ? "Boss" : "Observed"} incoming damage sources</h2></div><span class="unit-label">${formatCompact(run.incomingDps)} ${scope.upper} IN / SEC</span></header><div class="breakdown-list">${sources.length ? sources.map((source,i)=>{const damage=number(source.damage??source.total);return `<div class="breakdown-row"><span class="name"><strong>${escapeHtml(source.name??source.source??"Unknown")}</strong><small>${source.hits ? `${formatNumber(source.hits)} ${scope.adjective} hits` : "Hit count not split by source"}${state.usingMock&&source.avoidable?" · demo avoidable":""}</small></span><span class="breakdown-bar" style="--bar-color:${state.usingMock&&source.avoidable?"var(--rose)":COLORS[i%COLORS.length]}"><i style="width:${damage/max*100}%"></i></span><span class="breakdown-value">${formatCompact(damage,2)}</span><span class="breakdown-share">${incomingTotal ? formatPercent(damage/incomingTotal*100) : "—"}</span></div>`}).join("") : `<div class="empty-state"><p>No incoming source breakdown was logged for this ${scope.selection}.</p></div>`}</div>${state.usingMock ? "" : `<div class="callout" style="--callout:var(--amber);margin-top:12px"><span>—</span><div><strong>Avoidability is not logged</strong><small>${scope.boss ? "Boss" : "Observed fallback"} source totals do not imply that a hit could have been prevented.</small></div></div>`}</article><article class="panel"><header class="panel-header"><div><span class="eyebrow">${scope.upper} TARGET LOAD</span><h2>${scope.boss ? "Boss" : "Observed"} damage taken by player</h2></div></header><div class="callout-list">${run.players?.length ? [...run.players].sort((a,b)=>(b.incoming||0)-(a.incoming||0)).map((player,i)=>`<div class="callout" style="--callout:${player.color}"><span>${i+1}</span><div><strong>${escapeHtml(player.name)} · ${formatCompact(player.incoming||player.incomingDps*run.duration,2)}</strong><small>${formatCompact(player.incomingDps||player.incoming/Math.max(run.duration,1))} ${scope.adjective} incoming DPS · ${escapeHtml(player.className||"Class not logged")}</small></div></div>`).join("") : `<div class="empty-state"><p>No player-attributed ${scope.adjective} incoming events were logged.</p></div>`}</div></article></div>`;
  }

  async function loadEventsForRun() {
    const runId = $("#eventRunSelect")?.value; if (!runId || state.usingMock) { renderEvents(); return; }
    try { const payload = await api(`/api/events?run_id=${safeId(runId)}`); const raw = Array.isArray(payload) ? payload : payload.events ?? payload.items ?? []; state.events = raw.map(normalizeEvent); state.eventLimit = 60; } catch (_) { showToast("Could not load events; keeping the current event set.", true); }
    renderEvents();
  }

  function visibleEvents() {
    const query = $("#eventSearch")?.value.trim().toLowerCase() || ""; const type = $("#eventTypeFilter")?.value || "all"; const strikeOnly = $("#strikeOnly")?.checked;
    return state.events.filter((event) => (type === "all" || event.type === type) && (!strikeOnly || event.flags.includes("STRIKE")) && (!query || `${event.source} ${event.action} ${event.target} ${event.raw}`.toLowerCase().includes(query)));
  }

  function renderEvents() {
    const root = $("#eventTableBody"); if (!root) return; const all = visibleEvents(), events = all.slice(0, state.eventLimit);
    const outgoingHits = all.filter((event) => event.rawType === "damage_dealt" || event.direction === "dealt");
    const outgoingAmount = outgoingHits.reduce((sum, event) => sum + event.amount, 0);
    $("#eventStatline").innerHTML = `<span><b>${formatNumber(all.length)}</b> matching rows</span><span><b>${formatCompact(outgoingAmount,2)}</b> outgoing hit damage</span><span><b>${formatNumber(outgoingHits.filter((event)=>event.flags.includes("STRIKE")).length)}</b> strike hit rows</span><span>Summaries and progress values are excluded from hit totals</span>`;
    root.innerHTML = events.length ? events.map((event) => `<tr><td class="rank-cell">${escapeHtml(event.time)}</td><td><span class="event-kind" style="--event-color:${EVENT_COLORS[event.type]}"><i></i>${escapeHtml(event.type)}</span></td><td>${escapeHtml(event.source)}</td><td>${escapeHtml(event.action)}</td><td>${escapeHtml(event.target)}</td><td class="numeric ${event.type === "heal" ? "positive" : ""}">${event.amount ? formatNumber(event.amount) : "—"}</td><td>${event.flags.map((flag) => `<span class="flag" style="--flag-color:${flag === "CRIT" ? "var(--amber)" : flag === "AVOIDABLE" ? "var(--rose)" : "var(--muted)"}">${escapeHtml(flag)}</span>`).join("")}</td></tr>`).join("") : `<tr><td colspan="7"><div class="empty-state"><div><strong>No matching combat events</strong><p>Clear a filter or search for another actor, action, or target.</p></div></div></td></tr>`;
    $("#loadMoreEvents").hidden = all.length <= state.eventLimit;
  }

  function overlayOptionsFromSearch(search = location.search) {
    const params = new URLSearchParams(search); const hasShow = params.has("show"); const showRaw = params.get("show") ?? "";
    const hitMode = ["hits", "recent_hits"].includes(params.get("mode"));
    const parsedShow = hasShow ? showRaw.split(",").map((item) => item === "recent_hits" ? "hits" : item).filter((item) => ["dps", "damage", "incoming", "encounter", "phase", "boss", "hits", "focus", "loadout"].includes(item)) : hitMode ? ["hits", "focus", "phase", "boss"] : ["dps", "damage", "encounter", "phase", "boss", "hits", "focus"];
    if (hasShow && params.get("ui") !== String(OVERLAY_SETTINGS_VERSION) && parsedShow.includes("encounter")) {
      if (!parsedShow.includes("phase")) parsedShow.push("phase");
      if (!parsedShow.includes("boss")) parsedShow.push("boss");
    }
    const requestedLayout = params.get("layout") || (hitMode ? "hits" : "leaderboard");
    return {
      surface: params.get("surface") === "desktop" ? "desktop" : "browser",
      profile: ["broadcast", "minimal", "vr"].includes(params.get("profile")) ? params.get("profile") : "broadcast",
      layout: ["leaderboard", "compact", "ticker", "hits"].includes(requestedLayout) ? requestedLayout : "leaderboard",
      theme: ["void", "glass"].includes(params.get("theme")) ? params.get("theme") : "void",
      accent: ACCENTS[params.get("accent")] ? params.get("accent") : "cyan",
      rows: clamp(Math.round(number(params.get("rows") ?? 5, 5)), 1, 8),
      hitRows: clamp(Math.round(number(params.get("hit_rows") ?? params.get("hitRows") ?? 4, 4)), 1, 8), show: parsedShow,
      bg: clamp(number(params.get("bg") ?? 78, 78), 0, 100), scale: clamp(number(params.get("scale") ?? 100, 100), 70, 160), statusCards: true
    };
  }

  function metricValue(player, metric, duration) {
    if (metric === "damage") return player.damage;
    if (metric === "incoming") return player.incomingDps || player.incoming / Math.max(duration, 1);
    return player.dps;
  }

  function renderCombatOverlay(root, live, options) {
    if (!root || !live) return;
    const accent = ACCENTS[options.accent] || ACCENTS.cyan;
    const status = options.statusCards && state.overlayServiceState === "connecting"
      ? { tone: "connecting", eyebrow: "MINMAXXER", title: "CONNECTING TO LOCAL SERVICE", detail: "Waiting for the local HUD service…", badge: "CONNECTING" }
      : options.statusCards && state.overlayServiceState === "lost"
        ? { tone: "lost", eyebrow: "MINMAXXER OFFLINE", title: "LOCAL SERVICE DISCONNECTED", detail: "Start MINMAXXER, then refresh this Browser Source if it does not reconnect.", badge: "RECONNECTING" }
        : options.statusCards && (!live.connected || !live.inWorld)
          ? { tone: "ready", eyebrow: "MINMAXXER READY", title: "NO LIVE ECLIPTICA INSTANCE", detail: "Join Ecliptica in VRChat. This overlay will update automatically.", badge: "WAITING FOR VRCHAT" }
          : null;
    if (status) {
      root.innerHTML = `<section class="combat-overlay surface-${options.surface} overlay-status-card status-${status.tone} profile-${options.profile} layout-${options.layout} theme-${options.theme}" style="--overlay-accent:${accent[0]};--overlay-accent-rgb:${accent[1]};--overlay-opacity:${options.bg/100};--overlay-scale:${options.scale/100}" aria-label="MINMAXXER overlay status"><header class="overlay-head"><div class="overlay-brand"><span class="overlay-logo">MX</span><div class="overlay-title"><strong>MINMAXXER</strong><span>LOCAL OBS HUD</span></div></div><div class="overlay-status-badge"><i></i>${escapeHtml(status.badge)}</div></header><div class="overlay-status-body"><span>${escapeHtml(status.eyebrow)}</span><strong>${escapeHtml(status.title)}</strong><p>${escapeHtml(status.detail)}</p></div><footer class="overlay-foot"><span class="live-dot"><i></i>${escapeHtml(status.badge)}</span><span>127.0.0.1 // PRIVATE</span></footer></section>`;
      return;
    }
    const displayDuration = live.encounter.duration + (live.encounter.active && !state.usingMock && state.lastLiveAt ? Math.max(0, (Date.now() - state.lastLiveAt) / 1000) : 0);
    const metrics = ["dps", "damage", "incoming"].filter((metric) => options.show.includes(metric));
    const primaryMetric = metrics[0] || null;
    const players = primaryMetric ? [...live.players].sort((a, b) => metricValue(b, primaryMetric, live.encounter.duration) - metricValue(a, primaryMetric, live.encounter.duration)).slice(0, options.rows) : [];
    const max = primaryMetric ? Math.max(...players.map((p) => metricValue(p, primaryMetric, live.encounter.duration)), 1) : 1;
    const scope = liveCombatScope(live);
    const metricLabel = { dps: `${scope.metric} DPS`, damage: `${scope.metric} DAMAGE`, incoming: `${scope.metric} INCOMING` };
    const metricFormat = (_, value) => formatCompact(value);
    const totals = metrics.slice(0, options.layout === "ticker" ? 1 : 3);
    const hits = recentHits(live, options.hitRows || 4);
    const newestHitKey = hits[0] ? `${hits[0].id}|${hits[0].amount}|${hits[0].time}` : ""; const hitChanged = Boolean(newestHitKey && overlayHitMemory.get(root) !== newestHitKey); overlayHitMemory.set(root, newestHitKey);
    const phaseState = runPhasePresentation(live);
    const bossState = bossNumberPresentation(live);
    const runChips = [];
    if (options.show.includes("phase")) runChips.push(`<span class="overlay-context-chip phase"><small>RUN PHASE</small><strong>${escapeHtml(phaseState.name)}</strong><em>${escapeHtml(phaseState.finalEye ? "EYE OF THE ECLIPSE" : phaseState.percent)}</em></span>`);
    if (options.show.includes("boss")) runChips.push(`<span class="overlay-context-chip boss${bossState.inferred ? " inferred" : ""}" title="${bossState.inferred ? "Estimated from Ecliptica's logged run progress after a mid-run observation." : bossState.known ? "Counted from Ecliptica stage markers in this run." : "Boss number has not been observed yet."}"><small>CURRENT BOSS</small><strong>${escapeHtml(bossState.short)}</strong><em>${escapeHtml(Number.isFinite(live.runContext?.bossSubphase) && live.runContext.bossSubphase > 1 ? `FORM ${Math.round(live.runContext.bossSubphase)}` : bossState.inferred ? "INFERRED" : bossState.known ? "RUN ORDER" : "AWAITING STAGE")}</em></span>`);
    const runContextWidget = runChips.length ? `<div class="overlay-run-context">${runChips.join("")}</div>` : "";
    const focusState = focusPresentation(live);
    const focus = options.show.includes("focus") ? `<div class="overlay-focus confidence-${escapeHtml(focusState.confidence)} ${live.focus && scope.key === "boss" ? "" : "idle"}" title="${escapeHtml(focusState.note)}"><span>BOSS TARGET?</span><strong>${escapeHtml(focusState.player)}</strong><em>${escapeHtml(focusState.detail)}</em><i>${escapeHtml(focusState.badge)}</i></div>` : "";
    const loadoutItems = live.loadout?.items || [];
    const loadout = options.show.includes("loadout") ? `<section class="overlay-loadout${live.loadout?.available ? "" : " unavailable"}" title="${escapeHtml(live.loadout?.sourceNote || "Loadout telemetry is unavailable.")}"><header><span>LOCAL LOADOUT</span><small>${live.loadout?.available ? `${loadoutItems.length} OBSERVED` : "NOT EXPOSED BY LOG"}</small></header>${live.loadout?.available ? `<div>${loadoutItems.length ? loadoutItems.slice(0,8).map((item)=>`<span><b>${escapeHtml(item.name)}</b><i>×${formatNumber(item.stacks)}</i></span>`).join("") : `<em>Observed empty loadout</em>`}</div>` : `<div><em>Ecliptica did not log shop item names or stacks.</em></div>`}</section>` : "";
    const hitWidget = (options.show.includes("hits") || options.layout === "hits") ? `<section class="overlay-hit-widget" aria-label="Previous local outgoing hits"><header><span>PREVIOUS HITS</span><small>LOCAL OUTGOING · LIVE</small></header>${hits.length ? hits.map((hit,index)=>`<div class="overlay-hit-row ${index===0&&hitChanged?"newest":""}"><span class="hit-age">${hit.age<10?hit.age.toFixed(1):Math.round(hit.age)}s</span><span class="hit-action">${escapeHtml(hit.action)}</span><span class="hit-type ${hit.strike?"strike":"non-strike"}">${hit.strike?"STRIKE":"NON-STRIKE"}</span><strong>${formatNumber(hit.amount)}${hit.flags.includes("CRIT")?" ✦":""}</strong></div>`).join("") : `<div class="overlay-hit-empty">Waiting for outgoing damage…</div>`}</section>` : "";
    const totalsWidget = totals.length ? `<div class="overlay-totals">${totals.map((metric,i)=>`<div class="overlay-total ${i===0?"accent":""}"><span>LOCAL ${metricLabel[metric]}${scope.excluded ? " · EXCLUDED" : ""}</span><strong>${metricFormat(metric,metric==="dps"?live.outgoing.dps:metric==="damage"?live.outgoing.total:live.incoming.dps)}</strong></div>`).join("")}</div>` : "";
    const rosterWidget = primaryMetric ? `<div class="overlay-roster"><div class="overlay-columns"><span>#</span><span>PLAYER</span>${metrics.map((metric)=>`<span>${metricLabel[metric]}</span>`).join("")}</div>${players.map((player,index)=>{const roleKey=player.role.toLowerCase();const rgb=hexToRgb(player.color);return `<div class="overlay-player" style="--share:${metricValue(player,primaryMetric,live.encounter.duration)/max*100};--player-rgb:${rgb}"><span class="overlay-rank">${String(index+1).padStart(2,"0")}</span><span class="overlay-player-name"><strong>${escapeHtml(player.name)}${player.you?" · YOU":""}</strong><small style="color:${ROLE_COLORS[roleKey]||ROLE_COLORS.unknown}">${escapeHtml(player.className||player.role)}</small></span>${metrics.map((metric,metricIndex)=>`<span class="overlay-player-value ${metricIndex===0?"primary":""}"><strong>${metricFormat(metric,metricValue(player,metric,live.encounter.duration))}</strong><small>${metric === "dps" ? formatPercent(player.dps/Math.max(live.outgoing.dps,1)*100) : metricLabel[metric]}</small></span>`).join("")}</div>`}).join("")}</div>` : "";
    root.innerHTML = `<section class="combat-overlay surface-${options.surface} profile-${options.profile} layout-${options.layout} theme-${options.theme}" style="--overlay-accent:${accent[0]};--overlay-accent-rgb:${accent[1]};--overlay-opacity:${options.bg/100};--overlay-scale:${options.scale/100};--metric-count:${metrics.length};--total-cols:${Math.max(1,totals.length)}" aria-label="Live Ecliptica combat overlay"><header class="overlay-head"><div class="overlay-brand"><span class="overlay-logo">MX</span><div class="overlay-title"><strong>${escapeHtml(options.show.includes("encounter") ? live.encounter.name : options.layout === "hits" ? "PREVIOUS HITS" : "MINMAXXER")}</strong><span>${escapeHtml(options.show.includes("encounter") ? `${live.stage} · ${live.world}` : "ECLIPTICA COMBAT")}</span></div></div><div class="overlay-timer"><strong>${formatDuration(displayDuration,true)}</strong><span>${live.encounter.active ? `● ${scope.label}` : live.connected ? scope.label : "WAITING"}</span></div></header>${runContextWidget}${focus}${totalsWidget}${loadout}${rosterWidget}${hitWidget}<footer class="overlay-foot"><span class="live-dot"><i></i>${state.usingMock?"DEMO DATA":live.connected?"LOG CONNECTED":"WAITING FOR LOG"}</span><span>${escapeHtml(scope.label)} // ${escapeHtml(live.stage)}</span></footer></section>`;
  }

  function hexToRgb(hex) { const value = String(hex).replace("#", ""); const parsed = parseInt(value.length === 3 ? value.split("").map((c) => c + c).join("") : value, 16); return `${(parsed >> 16) & 255},${(parsed >> 8) & 255},${parsed & 255}`; }

  function readStudioOptions() {
    return {
      profile: $(".profile-card.active")?.dataset.profile || "broadcast", layout: $("#overlayLayout")?.value || "leaderboard", theme: $("#overlayTheme")?.value || "void",
      accent: $("#accentOptions button.active")?.dataset.accent || "cyan", rows: number($("#overlayRows")?.value, 5), hitRows: number($("#overlayHitRows")?.value, 4), scale: number($("#overlayScale")?.value, 100), bg: number($("#overlayBg")?.value, 78),
      show: $$(".show-options input:checked:not(:disabled)").map((input) => input.value)
    };
  }

  function overlayUrl(options) {
    const params = new URLSearchParams();
    params.set("ui", OVERLAY_SETTINGS_VERSION); params.set("profile", options.profile); params.set("layout", options.layout); params.set("theme", options.theme); params.set("accent", options.accent); params.set("rows", options.rows); params.set("hit_rows", options.hitRows); params.set("show", options.show.join(",")); params.set("bg", options.bg); params.set("scale", options.scale);
    return `${DEFAULT_ORIGIN}/overlay?${params}`;
  }

  function renderVrStatus() {
    const root = $("#vrOverlayStatus"); if (!root) return;
    const enabled = Boolean(state.settings.vr_overlay_enabled ?? state.settings.vrOverlayEnabled ?? state.settings.vr_overlay?.enabled);
    const status = state.vrStatus;
    if (!enabled) { root.textContent = "Disabled · enable after SteamVR is running"; return; }
    if (!status) { root.textContent = "Checking the native OpenVR worker…"; return; }
    if (status.last_error) { root.textContent = `OpenVR error · ${status.last_error}`; return; }
    if (status.visible) { root.textContent = `Visible in SteamVR · ${formatNumber(status.frames_submitted)} frames submitted`; return; }
    if (status.active) { root.textContent = status.placement_note || "SteamVR connected · waiting for a HUD frame"; return; }
    root.textContent = status.runtime_available ? "SteamVR runtime found · initializing overlay" : "Waiting for the SteamVR runtime";
  }

  async function refreshVrStatus() {
    try { state.vrStatus = await api("/api/vr-status", {}, 1000); } catch (_) { state.vrStatus = null; }
    renderVrStatus();
  }

  function queueOverlayProfileSave(options) {
    state.profileSavePendingOptions = { ...options };
    clearTimeout(state.profileSaveTimer);
    state.profileSaveTimer = setTimeout(flushOverlayProfileSave, 450);
  }

  async function flushOverlayProfileSave() {
    if (state.profileSaveInFlight || !state.profileSavePendingOptions) return;
    const options = state.profileSavePendingOptions;
    state.profileSavePendingOptions = null;
    state.profileSaveInFlight = true;
    let failed = false;
    const overlayPatch = { ...options, schema_version: OVERLAY_SETTINGS_VERSION, hit_rows: options.hitRows, recent_hit_rows: options.hitRows };
    try {
      await api("/api/settings", { method: "PUT", body: JSON.stringify({ overlay: overlayPatch }) });
      // A newer edit may have arrived while this request was in flight. Its local pending copy
      // must remain canonical until the serialized follow-up request succeeds.
      if (!state.profileSavePendingOptions) {
        state.studioBackendPending = false;
        storeStudioOptions(options);
      }
    } catch (_) {
      failed = true;
      // Preserve the newest payload (or this one when no newer edit exists) for launch-time retry.
      if (!state.profileSavePendingOptions) state.profileSavePendingOptions = options;
    } finally {
      state.profileSaveInFlight = false;
      if (!failed && state.profileSavePendingOptions) {
        clearTimeout(state.profileSaveTimer);
        state.profileSaveTimer = setTimeout(flushOverlayProfileSave, 0);
      }
    }
  }

  function storeStudioOptions(options) {
    try { localStorage.setItem("minmaxxer.overlay", JSON.stringify({ ...options, schemaVersion: OVERLAY_SETTINGS_VERSION, backendPending: state.studioBackendPending })); }
    catch (_) { /* Storage is optional. */ }
  }

  function renderStudioPreview(persist = false) {
    if (!$("#studioOverlayPreview") || !state.live) return; const options = readStudioOptions(); state.overlay = options;
    if (persist) state.studioBackendPending = true;
    text("#overlayRowsValue", options.rows); text("#overlayHitRowsValue", options.hitRows); text("#overlayScaleValue", `${options.scale}%`); text("#overlayBgValue", `${options.bg}%`);
    $("#obsUrl").value = overlayUrl(options); renderCombatOverlay($("#studioOverlayPreview"), state.live, { ...options, scale: 100, statusCards: false });
    storeStudioOptions(options);
    renderVrStatus();
    if (persist) queueOverlayProfileSave(options);
  }

  function studioOptionsFromSettings() {
    const desktop = state.settings?.desktop_overlay ?? state.settings?.desktopOverlay ?? {};
    const profiles = state.settings?.overlay_profiles ?? state.settings?.overlayProfiles;
    if (!Array.isArray(profiles) || !profiles.length) return null;
    const requested = String(desktop.profile || "broadcast");
    const profile = profiles.find((item) => String(item.id) === requested) || profiles[0];
    const show = [];
    if (profile.show_dps) show.push("dps");
    if (profile.show_damage) show.push("damage");
    if (profile.show_incoming) show.push("incoming");
    if (profile.show_encounter) show.push("encounter");
    if (profile.show_phase) show.push("phase");
    if (profile.show_boss_number) show.push("boss");
    if (profile.show_hits || profile.show_recent_hits) show.push("hits");
    if (profile.show_focus) show.push("focus");
    return {
      profile: String(profile.id || requested), layout: String(profile.layout || "leaderboard"), theme: String(profile.theme || "void"), accent: profile.accent === "#8ff0cf" ? "mint" : String(profile.accent || "cyan"),
      rows: number(profile.rows, 5), hitRows: number(profile.recent_hit_rows ?? profile.recentHitRows, 4), scale: number(profile.scale, 1) * 100,
      bg: number(desktop.opacity, .78) * 100, show, schemaVersion: OVERLAY_SETTINGS_VERSION
    };
  }

  function applySavedStudioOptions() {
    let local = null; try { local = JSON.parse(localStorage.getItem("minmaxxer.overlay")); } catch (_) { local = null; }
    const localVersion = number(local?.schemaVersion ?? local?.schema_version, 1);
    const migrateLocal = Boolean(local) && localVersion < OVERLAY_SETTINGS_VERSION;
    const retryLocal = Boolean(local?.backendPending);
    let saved = migrateLocal || retryLocal ? local : studioOptionsFromSettings() || local;
    if (!saved) return;
    $$(".profile-card").forEach((button) => button.classList.toggle("active", button.dataset.profile === saved.profile));
    if (saved.layout) $("#overlayLayout").value = saved.layout;
    if (saved.theme) $("#overlayTheme").value = ["void", "glass"].includes(saved.theme) ? saved.theme : "void";
    if (saved.rows) $("#overlayRows").value = saved.rows; if (saved.hitRows) $("#overlayHitRows").value = saved.hitRows; if (saved.scale) $("#overlayScale").value = saved.scale; if (saved.bg !== undefined) $("#overlayBg").value = saved.bg;
    $$("#accentOptions button").forEach((button) => button.classList.toggle("active", button.dataset.accent === saved.accent));
    if (Array.isArray(saved.show)) {
      const restoredShow = new Set(saved.show);
      if (number(saved.schemaVersion, 1) < OVERLAY_SETTINGS_VERSION && restoredShow.has("encounter")) { restoredShow.add("phase"); restoredShow.add("boss"); }
      $$(".show-options input").forEach((input) => { input.checked = !input.disabled && restoredShow.has(input.value); });
    }
    if (migrateLocal || retryLocal) {
      state.studioBackendPending = true;
      const migrated = readStudioOptions();
      storeStudioOptions(migrated);
      queueOverlayProfileSave(migrated);
    }
  }

  async function setOutputEnabled(kind, enabled, button) {
    button.setAttribute("aria-checked", String(enabled)); button.classList.toggle("active", enabled);
    const nestedKey = `${kind}_overlay`; state.settings[`${kind}_overlay_enabled`] = enabled; state.settings[nestedKey] = { ...(state.settings[nestedKey] || {}), enabled };
    const studio = readStudioOptions(); const overlayPatch = { ...studio, schema_version: OVERLAY_SETTINGS_VERSION, hit_rows: studio.hitRows, recent_hit_rows: studio.hitRows };
    try { await api("/api/settings", { method: "PUT", body: JSON.stringify({ [`${kind}_overlay_enabled`]: enabled, [nestedKey]: { enabled }, overlay: overlayPatch }) }); showToast(`${kind === "vr" ? "VR" : "Desktop"} overlay ${enabled ? "enabled" : "disabled"}.`); if (kind === "vr") setTimeout(refreshVrStatus, 500); }
    catch (_) { state.settings[`${kind}_overlay_enabled`] = !enabled; state.settings[nestedKey] = { ...(state.settings[nestedKey] || {}), enabled: !enabled }; setSwitch(button, !enabled); showToast(`The ${kind} overlay setting was not changed because the service is offline.`, true); }
  }

  async function setDesktopHeadsetVisibility(button) {
    const enabled = button.getAttribute("aria-checked") !== "true";
    setSwitch(button, enabled);
    try {
      await api("/api/settings", { method: "PUT", body: JSON.stringify({ desktop_overlay: { show_when_vr_active: enabled } }) });
      state.settings.desktop_overlay = { ...(state.settings.desktop_overlay || {}), show_when_vr_active: enabled };
      showToast(`Desktop HUD ${enabled ? "will remain visible" : "will hide"} when a headset is detected.`);
    } catch (_) { setSwitch(button, !enabled); showToast("The desktop headset setting was not changed.", true); }
  }

  function hydrateVrControls(values = state.settings.vr_overlay || {}) {
    const defaults = { x: .30, y: .08, z: -1.05, width_m: .78, opacity: .92, controller_grab_enabled: false };
    const merged = { ...defaults, ...values };
    [["#vrX", "x"], ["#vrY", "y"], ["#vrZ", "z"], ["#vrWidth", "width_m"], ["#vrOpacity", "opacity"]].forEach(([selector, key]) => { if ($(selector)) $(selector).value = number(merged[key], defaults[key]).toFixed(2); });
    setSwitch($("#vrGrabToggle"), Boolean(merged.controller_grab_enabled));
  }

  function readVrPlacement() {
    return { x: clamp(number($("#vrX")?.value, .30), -3, 3), y: clamp(number($("#vrY")?.value, .08), -3, 3), z: clamp(number($("#vrZ")?.value, -1.05), -5, -.1), width_m: clamp(number($("#vrWidth")?.value, .78), .2, 3), opacity: clamp(number($("#vrOpacity")?.value, .92), .1, 1), controller_grab_enabled: $("#vrGrabToggle")?.getAttribute("aria-checked") === "true" };
  }

  async function saveVrPlacement() {
    const placement = readVrPlacement();
    try { await api("/api/settings", { method: "PUT", body: JSON.stringify({ vr_overlay: placement }) }); state.settings.vr_overlay = { ...(state.settings.vr_overlay || {}), ...placement }; hydrateVrControls(placement); showToast("VR placement and grab setting applied."); }
    catch (_) { showToast("VR placement was not changed because the service is offline.", true); }
  }

  function bindStudio() {
    $$(".profile-card").forEach((button) => button.addEventListener("click", () => { $$(".profile-card").forEach((b) => b.classList.toggle("active", b === button)); renderStudioPreview(true); }));
    $$("#overlayLayout,#overlayTheme,#overlayRows,#overlayHitRows,#overlayScale,#overlayBg,.show-options input").forEach((control) => control.addEventListener("input", () => renderStudioPreview(true)));
    $$("#accentOptions button").forEach((button) => button.addEventListener("click", () => { $$("#accentOptions button").forEach((b) => b.classList.toggle("active", b === button)); renderStudioPreview(true); }));
    $("#copyObsUrl")?.addEventListener("click", async () => { await copyText($("#obsUrl").value); showToast("URL copied. In OBS, press Ctrl+A in the URL field before pasting."); });
    $("#desktopOverlayToggle")?.addEventListener("click", (event) => setOutputEnabled("desktop", event.currentTarget.getAttribute("aria-checked") !== "true", event.currentTarget));
    $("#desktopHeadsetToggle")?.addEventListener("click", (event) => setDesktopHeadsetVisibility(event.currentTarget));
    $("#vrOverlayToggle")?.addEventListener("click", (event) => setOutputEnabled("vr", event.currentTarget.getAttribute("aria-checked") !== "true", event.currentTarget));
    $("#vrGrabToggle")?.addEventListener("click", (event) => toggleSwitch(event.currentTarget));
    $("#saveVrPlacement")?.addEventListener("click", saveVrPlacement);
    $("#resetVrPlacement")?.addEventListener("click", () => { hydrateVrControls({ x:.30, y:.08, z:-1.05, width_m:.78, opacity:.92, controller_grab_enabled:false }); showToast("Default VR placement loaded. Apply to save it."); });
    $("#previewFit")?.addEventListener("click", () => { $("#studioOverlayPreview").style.transform = "scale(.69)"; $("#previewFit").classList.add("active"); $("#previewOneToOne").classList.remove("active"); });
    $("#previewOneToOne")?.addEventListener("click", () => { $("#studioOverlayPreview").style.transform = "scale(1)"; $("#previewOneToOne").classList.add("active"); $("#previewFit").classList.remove("active"); });
  }

  function showToast(message, error = false) {
    const root = $("#toastRegion"); if (!root) return; const toast = document.createElement("div"); toast.className = `toast${error ? " error" : ""}`; toast.innerHTML = `<span>${error ? "!" : "✓"}</span><div>${escapeHtml(message)}</div>`; root.append(toast); setTimeout(() => toast.remove(), 3200);
  }

  async function copyText(value) {
    try { await navigator.clipboard.writeText(value); }
    catch (_) { const input = document.createElement("textarea"); input.value = value; input.style.position = "fixed"; input.style.opacity = "0"; document.body.append(input); input.select(); document.execCommand("copy"); input.remove(); }
  }

  function exportRunsCsv() {
    const rows = [["Run", "Encounter", "Session state", "Metrics scope", "Imported player logs", "Boss fights", "Boss seconds", "Boss DPS", "Boss damage", "Boss outgoing hits", "Largest boss hit", "Boss incoming DPS", "Observed fallback seconds", "Observed fallback DPS", "Observed fallback damage", "Observed fallback outgoing hits", "Largest observed fallback hit", "Observed fallback incoming DPS", "Pre-boss excluded seconds", "Pre-boss outgoing excluded", "Pre-boss incoming excluded", "Total observed seconds"], ...filteredRuns().map((run) => { const boss = isBossScoped(run); return [run.number, run.encounter, run.result, run.metricsScope, run.sourceCount, boss ? run.bossCount : 0, boss ? run.duration : "", boss ? run.dps : "", boss ? run.totalDamage : "", boss ? run.hits : "", boss ? run.biggestHit : "", boss ? run.incomingDps : "", boss ? "" : run.duration, boss ? "" : run.dps, boss ? "" : run.totalDamage, boss ? "" : run.hits, boss ? "" : run.biggestHit, boss ? "" : run.incomingDps, boss ? run.preBossDuration : "", boss ? preBossTotal(run, "outgoing") : "", boss ? preBossTotal(run, "incoming") : "", run.observedDuration]; })];
    const csv = rows.map((row) => row.map((cell) => `"${String(cell).replaceAll('"', '""')}"`).join(",")).join("\n"); const url = URL.createObjectURL(new Blob([csv], { type: "text/csv" })); const link = document.createElement("a"); link.href = url; link.download = `minmaxxer-runs-${new Date().toISOString().slice(0,10)}.csv`; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  async function importLog(file) {
    if (!file) return; const form = new FormData(); form.append("file", file);
    try { showToast(`Importing ${file.name}…`); const result = await api("/api/import", { method: "POST", body: form }, 30000); await refreshRuns(); showToast(result?.message || "Log imported and encounters indexed."); setPage("runs"); }
    catch (_) { showToast("The import service is unavailable. Start MINMAXXER and try again.", true); }
  }

  function openSettings() {
    const dialog = $("#settingsDialog"); if (!dialog) return;
    $("#logPath").value = state.settings.log_path ?? state.settings.logPath ?? state.settings.log_directory ?? "";
    $("#importDays").value = clamp(number(state.settings.import_days, 3), 1, 30);
    setSwitch($("#autoImportToggle"), state.settings.auto_import_recent_logs ?? true); setSwitch($("#launchMinimizedToggle"), state.settings.launch_minimized ?? false); setSwitch($("#minimizeTrayToggle"), state.settings.minimize_to_tray ?? true); dialog.showModal();
  }

  function setSwitch(button, active) { if (!button) return; button.classList.toggle("active", active); button.setAttribute("aria-checked", String(active)); }
  function toggleSwitch(button) { setSwitch(button, button.getAttribute("aria-checked") !== "true"); }

  async function saveSettings() {
    const patch = { log_path: $("#logPath").value.trim(), import_days: clamp(Math.round(number($("#importDays").value, 3)), 1, 30), auto_import_recent_logs: $("#autoImportToggle").getAttribute("aria-checked") === "true", launch_minimized: $("#launchMinimizedToggle").getAttribute("aria-checked") === "true", minimize_to_tray: $("#minimizeTrayToggle").getAttribute("aria-checked") === "true" };
    try { await api("/api/settings", { method: "PUT", body: JSON.stringify(patch) }); state.settings = { ...state.settings, ...patch }; $("#settingsDialog").close(); showToast("Settings saved."); }
    catch (_) { text("#settingsStatus", "Service unavailable — changes not applied."); }
  }

  function bindMainEvents() {
    $$("[data-view]").forEach((button) => button.addEventListener("click", () => setPage(button.dataset.view)));
    $$("[data-jump]").forEach((button) => button.addEventListener("click", () => setPage(button.dataset.jump)));
    $("#menuButton")?.addEventListener("click", () => { const sidebar = $("#sidebar"); sidebar.classList.toggle("open"); $("#menuButton").setAttribute("aria-expanded", String(sidebar.classList.contains("open"))); });
    $("#freezeButton")?.addEventListener("click", (event) => { state.frozen = !state.frozen; event.currentTarget.setAttribute("aria-pressed", String(state.frozen)); $(".button-label", event.currentTarget).textContent = state.frozen ? "Resume" : "Freeze"; showToast(state.frozen ? "Live display frozen; capture continues." : "Live display resumed."); });
    $("#importButton")?.addEventListener("click", () => $("#importInput").click()); $("#importInput")?.addEventListener("change", (event) => { importLog(event.target.files[0]); event.target.value = ""; });
    $$(".chart-range button").forEach((button) => button.addEventListener("click", () => { $$(".chart-range button").forEach((b) => b.classList.toggle("active", b === button)); drawLiveChart(); }));
    $$("[data-party-metric]").forEach((button) => button.addEventListener("click", () => { state.partyMetric = button.dataset.partyMetric; $$("[data-party-metric]").forEach((b) => b.classList.toggle("active", b === button)); renderParty(); }));
    $("#runSearch")?.addEventListener("input", renderRuns); $("#runSort")?.addEventListener("change", renderRuns); $$("[data-run-filter]").forEach((button) => button.addEventListener("click", () => { state.runFilter = button.dataset.runFilter; $$("[data-run-filter]").forEach((b) => b.classList.toggle("active", b === button)); renderRuns(); }));
    $("#exportRunsButton")?.addEventListener("click", exportRunsCsv); $("#compareA")?.addEventListener("change", renderComparison); $("#compareB")?.addEventListener("change", renderComparison);
    $$("[data-analysis-tab]").forEach((button) => button.addEventListener("click", () => { state.analysisTab = button.dataset.analysisTab; $$("[data-analysis-tab]").forEach((b) => b.classList.toggle("active", b === button)); renderAnalysis(); }));
    $("#analysisRunSelect")?.addEventListener("change", renderAnalysis);
    $("#analysisEncounterSelect")?.addEventListener("change", (event) => { const run = selectedAnalysisRun(); if (run) state.analysisEncounterByRun[run.id] = event.target.value; renderAnalysis(); });
    $("#analysisPlayerSelect")?.addEventListener("change", (event) => { const run = selectedAnalysisRun(); if (run) { const context = selectedAnalysisContext(run); state.analysisPlayerByRun[`${run.id}:${context?.id ?? "all-bosses"}`] = event.target.value; } renderAnalysis(); });
    $("#eventRunSelect")?.addEventListener("change", loadEventsForRun); $("#eventSearch")?.addEventListener("input", renderEvents); $("#eventTypeFilter")?.addEventListener("change", renderEvents); $("#strikeOnly")?.addEventListener("change", renderEvents);
    $("#loadMoreEvents")?.addEventListener("click", () => { state.eventLimit += 80; renderEvents(); }); $("#copyEventsButton")?.addEventListener("click", async () => { await copyText(visibleEvents().map((event) => `${event.time}\t${event.type}\t${event.source}\t${event.action}\t${event.target}\t${event.amount}`).join("\n")); showToast("Visible events copied as tab-separated text."); });
    $("#settingsButton")?.addEventListener("click", openSettings); ["#autoImportToggle", "#launchMinimizedToggle", "#minimizeTrayToggle"].forEach((selector) => $(selector)?.addEventListener("click", (event) => toggleSwitch(event.currentTarget))); $("#saveSettings")?.addEventListener("click", saveSettings);
    $("#detectLogButton")?.addEventListener("click", () => { $("#logPath").value = "%USERPROFILE%\\AppData\\LocalLow\\VRChat\\VRChat"; text("#settingsStatus", "Default VRChat location selected."); });
    window.addEventListener("resize", debounce(() => { drawLiveChart(); drawCompareChart(); }, 120));
    document.addEventListener("visibilitychange", () => { if (!document.hidden) renderAll(); });
    document.addEventListener("keydown", (event) => { if (event.ctrlKey || event.metaKey || event.altKey || /INPUT|SELECT|TEXTAREA/.test(document.activeElement?.tagName)) return; const map = { l:"live",r:"runs",c:"compare",a:"analysis",e:"events",o:"overlay" }; if (map[event.key.toLowerCase()]) setPage(map[event.key.toLowerCase()]); });
    window.addEventListener("popstate", () => setPage(new URLSearchParams(location.search).get("view") || "live", false)); bindChartTooltip(); bindStudio();
  }

  function debounce(fn, wait) { let timer; return (...args) => { clearTimeout(timer); timer = setTimeout(() => fn(...args), wait); }; }

  function renderAll() { renderLive(); renderRuns(); populateRunSelects(); renderComparison(); renderAnalysis(); renderEvents(); renderStudioPreview(); }

  function startDemoClock(onTick) {
    if (!state.usingMock) return;
    state.timers.push(setInterval(() => {
      if (!state.live || state.frozen) return;
      state.live.encounter.duration += 1;
      const drift = Math.sin(state.live.encounter.duration * .17) * .006;
      state.live.outgoing.dps *= 1 + drift; state.live.partyHps *= 1 - drift * .7; state.live.incoming.dps *= 1 + Math.sin(state.live.encounter.duration * .11) * .004;
      onTick();
    }, 1000));
  }

  async function bootOverlay() {
    document.body.dataset.overlay = "true"; const options = overlayOptionsFromSearch();
    const root = $("#overlayRoot"); const render = () => renderCombatOverlay(root, state.live, options);
    state.live = makeOverlayWaitingLive(false); state.usingMock = false; state.overlayServiceState = "connecting"; render();
    try { state.live = normalizeLive(await api("/api/live"), makeOverlayWaitingLive(true)); state.apiOnline = true; state.overlayServiceState = "ready"; state.lastLiveAt = Date.now(); }
    catch (_) { state.apiOnline = false; state.overlayServiceState = "lost"; }
    render();
    connectStream(render, { onStreamState: (signal) => {
      if (signal === "error") {
        if (state.overlayServiceState === "lost" || state.overlayStreamLossTimer !== null) return;
        state.overlayStreamLossTimer = setTimeout(() => { state.overlayStreamLossTimer = null; state.overlayServiceState = "lost"; render(); }, 2500);
        return;
      }
      clearTimeout(state.overlayStreamLossTimer); state.overlayStreamLossTimer = null;
      state.overlayServiceState = signal === "message" ? "ready" : "connecting";
      if (signal !== "message") render();
    } });
    state.timers.push(setInterval(render, 1000));
  }

  async function bootApp() {
    await loadInitialData(); applySavedStudioOptions(); hydrateVrControls(); bindMainEvents(); renderAll();
    const initialPage = new URLSearchParams(location.search).get("view") || "live"; setPage(initialPage, false);
    connectStream(() => { if (document.hidden) return; renderLive(); if ($('[data-page="overlay"]').classList.contains("active")) renderStudioPreview(); }, { refreshArchive: true, clearLiveOnError: true });
    startDemoClock(() => { if (document.hidden) return; renderLive(); if ($('[data-page="overlay"]').classList.contains("active")) renderStudioPreview(); });
    const desktop = Boolean(state.settings.desktop_overlay_enabled ?? state.settings.desktopOverlayEnabled ?? state.settings.desktop_overlay?.enabled); const vr = Boolean(state.settings.vr_overlay_enabled ?? state.settings.vrOverlayEnabled ?? state.settings.vr_overlay?.enabled); setSwitch($("#desktopOverlayToggle"), desktop); setSwitch($("#vrOverlayToggle"), vr);
    setSwitch($("#desktopHeadsetToggle"), Boolean(state.settings.desktop_overlay?.show_when_vr_active));
    renderVrStatus();
    state.timers.push(setInterval(() => { if (!document.hidden && $('[data-page="overlay"]')?.classList.contains("active")) refreshVrStatus(); }, 2000));
  }

  const overlayMode = document.body.dataset.forcedOverlay === "true" || location.pathname.replace(/\/+$/, "").endsWith("/overlay");
  if (overlayMode) bootOverlay(); else bootApp();
})();
