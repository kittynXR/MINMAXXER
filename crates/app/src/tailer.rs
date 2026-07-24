use crate::boss_alert::{BossTargetEvent, BossTargetUpdate};
use crate::config::AppConfig;
use crate::storage::Storage;
use anyhow::{Context, Result};
use chrono::Local;
use minmaxxer_core::{
    BossTargetObservation, CombatEngine, EclipticaParser, EngineSnapshot, ParseOutcome,
};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc as tokio_mpsc, watch};

const POLL_INTERVAL: Duration = Duration::from_millis(250);
const DIRECTORY_SCAN_INTERVAL: Duration = Duration::from_secs(2);
const CLOCK_SNAPSHOT_INTERVAL: Duration = Duration::from_secs(1);
const LIVE_ACTIVITY_THRESHOLD: Duration = Duration::from_secs(5 * 60);
const READ_BUFFER_SIZE: usize = 64 * 1024;
type BossTargetSender = tokio_mpsc::UnboundedSender<BossTargetUpdate>;

struct PendingBossTargetUpdates {
    source_file: String,
    baseline_after_offset: Option<u64>,
    baseline: Option<Option<BossTargetEvent>>,
    live: Vec<BossTargetEvent>,
}

impl PendingBossTargetUpdates {
    fn new(path: &Path, baseline_after_offset: Option<u64>) -> Self {
        Self {
            source_file: path.to_string_lossy().into_owned(),
            baseline_after_offset,
            baseline: None,
            live: Vec::new(),
        }
    }

    fn before_line(&mut self, line_end: u64, engine: &CombatEngine) {
        if self
            .baseline_after_offset
            .is_some_and(|offset| line_end > offset)
            && self.baseline.is_none()
        {
            self.capture_baseline(engine);
        }
    }

    fn record(&mut self, line_end: u64, observation: BossTargetObservation) {
        if self
            .baseline_after_offset
            .is_none_or(|offset| line_end > offset)
        {
            self.live.push(target_event(&self.source_file, observation));
        }
    }

    fn finish_replay(&mut self, engine: &CombatEngine) {
        if self.baseline_after_offset.is_some() && self.baseline.is_none() {
            self.capture_baseline(engine);
        }
    }

    fn capture_baseline(&mut self, engine: &CombatEngine) {
        self.baseline = Some(
            engine
                .boss_target_baseline()
                .map(|observation| target_event(&self.source_file, observation)),
        );
    }

    fn publish(self, sender: &BossTargetSender) {
        if let Some(baseline) = self.baseline {
            let _ = sender.send(BossTargetUpdate::Baseline(baseline));
        }
        for event in self.live {
            let _ = sender.send(BossTargetUpdate::Live(event));
        }
    }
}

