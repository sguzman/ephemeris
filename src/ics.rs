use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Context;
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::domain::{SourceAuthority, SourceKind, TemporalSource};
use crate::ical::{
    IcalVcalendar, format_ical_content_line, parse_vcalendar, unescape_ical_text,
};
use crate::store::TemporalStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IcsImportReport {
    pub source_id: Uuid,
    pub source_external_ref: String,
    pub source_name: String,
    pub event_ids: BTreeSet<Uuid>,
    pub total_events: usize,
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub retained_missing: usize,
}

pub fn import_ics_file(
    store: &TemporalStore,
    path: impl AsRef<Path>,
) -> anyhow::Result<IcsImportReport> {
    let path = path.as_ref();
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read iCalendar file {}", path.display()))?;
    let canonical = std::fs::canonicalize(path)
        .with_context(|| format!("failed to canonicalize iCalendar file {}", path.display()))?;
    let locator = canonical.display().to_string();
    let external_ref = format!("ics:file:{locator}");
    import_ics_text(store, &raw, &external_ref, Some(&locator))
}

pub fn import_ics_text(
    store: &TemporalStore,
    raw: &str,
    external_ref: &str,
    locator: Option<&str>,
) -> anyhow::Result<IcsImportReport> {
    let calendar = parse_vcalendar(raw).context("failed to parse iCalendar VCALENDAR payload")?;
    let mut events = calendar
        .canonical_events()
        .context("failed to project iCalendar VEVENTs into canonical events")?;

    let source_name = calendar_display_name(&calendar, locator);
    let mut source = match store.source_by_external_ref(external_ref)? {
        Some(source) => source,
        None => {
            let mut source = TemporalSource::new(
                source_name.clone(),
                SourceKind::Ics,
                SourceAuthority::Unknown,
            );
            source.external_ref = Some(external_ref.to_string());
            source
        }
    };

    source.external_ref = Some(external_ref.to_string());
    source.name.clone_from(&source_name);
    source.kind = SourceKind::Ics;
    source.authority = SourceAuthority::Unknown;
    source.locator = locator.map(ToOwned::to_owned);
    source.enabled = true;
    source.read_only = true;
    source.updated_at = Utc::now();
    source.properties = source_properties(&calendar)?;

    let batch = store.import_batch(&source, &mut events)?;
    let event_ids = events.iter().map(|event| event.id).collect();

    Ok(IcsImportReport {
        source_id: source.id,
        source_external_ref: external_ref.to_string(),
        source_name,
        event_ids,
        total_events: events.len(),
        created: batch.created,
        updated: batch.updated,
        unchanged: batch.unchanged,
        retained_missing: batch.retained_missing,
    })
}

fn calendar_display_name(calendar: &IcalVcalendar, locator: Option<&str>) -> String {
    if let Some(property) = calendar
        .properties
        .iter()
        .find(|property| property.name == "X-WR-CALNAME")
    {
        let decoded = unescape_ical_text(&property.value)
            .unwrap_or_else(|_| property.value.clone());
        if !decoded.trim().is_empty() {
            return decoded;
        }
    }

    if let Some(locator) = locator
        && let Some(name) = Path::new(locator).file_name().and_then(|name| name.to_str())
        && !name.is_empty()
    {
        return name.to_string();
    }

    "iCalendar source".to_string()
}

