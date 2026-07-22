use minmaxxer_core::{analyze_runs, EclipticaParser, EventKind, GameEvent, ParseOutcome};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

#[derive(Serialize)]
struct FileAudit {
    path: PathBuf,
    lines: u64,
    timestamped_lines: u64,
    emitted: u64,
    relevant_unparsed: u64,
    event_kinds: BTreeMap<String, usize>,
    unknown_samples: Vec<String>,
}

#[derive(Serialize)]
struct Audit {
    files: Vec<FileAudit>,
    outgoing_hits: usize,
    outgoing_damage: f64,
    incoming_hits: usize,
    incoming_damage: f64,
    runs: Vec<minmaxxer_core::RunSummary>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if paths.is_empty() {
        return Err("pass one or more VRChat output_log paths".into());
    }

    let mut files = Vec::new();
    let mut events = Vec::<GameEvent>::new();
    for path in paths {
        let mut parser = EclipticaParser::new();
        let mut event_kinds = BTreeMap::new();
        let mut unknown_samples = Vec::new();
        for line in BufReader::new(File::open(&path)?).lines() {
            let line = line?;
            match parser.process_line(&line) {
                ParseOutcome::Event(event) => {
                    *event_kinds
                        .entry(event.kind.as_str().to_owned())
                        .or_insert(0) += 1;
                    events.push(event);
                }
                ParseOutcome::RelevantButUnknown if unknown_samples.len() < 25 => {
                    unknown_samples.push(line);
                }
                _ => {}
            }
        }
        let coverage = parser.coverage();
        files.push(FileAudit {
            path,
            lines: coverage.lines_seen,
            timestamped_lines: coverage.timestamped_lines,
            emitted: coverage.events_emitted,
            relevant_unparsed: coverage.relevant_unparsed,
            event_kinds,
            unknown_samples,
        });
    }

    events.sort_by_key(|event| (event.timestamp, event.sequence));
    let outgoing: Vec<_> = events
        .iter()
        .filter(|event| event.kind == EventKind::DamageDealt)
        .collect();
    let incoming: Vec<_> = events
        .iter()
        .filter(|event| event.kind == EventKind::DamageTaken)
        .collect();
    let audit = Audit {
        files,
        outgoing_hits: outgoing.len(),
        outgoing_damage: outgoing.iter().map(|event| event.amount()).sum(),
        incoming_hits: incoming.len(),
        incoming_damage: incoming.iter().map(|event| event.amount()).sum(),
        runs: analyze_runs(&events),
    };
    println!("{}", serde_json::to_string_pretty(&audit)?);
    Ok(())
}