fn target_event(source_file: &str, observation: BossTargetObservation) -> BossTargetEvent {
    BossTargetEvent {
        source_file: source_file.to_owned(),
        encounter_name: observation.encounter_name,
        encounter_started_at: observation.encounter_started_at,
        target_player: observation.focus.player,
        observed_player: observation.observed_player,
        observed_at: observation.focus.observed_at,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectorSettings {
    pub log_directory: PathBuf,
    pub import_days: u32,
    pub auto_import_recent_logs: bool,
}

impl From<&AppConfig> for CollectorSettings {
    fn from(config: &AppConfig) -> Self {
        Self {
            log_directory: config.log_directory.clone(),
            import_days: config.import_days,
            auto_import_recent_logs: config.auto_import_recent_logs,
        }
    }
}

impl CollectorSettings {
    fn historical_days(&self) -> u32 {
        if self.auto_import_recent_logs {
            self.import_days
        } else {
            0
        }
    }
}

#[derive(Debug)]
pub enum CollectorCommand {
    Import {
        path: PathBuf,
        reply: mpsc::SyncSender<std::result::Result<(), String>>,
    },
    Reconfigure(CollectorSettings),
    Rescan,
    Stop,
}

#[derive(Clone)]
pub struct CollectorHandle {
    commands: mpsc::Sender<CollectorCommand>,
}

impl CollectorHandle {
    pub fn import(&self, path: PathBuf) -> Result<()> {
        let (reply, response) = mpsc::sync_channel(1);
        self.commands
            .send(CollectorCommand::Import { path, reply })
            .context("collector stopped")?;
        response
            .recv_timeout(Duration::from_secs(30))
            .context("log import timed out")?
            .map_err(anyhow::Error::msg)
    }

    pub fn rescan(&self) -> Result<()> {
        self.commands
            .send(CollectorCommand::Rescan)
            .context("collector stopped")
    }

    pub fn reconfigure(&self, settings: CollectorSettings) -> Result<()> {
        self.commands
            .send(CollectorCommand::Reconfigure(settings))
            .context("collector stopped")
    }

    pub fn stop(&self) {
        let _ = self.commands.send(CollectorCommand::Stop);
    }
}

pub fn spawn_collector(
    settings: CollectorSettings,
    storage: Arc<Storage>,
    snapshots: watch::Sender<EngineSnapshot>,
    boss_target_events: BossTargetSender,
) -> CollectorHandle {
    let (commands, receiver) = mpsc::channel();
    thread::Builder::new()
        .name("minmaxxer-log-collector".to_owned())
        .spawn(move || {
            if let Err(error) =
                collector_main(settings, storage, snapshots, boss_target_events, receiver)
            {
                tracing::error!(%error, "log collector stopped unexpectedly");
            }
        })
        .expect("failed spawning log collector");
    CollectorHandle { commands }
}

fn collector_main(
    mut settings: CollectorSettings,
    storage: Arc<Storage>,
    snapshots: watch::Sender<EngineSnapshot>,
    boss_target_events: BossTargetSender,
    commands: mpsc::Receiver<CollectorCommand>,
) -> Result<()> {
    let mut engine = CombatEngine::new();
    let mut active: Option<ActiveTail> = None;
    if let Err(error) = reload_collection(
        &settings,
        &storage,
        &mut engine,
        &mut active,
        Some(&boss_target_events),
    ) {
        tracing::warn!(%error, "initial VRChat log scan failed; collector will retry");
    }
    let _ = snapshots.send(engine.snapshot_at(Some(Local::now().naive_local())));
    let mut last_directory_scan = Instant::now();
    let mut last_clock_snapshot = Instant::now();

    loop {
        let mut changed = false;
        match commands.recv_timeout(POLL_INTERVAL) {
            Ok(CollectorCommand::Import { path, reply }) => {
                let result = import_path(&path, &storage).map_err(|error| error.to_string());
                if let Err(error) = &result {
                    tracing::warn!(%error, path = %path.display(), "manual log import failed");
                }
                let _ = reply.send(result);
            }
            Ok(CollectorCommand::Reconfigure(next)) => {
                if next != settings {
                    settings = next;
                    if let Err(error) = reload_collection(
                        &settings,
                        &storage,
                        &mut engine,
                        &mut active,
                        Some(&boss_target_events),
                    ) {
                        tracing::warn!(%error, "VRChat log reconfiguration failed; collector will retry");
                    }
                    last_directory_scan = Instant::now();
                    changed = true;
                }
            }
            Ok(CollectorCommand::Rescan) => {
                if let Err(error) = reload_collection(
                    &settings,
                    &storage,
                    &mut engine,
                    &mut active,
                    Some(&boss_target_events),
                ) {
                    tracing::warn!(%error, "VRChat log rescan failed");
                } else {
                    last_directory_scan = Instant::now();
                    changed = true;
                }
            }
            Ok(CollectorCommand::Stop) => return Ok(()),
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }

        if last_directory_scan.elapsed() >= DIRECTORY_SCAN_INTERVAL {
            last_directory_scan = Instant::now();
            let newest = match newest_log(&settings.log_directory) {
                Ok(newest) => newest,
                Err(error) => {
                    tracing::warn!(%error, "VRChat log directory scan failed; collector will retry");
                    continue;
                }
            };
            let active_path = active.as_ref().map(|tail| tail.path.as_path());
            if newest.as_deref() != active_path {
                if let Some(old) = active.take() {
                    if let Err(error) = old.finish(&storage) {
                        tracing::warn!(%error, "could not finalize rotated VRChat log");
                    }
                }
                engine = CombatEngine::new();
                if let Some(path) = newest {
                    match open_newest(
                        path,
                        settings.historical_days(),
                        &storage,
                        &mut engine,
                        Some(&boss_target_events),
                        None,
                    ) {
                        Ok(tail) => active = Some(tail),
                        Err(error) => {
                            tracing::warn!(%error, "could not open newest VRChat log; collector will retry");
                        }
                    }
                } else {
                    let _ = boss_target_events.send(BossTargetUpdate::Baseline(None));
                }
                changed = true;
            }
        }

        let drain_result = active
            .as_mut()
            .map(|tail| tail.drain(&storage, &mut engine, Some(&boss_target_events), None));
        match drain_result {
            Some(Ok(tail_changed)) => {
                changed |= tail_changed;
                if let Some(tail) = active.as_ref() {
                    engine.set_coverage(tail.parser.coverage().clone());
                }
            }
            Some(Err(error)) => {
                tracing::warn!(%error, "VRChat log read failed; rebuilding the active source on retry");
                if let Some(path) = active.as_ref().map(|tail| tail.path.clone()) {
                    if let Err(reset_error) = storage.reset_source(&path) {
                        tracing::warn!(%reset_error, path = %path.display(), "could not reset failed log source");
                    }
                }
                active = None;
                engine = CombatEngine::new();
                last_directory_scan = Instant::now()
                    .checked_sub(DIRECTORY_SCAN_INTERVAL)
                    .unwrap_or_else(Instant::now);
                changed = true;
            }
            None => {}
        }
        let snapshot_clock = Local::now().naive_local();
        let clock_due = engine.requires_clock_tick_at(Some(snapshot_clock))
            && last_clock_snapshot.elapsed() >= CLOCK_SNAPSHOT_INTERVAL;
        if changed || clock_due {
            let _ = snapshots.send(engine.snapshot_at(Some(snapshot_clock)));
            last_clock_snapshot = Instant::now();
        }
    }
}

fn reload_collection(
    settings: &CollectorSettings,
    storage: &Storage,
    engine: &mut CombatEngine,
    active: &mut Option<ActiveTail>,
    boss_target_events: Option<&BossTargetSender>,
) -> Result<()> {
    // Dropping the previous handle avoids treating a still-open partial line as a completed line
    // during a user-requested rescan or directory change.
    *active = None;
    *engine = CombatEngine::new();

    let mut logs = discover_logs(&settings.log_directory)?;
    if logs.is_empty() {
        if let Some(sender) = boss_target_events {
            let _ = sender.send(BossTargetUpdate::Baseline(None));
        }
        return Ok(());
    }
    let newest = logs.last().cloned();
    let historical_days = settings.historical_days();

    for path in logs.drain(..) {
        if Some(&path) == newest.as_ref() {
            continue;
        }
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "could not inspect log");
                continue;
            }
        };
        if !within_history_window(&metadata, historical_days, SystemTime::now()) {
            continue;
        }
        import_complete_file_if_needed(&path, &metadata, storage)?;
    }

    if let Some(path) = newest {
        *active = Some(open_newest(
            path,
            historical_days,
            storage,
            engine,
            boss_target_events,
            None,
        )?);
    } else if let Some(sender) = boss_target_events {
        let _ = sender.send(BossTargetUpdate::Baseline(None));
    }
    Ok(())
}

