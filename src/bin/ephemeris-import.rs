use std::path::PathBuf;

use anyhow::{Context, anyhow};
use ephemeris::store::TemporalStore;
use ephemeris::taria::import_reconciled_event_set_file;

fn main() -> anyhow::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("usage: ephemeris-import <taria-reconciled-event-set.json>"))?;

    let store = TemporalStore::open_default().context("failed to open Ephemeris database")?;
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
