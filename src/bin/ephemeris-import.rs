use std::path::PathBuf;

use anyhow::{Context, anyhow};
use ephemeris::csv::import_csv_file;
use ephemeris::ics::{import_ics_file, import_remote_ics};
use ephemeris::interchange::import_canonical_json_file;
use ephemeris::store::TemporalStore;
use ephemeris::taria::import_reconciled_event_set_file;

fn main() -> anyhow::Result<()> {
    let input = std::env::args_os().nth(1).ok_or_else(|| {
        anyhow!(
            "usage: ephemeris-import <calendar.ics|calendar-url|events.csv|snapshot.ephemeris.json|taria-reconciled-event-set.json>"
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

    if is_canonical_json_path(&path) {
        let report = import_canonical_json_file(&store, &path)?;
        println!("Imported Ephemeris canonical JSON snapshot");
        println!(
            "Sources: {} created  {} updated  {} unchanged",
            report.sources_created, report.sources_updated, report.sources_unchanged
        );
        println!(
            "Entities: {} created  {} updated  {} unchanged",
            report.entities_created, report.entities_updated, report.entities_unchanged
        );
        println!(
            "Events: {} created  {} updated  {} unchanged",
            report.events_created, report.events_updated, report.events_unchanged
        );
        println!(
            "Relations: {} created  {} updated  {} unchanged",
            report.relations_created, report.relations_updated, report.relations_unchanged
        );
        println!(
            "Collections: {} created  {} updated  {} unchanged",
            report.collections_created, report.collections_updated, report.collections_unchanged
        );
        println!(
            "Collection memberships: {} replaced  {} unchanged",
            report.collection_memberships_replaced, report.collection_memberships_unchanged
        );
        println!(
            "Identity assessments: {} created  {} updated  {} unchanged",
            report.identity_assessments_created,
            report.identity_assessments_updated,
            report.identity_assessments_unchanged
        );
        println!(
            "Annotations: {} created  {} updated  {} unchanged",
            report.annotations_created, report.annotations_updated, report.annotations_unchanged
        );
        println!(
            "Provenance records: {} created  {} updated  {} unchanged",
            report.provenance_records_created,
            report.provenance_records_updated,
            report.provenance_records_unchanged
        );
        println!(
            "Participant entity bindings: {} created  {} updated  {} unchanged",
            report.participant_entity_bindings_created,
            report.participant_entity_bindings_updated,
            report.participant_entity_bindings_unchanged
        );
        return Ok(());
    }

    if extension.as_deref() == Some("csv") {
        let report = import_csv_file(&store, &path)?;
        println!("Imported CSV source: {}", report.source_name);
        println!("Source: {}", report.source_external_ref);
        println!("Events in CSV: {}", report.total_events);
        println!(
            "Created: {}  Updated: {}  Unchanged: {}",
            report.created, report.updated, report.unchanged
        );
        println!("Prior missing retained: {}", report.retained_missing);
        return Ok(());
    }

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

fn is_canonical_json_path(path: &std::path::Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.ends_with(".ephemeris.json"))
}

#[cfg(test)]
mod tests {
    use super::is_canonical_json_path;

    #[test]
    fn canonical_snapshot_suffix_is_explicit() {
        assert!(is_canonical_json_path(std::path::Path::new(
            "/tmp/snapshot.ephemeris.json"
        )));
        assert!(!is_canonical_json_path(std::path::Path::new(
            "/tmp/reconciled-event-set.json"
        )));
        assert!(!is_canonical_json_path(std::path::Path::new(
            "/tmp/calendar.ics"
        )));
    }
}