fn open_newest(
    path: PathBuf,
    historical_days: u32,
    storage: &Storage,
    engine: &mut CombatEngine,
    boss_target_events: Option<&BossTargetSender>,
    alert_after_offset: Option<u64>,
) -> Result<ActiveTail> {
    let metadata = std::fs::metadata(&path)?;
    if is_recently_active(&metadata, SystemTime::now()) {
        // A newly discovered file may already contain minutes of history after a delayed scan or
        // system resume. Baseline its present EOF and emit only lines appended concurrently with
        // or after the replay; sounding old target transfers would be actively misleading.
        let alert_after_offset =
            boss_target_events.map(|_| alert_after_offset.unwrap_or(metadata.len()));
        return ActiveTail::open_and_replay(
            path,
            storage,
            engine,
            boss_target_events,
            alert_after_offset,
        );
    }

    // A completed log may still belong in history, but it must not seed a live HUD. With history
    // import disabled, opening dormantly also means no old events are indexed until the file grows.
    if within_history_window(&metadata, historical_days, SystemTime::now()) {
        import_complete_file_if_needed(&path, &metadata, storage)?;
    }
    let tail = ActiveTail::open_dormant(path)?;
    if let Some(sender) = boss_target_events {
        let _ = sender.send(BossTargetUpdate::Baseline(None));
    }
    Ok(tail)
}

