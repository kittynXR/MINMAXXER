use crate::config::AppConfig;
use crate::storage::Storage;
use anyhow::{Context, Result};
use chrono::Local;
use minmaxxer_core::{CombatEngine, EclipticaParser, EngineSnapshot, ParseOutcome};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

const POLL_INTERVAL: Duration = Duration::from_millis(250);
const DIRECTORY_SCAN_INTERVAL: Duration = Duration::from_secs(2);
const CLOCK_SNAPSHOT_INTERVAL: Duration = Duration::from_secs(1);
const LIVE_ACTIVITY_THRESHOLD: Duration = Duration::from_secs(5 * 60);
const READ_BUFFER_SIZE: usize = 64 * 1024;

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
) -> CollectorHandle {
    let (commands, receiver) = mpsc::channel();
    thread::Builder::new()
        .name("minmaxxer-log-collector".to_owned())
        .spawn(move || {
            if let Err(error) = collector_main(settings, storage, snapshots, receiver) {
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
    commands: mpsc::Receiver<CollectorCommand>,
) -> Result<()> {
    let mut engine = CombatEngine::new();
    let mut active: Option<ActiveTail> = None;
    if let Err(error) = reload_collection(&settings, &storage, &mut engine, &mut active) {
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
                    if let Err(error) =
                        reload_collection(&settings, &storage, &mut engine, &mut active)
                    {
                        tracing::warn!(%error, "VRChat log reconfiguration failed; collector will retry");
                    }
                    last_directory_scan = Instant::now();
                    changed = true;
                }
            }
            Ok(CollectorCommand::Rescan) => {
                if let Err(error) = reload_collection(&settings, &storage, &mut engine, &mut active)
                {
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
                    match open_newest(path, settings.historical_days(), &storage, &mut engine) {
                        Ok(tail) => active = Some(tail),
                        Err(error) => {
                            tracing::warn!(%error, "could not open newest VRChat log; collector will retry");
                        }
                    }
                }
                changed = true;
            }
        }

        let drain_result = active
            .as_mut()
            .map(|tail| tail.drain(&storage, &mut engine));
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
) -> Result<()> {
    // Dropping the previous handle avoids treating a still-open partial line as a completed line
    // during a user-requested rescan or directory change.
    *active = None;
    *engine = CombatEngine::new();

    let mut logs = discover_logs(&settings.log_directory)?;
    if logs.is_empty() {
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
        *active = Some(open_newest(path, historical_days, storage, engine)?);
    }
    Ok(())
}

fn open_newest(
    path: PathBuf,
    historical_days: u32,
    storage: &Storage,
    engine: &mut CombatEngine,
) -> Result<ActiveTail> {
    let metadata = std::fs::metadata(&path)?;
    if is_recently_active(&metadata, SystemTime::now()) {
        return ActiveTail::open_and_replay(path, storage, engine);
    }

    // A completed log may still belong in history, but it must not seed a live HUD. With history
    // import disabled, opening dormantly also means no old events are indexed until the file grows.
    if within_history_window(&metadata, historical_days, SystemTime::now()) {
        import_complete_file_if_needed(&path, &metadata, storage)?;
    }
    ActiveTail::open_dormant(path)
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
        tail.drain(storage, engine)?;
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

    fn drain(&mut self, storage: &Storage, engine: &mut CombatEngine) -> Result<bool> {
        let metadata = self.file.metadata()?;
        let current_modified_millis = modified_millis(&metadata);
        if !self.live {
            if metadata.len() == self.last_size && current_modified_millis == self.modified_millis {
                return Ok(false);
            }
            if metadata.len() <= self.last_size {
                storage.reset_source(&self.path)?;
            }
            *engine = CombatEngine::new();
            *self = Self::open_and_replay(self.path.clone(), storage, engine)?;
            return Ok(true);
        }
        if metadata.len() < self.cursor {
            tracing::warn!(path = %self.path.display(), "active log was truncated; replaying");
            storage.reset_source(&self.path)?;
            *engine = CombatEngine::new();
            *self = Self::open_and_replay(self.path.clone(), storage, engine)?;
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
                return Ok(true);
            }
            return Ok(false);
        }
        let buffer_start = self.cursor.saturating_sub(self.partial.len() as u64);
        self.cursor += bytes.len() as u64;
        self.partial.extend_from_slice(&bytes);
        let mut events = Vec::new();
        let consumed = consume_complete_lines(
            &self.path,
            buffer_start,
            &self.partial,
            &mut self.parser,
            storage,
            Some(engine),
            &mut events,
        )?;
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
        // Ignored VRChat noise can be frequent; only semantic parser events warrant a full HUD
        // snapshot broadcast.
        Ok(!events.is_empty())
    }

    fn finish(mut self, storage: &Storage) -> Result<()> {
        if !self.live {
            return Ok(());
        }
        let mut sink = CombatEngine::new();
        let _ = self.drain(storage, &mut sink)?;
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
    events: &mut Vec<(u64, minmaxxer_core::GameEvent)>,
) -> Result<()> {
    let line = String::from_utf8_lossy(bytes);
    let line = line.trim_end_matches('\r');
    match parser.process_line(line) {
        ParseOutcome::Event(event) => {
            if let Some(engine) = engine {
                engine.ingest(event.clone());
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
}