fn source_properties(calendar: &IcalVcalendar) -> anyhow::Result<serde_json::Value> {
    let properties = calendar
        .properties
        .iter()
        .map(format_ical_content_line)
        .collect::<Result<Vec<_>, _>>()
        .context("failed to preserve VCALENDAR source properties")?;

    Ok(json!({
        "ical": {
            "transport": "rfc5545_vcalendar",
            "calendar_properties": properties,
            "vevent_component_count": calendar.events.len(),
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = concat!(
        "BEGIN:VCALENDAR\r\n",
        "PRODID:-//Ephemeris Fixture//EN\r\n",
        "VERSION:2.0\r\n",
        "X-WR-CALNAME:Fixture Calendar\r\n",
        "BEGIN:VEVENT\r\n",
        "UID:first@example.com\r\n",
        "DTSTAMP:20261006T120000Z\r\n",
        "DTSTART:20261007T090000Z\r\n",
        "SUMMARY:First\r\n",
        "END:VEVENT\r\n",
        "BEGIN:VEVENT\r\n",
        "UID:second@example.com\r\n",
        "DTSTAMP:20261006T120000Z\r\n",
        "DTSTART;VALUE=DATE:20261008\r\n",
        "SUMMARY:Second\r\n",
        "END:VEVENT\r\n",
        "END:VCALENDAR\r\n"
    );

    #[test]
    fn imports_ics_into_store_with_stable_source_and_event_identity() {
        let store = TemporalStore::open_in_memory().expect("store");
        let first = import_ics_text(
            &store,
            FIXTURE,
            "ics:test:fixture",
            Some("/tmp/fixture.ics"),
        )
        .expect("first import");

        assert_eq!(first.total_events, 2);
        assert_eq!(first.created, 2);
        assert_eq!(first.updated, 0);
        assert_eq!(first.unchanged, 0);
        assert_eq!(first.retained_missing, 0);
        assert_eq!(first.source_name, "Fixture Calendar");

        let source = store
            .source_by_external_ref("ics:test:fixture")
            .expect("source query")
            .expect("source");
        assert_eq!(source.id, first.source_id);
        assert_eq!(source.kind, SourceKind::Ics);
        assert!(source.read_only);
        assert_eq!(
            source.properties["ical"]["transport"],
            json!("rfc5545_vcalendar")
        );

        let first_event = store
            .event_by_source_record(first.source_id, "first@example.com")
            .expect("event query")
            .expect("first event");
        assert_eq!(first_event.normalized_title, "First");

        let second = import_ics_text(
            &store,
            FIXTURE,
            "ics:test:fixture",
            Some("/tmp/fixture.ics"),
        )
        .expect("second import");
        assert_eq!(second.source_id, first.source_id);
        assert_eq!(second.created, 0);
        assert_eq!(second.updated, 0);
        assert_eq!(second.unchanged, 2);
        assert_eq!(second.event_ids, first.event_ids);
    }

    #[test]
    fn refresh_updates_changed_uid_in_place() {
        let store = TemporalStore::open_in_memory().expect("store");
        let first = import_ics_text(&store, FIXTURE, "ics:test:update", None)
            .expect("first import");
        let before = store
            .event_by_source_record(first.source_id, "first@example.com")
            .expect("event query")
            .expect("event");

        let changed = FIXTURE.replace(
            "DTSTART:20261007T090000Z\r\nSUMMARY:First",
            "DTSTART:20261007T110000Z\r\nSUMMARY:First updated",
        );
        let report = import_ics_text(&store, &changed, "ics:test:update", None)
            .expect("refresh");

        assert_eq!(report.created, 0);
        assert_eq!(report.updated, 1);
        assert_eq!(report.unchanged, 1);

        let after = store
            .event_by_source_record(first.source_id, "first@example.com")
            .expect("event query")
            .expect("event");
        assert_eq!(after.id, before.id);
        assert_eq!(after.normalized_title, "First updated");
        assert_ne!(after.time, before.time);
    }

    #[test]
    fn refresh_retains_events_missing_from_later_snapshot() {
        let store = TemporalStore::open_in_memory().expect("store");
        let first = import_ics_text(&store, FIXTURE, "ics:test:missing", None)
            .expect("first import");

        let reduced = FIXTURE.replace(
            concat!(
                "BEGIN:VEVENT\r\n",
                "UID:second@example.com\r\n",
                "DTSTAMP:20261006T120000Z\r\n",
                "DTSTART;VALUE=DATE:20261008\r\n",
                "SUMMARY:Second\r\n",
                "END:VEVENT\r\n"
            ),
            "",
        );
        let report = import_ics_text(&store, &reduced, "ics:test:missing", None)
            .expect("refresh");

        assert_eq!(report.retained_missing, 1);
        assert!(store
            .event_by_source_record(first.source_id, "second@example.com")
            .expect("event query")
            .is_some());
    }

    #[test]
    fn file_import_uses_canonical_path_as_stable_source_reference() {
        let store = TemporalStore::open_in_memory().expect("store");
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("calendar.ics");
        std::fs::write(&path, FIXTURE).expect("write fixture");

        let first = import_ics_file(&store, &path).expect("file import");
        let second = import_ics_file(&store, &path).expect("file reimport");

        assert!(first.source_external_ref.starts_with("ics:file:"));
        assert_eq!(second.source_id, first.source_id);
        assert_eq!(second.unchanged, 2);
    }

    #[test]
    fn invalid_calendar_does_not_create_source() {
        let store = TemporalStore::open_in_memory().expect("store");
        let error = import_ics_text(
            &store,
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nEND:VCALENDAR\r\n",
            "ics:test:invalid",
            None,
        )
        .expect_err("invalid calendar must fail");

        assert!(error.to_string().contains("failed to parse iCalendar"));
        assert!(store
            .source_by_external_ref("ics:test:invalid")
            .expect("source query")
            .is_none());
    }
}