fn import_complete_file_if_needed(
    path: &Path,
    metadata: &std::fs::Metadata,
    storage: &Storage,
) -> Result<()> {
    if let Some(state) = storage.ingest_state(path)? {
        let current = state.complete
            && state.file_size == metadata.len()
            && state.modified_millis == modified_millis(metadata)
            && state.parser_version == env!("CARGO_PKG_VERSION");
        if current {
            return Ok(());
        }
        // A complete-file import is an atomic logical replay. Clear partial output or output from
        // an older parser version before rebuilding it, otherwise reused offsets stay stale.
        storage.reset_source(path)?;
    }
    import_complete_file(path, storage)
}

fn within_history_window(
    metadata: &std::fs::Metadata,
    historical_days: u32,
    now: SystemTime,
) -> bool {
    if historical_days == 0 {
        return false;
    }
    let cutoff = now
        .checked_sub(Duration::from_secs(historical_days as u64 * 86_400))
        .unwrap_or(UNIX_EPOCH);
    metadata.modified().unwrap_or(UNIX_EPOCH) >= cutoff
}

fn is_recently_active(metadata: &std::fs::Metadata, now: SystemTime) -> bool {
    metadata.modified().is_ok_and(|modified| {
        now.duration_since(modified)
            .map(|age| age <= LIVE_ACTIVITY_THRESHOLD)
            .unwrap_or(true)
    })
}

fn import_path(path: &Path, storage: &Storage) -> Result<()> {
    if path.is_dir() {
        for log in discover_logs(path)? {
            let metadata = std::fs::metadata(&log)?;
            import_complete_file_if_needed(&log, &metadata, storage)?;
        }
    } else {
        let metadata = std::fs::metadata(path)?;
        import_complete_file_if_needed(path, &metadata, storage)?;
    }
    Ok(())
}

fn import_complete_file(path: &Path, storage: &Storage) -> Result<()> {
    let mut file = open_shared(path)?;
    let metadata = file.metadata()?;
    let modified_millis = modified_millis(&metadata);
    let mut parser = EclipticaParser::new();
    let mut buffer = Vec::new();
    let mut cursor = 0_u64;
    let mut chunk = vec![0_u8; READ_BUFFER_SIZE];
    let mut pending_events = Vec::new();

    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        let buffer_start = cursor.saturating_sub(buffer.len() as u64);
        cursor += read as u64;
        buffer.extend_from_slice(&chunk[..read]);
        let consumed = consume_complete_lines(
            path,
            buffer_start,
            &buffer,
            &mut parser,
            storage,
            None,
            None,
            &mut pending_events,
        )?;
        if consumed > 0 {
            buffer.drain(..consumed);
        }
        if pending_events.len() >= 128 {
            storage.insert_events(
                path,
                &pending_events,
                cursor,
                metadata.len(),
                modified_millis,
                false,
            )?;
            pending_events.clear();
        }
    }
    // A closed/imported log's final line is still useful even if it lacks a newline.
    if !buffer.is_empty() {
        let start = cursor.saturating_sub(buffer.len() as u64);
        consume_one_line(
            path,
            start,
            &buffer,
            &mut parser,
            storage,
            None,
            None,
            &mut pending_events,
        )?;
    }
    storage.insert_events(
        path,
        &pending_events,
        cursor,
        metadata.len(),
        modified_millis,
        true,
    )?;
    tracing::info!(
        path = %path.display(),
        events = parser.coverage().events_emitted,
        unknown = parser.coverage().relevant_unparsed,
        "imported VRChat log"
    );
    Ok(())
}

struct ActiveTail {
    path: PathBuf,
    file: File,
    parser: EclipticaParser,
    cursor: u64,
    partial: Vec<u8>,
    last_size: u64,
    modified_millis: i64,
    live: bool,
}

impl ActiveTail {
    fn open_and_replay(
        path: PathBuf,
        storage: &Storage,
        engine: &mut CombatEngine,
        boss_target_events: Option<&BossTargetSender>,
        alert_after_offset: Option<u64>,
    ) -> Result<Self> {
        let file = open_shared(&path)?;
        let metadata = file.metadata()?;
        if let Some(state) = storage.ingest_state(&path)? {
            let source_was_replaced = metadata.len() < state.file_size
                || (metadata.len() == state.file_size
                    && modified_millis(&metadata) != state.modified_millis);
            if source_was_replaced || state.parser_version != env!("CARGO_PKG_VERSION") {
                storage.reset_source(&path)?;
            }
        }
        let mut tail = Self {
            path: path.clone(),
            file,
            parser: EclipticaParser::new(),
            cursor: 0,
            partial: Vec::new(),
            last_size: metadata.len(),
            modified_millis: modified_millis(&metadata),
            live: true,
        };
        engine.set_source_file(Some(path.to_string_lossy().into_owned()));
        tail.drain(storage, engine, boss_target_events, alert_after_offset)?;
        engine.set_coverage(tail.parser.coverage().clone());
        Ok(tail)
    }

