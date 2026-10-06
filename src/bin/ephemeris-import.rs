use std::path::PathBuf;

use anyhow::{Context, anyhow};
use ephemeris::ics::import_ics_file;
use ephemeris::store::TemporalStore;
use ephemeris::taria::import_reconciled_event_set_file;

fn main() -> anyhow::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| {
            anyhow!(
                "usage: ephemeris-import <calendar.ics|taria-reconciled-event-set.json>"
            )
        })?;

    let store = TemporalStore::open_default().context("failed to open Ephemeris database")?;
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
