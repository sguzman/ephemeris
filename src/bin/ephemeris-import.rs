use std::path::PathBuf;

use anyhow::{Context, anyhow};
use ephemeris::ics::{import_ics_file, import_remote_ics};
use ephemeris::store::TemporalStore;
use ephemeris::taria::import_reconciled_event_set_file;

fn main() -> anyhow::Result<()> {
    let input = std::env::args_os().nth(1).ok_or_else(|| {
        anyhow!(
            "usage: ephemeris-import <calendar.ics|calendar-url|taria-reconciled-event-set.json>"
        )
    })?;

    let store = TemporalStore::open_default().context("failed to open Ephemeris database")?;

    if let Some(remote) = input.to_str()
        && matches!(
            remote,
            value if value.starts_with("http://")
                || value.starts_with("https://")
                || value.starts_with("webcal://")
                || value.starts_with("webcals://")
        )
    {
        let report = import_remote_ics(&store, remote)?;
        if report.not_modified {
            println!(
                "Remote iCalendar not modified (HTTP 304): {}",
                report.source_name
            );
            println!("Events in calendar: {}", report.total_events);
        } else {
            println!("Imported remote iCalendar source: {}", report.source_name);
            println!("Events in calendar: {}", report.total_events);
            println!(
                "Created: {}  Updated: {}  Unchanged: {}",
                report.created, report.updated, report.unchanged
            );
            println!("Prior missing retained: {}", report.retained_missing);
        }
        println!("Source: {}", report.source_external_ref);
        return Ok(());
    }

    let path = PathBuf::from(input);
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);

    if matches!(extension.as_deref(), Some("ics" | "ical")) {
        let report = import_ics_file(&store, &path)?;
        println!("Imported iCalendar source: {}", report.source_name);
        println!("Source: {}", report.source_external_ref);
        println!("Events in calendar: {}", report.total_events);
        println!(
            "Created: {}  Updated: {}  Unchanged: {}",
            report.created, report.updated, report.unchanged
        );
        println!("Prior missing retained: {}", report.retained_missing);
        return Ok(());
    }

    let report = import_reconciled_event_set_file(&store, &path)?;

    println!("Imported Taria projection: {}", report.projection_ref);
    if let Some(reconciled) = report.reconciled_set_ref.as_deref() {
        println!("Reconciled set: {reconciled}");
    }
    println!("Events in artifact: {}", report.total_events);
    println!(
        "Created: {}  Updated: {}  Unchanged: {}",
        report.created, report.updated, report.unchanged
    );
    println!(
        "Imprecise: {}  Unplaced: {}  Blocked/undated: {}  Prior missing retained: {}",
        report.imprecise, report.unplaced, report.blocked_or_undated, report.retained_missing
    );

    Ok(())
}