    fn open_dormant(path: PathBuf) -> Result<Self> {
        let mut file = open_shared(&path)?;
        let metadata = file.metadata()?;
        let cursor = file.seek(SeekFrom::End(0))?;
        Ok(Self {
            path,
            file,
            parser: EclipticaParser::new(),
            cursor,
            partial: Vec::new(),
            last_size: metadata.len(),
            modified_millis: modified_millis(&metadata),
            live: false,
        })
    }

    fn drain(
        &mut self,
        storage: &Storage,
        engine: &mut CombatEngine,
        boss_target_events: Option<&BossTargetSender>,
        alert_after_offset: Option<u64>,
    ) -> Result<bool> {
        let metadata = self.file.metadata()?;
        let current_modified_millis = modified_millis(&metadata);
        if !self.live {
            if metadata.len() == self.last_size && current_modified_millis == self.modified_millis {
                return Ok(false);
            }
            if metadata.len() <= self.last_size {
                storage.reset_source(&self.path)?;
            }
            // A dormant path may have been replaced and regrown beyond the former EOF between
            // polls. Treat every byte present at wake-up as a silent baseline; only a concurrent
            // append beyond this captured length is safe to classify as live.
            let baseline_offset = metadata.len();
            *engine = CombatEngine::new();
            *self = Self::open_and_replay(
                self.path.clone(),
                storage,
                engine,
                boss_target_events,
                boss_target_events.map(|_| baseline_offset),
            )?;
            return Ok(true);
        }
        if metadata.len() < self.cursor {
            tracing::warn!(path = %self.path.display(), "active log was truncated; replaying");
            storage.reset_source(&self.path)?;
            *engine = CombatEngine::new();
            *self = Self::open_and_replay(
                self.path.clone(),
                storage,
                engine,
                boss_target_events,
                boss_target_events.map(|_| metadata.len()),
            )?;
            return Ok(true);
        }
        self.last_size = metadata.len();
        self.modified_millis = current_modified_millis;
        let mut bytes = Vec::new();
        self.file.read_to_end(&mut bytes)?;
        if bytes.is_empty() {
            if !is_recently_active(&metadata, SystemTime::now()) {
                self.live = false;
                self.partial.clear();
                *engine = CombatEngine::new();
                if let Some(sender) = boss_target_events {
                    let _ = sender.send(BossTargetUpdate::Baseline(None));
                }
                return Ok(true);
            }
            if alert_after_offset.is_some() {
                if let Some(sender) = boss_target_events {
                    let mut pending = PendingBossTargetUpdates::new(&self.path, alert_after_offset);
                    pending.finish_replay(engine);
                    pending.publish(sender);
                }
            }
            return Ok(false);
        }
        let buffer_start = self.cursor.saturating_sub(self.partial.len() as u64);
        self.cursor += bytes.len() as u64;
        self.partial.extend_from_slice(&bytes);
        let mut events = Vec::new();
        let mut pending_boss_targets = boss_target_events
            .map(|_| PendingBossTargetUpdates::new(&self.path, alert_after_offset));
        let consumed = consume_complete_lines(
            &self.path,
            buffer_start,
            &self.partial,
            &mut self.parser,
            storage,
            Some(engine),
            pending_boss_targets.as_mut(),
            &mut events,
        )?;
        if let Some(pending) = pending_boss_targets.as_mut() {
            pending.finish_replay(engine);
        }
        if consumed > 0 {
            self.partial.drain(..consumed);
        }
        storage.insert_events(
            &self.path,
            &events,
            self.cursor,
            self.last_size,
            self.modified_millis,
            false,
        )?;
        if let (Some(sender), Some(pending)) = (boss_target_events, pending_boss_targets) {
            pending.publish(sender);
        }
        // Ignored VRChat noise can be frequent; only semantic parser events warrant a full HUD
        // snapshot broadcast.
        Ok(!events.is_empty())
    }

    fn finish(mut self, storage: &Storage) -> Result<()> {
        if !self.live {
            return Ok(());
        }
        let mut sink = CombatEngine::new();
        let _ = self.drain(storage, &mut sink, None, None)?;
        if !self.partial.is_empty() {
            let start = self.cursor.saturating_sub(self.partial.len() as u64);
            let mut events = Vec::new();
            consume_one_line(
                &self.path,
                start,
                &self.partial,
                &mut self.parser,
                storage,
                None,
                None,
                &mut events,
            )?;
            storage.insert_events(
                &self.path,
                &events,
                self.cursor,
                self.last_size,
                self.modified_millis,
                true,
            )?;
        } else {
            storage.insert_events(
                &self.path,
                &[],
                self.cursor,
                self.last_size,
                self.modified_millis,
                true,
            )?;
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn consume_complete_lines(
    path: &Path,
    buffer_start: u64,
    buffer: &[u8],
    parser: &mut EclipticaParser,
    storage: &Storage,
    mut engine: Option<&mut CombatEngine>,
    mut pending_boss_targets: Option<&mut PendingBossTargetUpdates>,
    events: &mut Vec<(u64, minmaxxer_core::GameEvent)>,
) -> Result<usize> {
    let mut consumed = 0;
    for (index, byte) in buffer.iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }
        let line = &buffer[consumed..index];
        let offset = buffer_start + consumed as u64;
        consume_one_line(
            path,
            offset,
            line,
            parser,
            storage,
            engine.as_deref_mut(),
            pending_boss_targets.as_deref_mut(),
            events,
        )?;
        consumed = index + 1;
    }
    Ok(consumed)
}

#[allow(clippy::too_many_arguments)]
fn consume_one_line(
    path: &Path,
    offset: u64,
    bytes: &[u8],
    parser: &mut EclipticaParser,
    storage: &Storage,
    engine: Option<&mut CombatEngine>,
    mut pending_boss_targets: Option<&mut PendingBossTargetUpdates>,
    events: &mut Vec<(u64, minmaxxer_core::GameEvent)>,
) -> Result<()> {
    let line = String::from_utf8_lossy(bytes);
    let line = line.trim_end_matches('\r');
    match parser.process_line(line) {
        ParseOutcome::Event(event) => {
            if let Some(engine) = engine {
                let line_end = offset.saturating_add(bytes.len() as u64);
                if let Some(pending) = pending_boss_targets.as_deref_mut() {
                    pending.before_line(line_end, engine);
                }
                let observation = engine.ingest_with_boss_target_observation(event.clone());
                if let (Some(observation), Some(pending)) = (observation, pending_boss_targets) {
                    pending.record(line_end, observation);
                }
            }
            events.push((offset, event));
        }
        ParseOutcome::RelevantButUnknown => {
            storage.insert_unknown_line(path, offset, line)?;
        }
        ParseOutcome::Ignored => {}
    }
    Ok(())
}

fn discover_logs(directory: &Path) -> Result<Vec<PathBuf>> {
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut logs = Vec::new();
    for entry in std::fs::read_dir(directory)
        .with_context(|| format!("failed reading log directory {}", directory.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let is_log = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("output_log_") && name.ends_with(".txt"));
        if is_log {
            logs.push(path);
        }
    }
    logs.sort();
    Ok(logs)
}

fn newest_log(directory: &Path) -> Result<Option<PathBuf>> {
    Ok(discover_logs(directory)?.pop())
}

fn open_shared(path: &Path) -> Result<File> {
    OpenOptions::new()
        .read(true)
        .open(path)
        .with_context(|| format!("failed opening VRChat log {}", path.display()))
}

fn modified_millis(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_storage(label: &str) -> (Storage, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "minmaxxer-{label}-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&path);
        (Storage::open(&path).unwrap(), path)
    }

    fn remove_storage_files(path: &Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[test]
    fn disabling_auto_import_means_zero_historical_days() {
        let settings = CollectorSettings {
            log_directory: PathBuf::from("logs"),
            import_days: 30,
            auto_import_recent_logs: false,
        };
        assert_eq!(settings.historical_days(), 0);
    }

    #[test]
    fn file_activity_threshold_distinguishes_live_and_stale_sources() {
        let path = std::env::temp_dir().join(format!(
            "minmaxxer-activity-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"test").unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        let modified = metadata.modified().unwrap();
        assert!(is_recently_active(
            &metadata,
            modified + LIVE_ACTIVITY_THRESHOLD / 2
        ));
        assert!(!is_recently_active(
            &metadata,
            modified + LIVE_ACTIVITY_THRESHOLD + Duration::from_secs(1)
        ));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn reloading_an_empty_log_directory_clears_the_boss_target_baseline() {
        let directory = std::env::temp_dir().join(format!(
            "minmaxxer-empty-logs-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();

        let (storage, database_path) = temporary_storage("boss-alert-empty-directory");
        let settings = CollectorSettings {
            log_directory: directory.clone(),
            import_days: 30,
            auto_import_recent_logs: true,
        };
        let mut engine = CombatEngine::new();
        let mut active: Option<ActiveTail> = None;
        let (sender, mut receiver) = tokio_mpsc::unbounded_channel();

        reload_collection(&settings, &storage, &mut engine, &mut active, Some(&sender)).unwrap();

        assert!(matches!(
            receiver.try_recv(),
            Ok(BossTargetUpdate::Baseline(None))
        ));
        assert!(receiver.try_recv().is_err());
        assert!(active.is_none());

        drop(storage);
        remove_storage_files(&database_path);
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn replay_baselines_silently_and_rapid_live_focus_edges_are_not_coalesced() {
        let (storage, database_path) = temporary_storage("boss-alert-edges");
        let source = Path::new("output_log_live.txt");
        let mut parser = EclipticaParser::new();
        let mut engine = CombatEngine::new();
        let mut parsed_events = Vec::new();
        let replay = concat!(
            "2026.07.23 20:00:00 Debug      -  [Behaviour] Entering Room: Ecliptica - Demo Playtest\n",
            "2026.07.23 20:00:01 Debug      -  [Behaviour] Initialized PlayerAPI \"Local Player\" is local\n",
            "2026.07.23 20:00:02 Debug      -  ECLIPTICA - now fighting boss: Astral Sovereign on phase: 0.1\n",
            "2026.07.23 20:00:03 Debug      -  ownership of Astral Sovereign transferred to Local Player\n",
            "2026.07.23 20:00:03 Debug      -  ownership of Astral Sovereign transferred to Other Player\n",
        );
        let (sender, mut receiver) = tokio_mpsc::unbounded_channel();
        let mut replay_updates = PendingBossTargetUpdates::new(source, Some(replay.len() as u64));

        consume_complete_lines(
            source,
            0,
            replay.as_bytes(),
            &mut parser,
            &storage,
            Some(&mut engine),
            Some(&mut replay_updates),
            &mut parsed_events,
        )
        .unwrap();
        replay_updates.finish_replay(&engine);
        assert!(
            receiver.try_recv().is_err(),
            "parser output must remain buffered until the storage batch succeeds"
        );
        replay_updates.publish(&sender);
        let BossTargetUpdate::Baseline(Some(baseline)) = receiver
            .try_recv()
            .expect("replay should synchronize silently")
        else {
            panic!("expected a retained-target baseline");
        };
        assert_eq!(baseline.target_player, "Other Player");

        let live = concat!(
            "2026.07.23 20:00:04 Debug      -  ownership of Astral-Sovereign transferred to Local Player\n",
            "2026.07.23 20:00:04 Debug      -  ownership of Astral Sovereign transferred to Other Player\n",
        );
        let mut live_updates = PendingBossTargetUpdates::new(source, None);
        consume_complete_lines(
            source,
            replay.len() as u64,
            live.as_bytes(),
            &mut parser,
            &storage,
            Some(&mut engine),
            Some(&mut live_updates),
            &mut parsed_events,
        )
        .unwrap();
        assert!(
            receiver.try_recv().is_err(),
            "live edges must remain buffered until the storage batch succeeds"
        );
        live_updates.publish(&sender);

        let mut targets = Vec::new();
        while let Ok(update) = receiver.try_recv() {
            let BossTargetUpdate::Live(event) = update else {
                panic!("normal append should emit live target events");
            };
            targets.push((event.target_player, event.encounter_name));
        }
        assert_eq!(
            targets,
            [
                ("Local Player".to_owned(), "Astral Sovereign".to_owned()),
                ("Other Player".to_owned(), "Astral Sovereign".to_owned())
            ]
        );

        drop(storage);
        remove_storage_files(&database_path);
    }

    #[test]
    fn late_local_identity_rechecks_the_current_fresh_boss_target() {
        let (storage, database_path) = temporary_storage("boss-alert-identity");
        let source = Path::new("output_log_live.txt");
        let mut parser = EclipticaParser::new();
        let mut engine = CombatEngine::new();
        let mut parsed_events = Vec::new();
        let replay = concat!(
            "2026.07.23 20:00:00 Debug      -  [Behaviour] Entering Room: Ecliptica - Demo Playtest\n",
            "2026.07.23 20:00:01 Debug      -  ECLIPTICA - now fighting boss: Astral Sovereign on phase: 0.1\n",
            "2026.07.23 20:00:02 Debug      -  ownership of Astral Sovereign transferred to Late Local\n",
        );
        consume_complete_lines(
            source,
            0,
            replay.as_bytes(),
            &mut parser,
            &storage,
            Some(&mut engine),
            None,
            &mut parsed_events,
        )
        .unwrap();

        let identity =
            "2026.07.23 20:00:03 Debug      -  [Behaviour] Initialized PlayerAPI \"Late Local\" is local\n";
        let (sender, mut receiver) = tokio_mpsc::unbounded_channel();
        let mut live_updates = PendingBossTargetUpdates::new(source, None);
        consume_complete_lines(
            source,
            replay.len() as u64,
            identity.as_bytes(),
            &mut parser,
            &storage,
            Some(&mut engine),
            Some(&mut live_updates),
            &mut parsed_events,
        )
        .unwrap();
        live_updates.publish(&sender);

        let BossTargetUpdate::Live(observation) =
            receiver.try_recv().expect("identity should refresh focus")
        else {
            panic!("identity refresh should be a live target update");
        };
        assert_eq!(observation.target_player, "Late Local");
        assert_eq!(observation.observed_player.as_deref(), Some("Late Local"));
        assert!(receiver.try_recv().is_err());

        let stale_identity =
            "2026.07.23 20:00:48 Debug      -  [Behaviour] Initialized PlayerAPI \"Late Local\" is local\n";
        let mut stale_updates = PendingBossTargetUpdates::new(source, None);
        consume_complete_lines(
            source,
            (replay.len() + identity.len()) as u64,
            stale_identity.as_bytes(),
            &mut parser,
            &storage,
            Some(&mut engine),
            Some(&mut stale_updates),
            &mut parsed_events,
        )
        .unwrap();
        stale_updates.publish(&sender);
        assert!(
            receiver.try_recv().is_err(),
            "an aging retained target must not become an audible edge"
        );

        drop(storage);
        remove_storage_files(&database_path);
    }

    #[test]
    fn replay_offset_emits_only_content_appended_to_a_dormant_log() {
        let (storage, database_path) = temporary_storage("boss-alert-offset");
        let source = Path::new("output_log_woken.txt");
        let mut parser = EclipticaParser::new();
        let mut engine = CombatEngine::new();
        let mut parsed_events = Vec::new();
        let old_content = concat!(
            "2026.07.23 20:00:00 Debug      -  [Behaviour] Entering Room: Ecliptica - Demo Playtest\n",
            "2026.07.23 20:00:01 Debug      -  [Behaviour] Initialized PlayerAPI \"Local Player\" is local\n",
            "2026.07.23 20:00:02 Debug      -  ECLIPTICA - now fighting boss: Astral Sovereign on phase: 0.1\n",
            "2026.07.23 20:00:03 Debug      -  ownership of Astral Sovereign transferred to Local Player\n",
            "2026.07.23 20:00:03 Debug      -  ownership of Astral Sovereign transferred to Other Player\n",
        );
        let appended =
            "2026.07.23 20:00:04 Debug      -  ownership of Astral Sovereign transferred to Local Player\n";
        let replayed = format!("{old_content}{appended}");
        let (sender, mut receiver) = tokio_mpsc::unbounded_channel();
        let mut pending = PendingBossTargetUpdates::new(source, Some(old_content.len() as u64));

        consume_complete_lines(
            source,
            0,
            replayed.as_bytes(),
            &mut parser,
            &storage,
            Some(&mut engine),
            Some(&mut pending),
            &mut parsed_events,
        )
        .unwrap();
        pending.finish_replay(&engine);
        pending.publish(&sender);

        let BossTargetUpdate::Baseline(Some(baseline)) = receiver
            .try_recv()
            .expect("old content should establish a baseline")
        else {
            panic!("expected a baseline before appended events");
        };
        assert_eq!(baseline.target_player, "Other Player");
        let BossTargetUpdate::Live(observation) = receiver
            .try_recv()
            .expect("the appended target transfer should be live")
        else {
            panic!("expected the appended transfer to be live");
        };
        assert_eq!(observation.target_player, "Local Player");
        assert!(receiver.try_recv().is_err());

        drop(storage);
        remove_storage_files(&database_path);
    }
}
