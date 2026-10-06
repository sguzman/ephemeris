use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::domain::{SourceAuthority, SourceKind, TemporalSource};
use crate::ical::{
    IcalVcalendar, export_temporal_events_vcalendar, format_ical_content_line, format_vcalendar,
    parse_ical_content_line, parse_vcalendar, unescape_ical_text,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IcsExportReport {
    pub source_id: Uuid,
    pub source_external_ref: String,
    pub total_events: usize,
    pub output_path: PathBuf,
}

pub fn import_ics_file(
    store: &TemporalStore,
    path: impl AsRef<Path>,
) -> anyhow::Result<IcsImportReport> {
    let path = path.as_ref();
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read iCalendar file {}", path.display()))?;
    let canonical = canonical_ics_file_path(path)?;
    let locator = canonical.display().to_string();
    let external_ref = ics_external_ref_for_canonical_path(&canonical);
    import_ics_text(store, &raw, &external_ref, Some(&locator))
}

pub fn ics_file_external_ref(path: impl AsRef<Path>) -> anyhow::Result<String> {
    let canonical = canonical_ics_file_path(path.as_ref())?;
    Ok(ics_external_ref_for_canonical_path(&canonical))
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

pub fn export_ics_source_file(
    store: &TemporalStore,
    source_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
) -> anyhow::Result<IcsExportReport> {
    let external_ref = ics_file_external_ref(source_path)?;
    export_ics_source(store, &external_ref, output_path)
}

pub fn export_ics_source(
    store: &TemporalStore,
    source_external_ref: &str,
    output_path: impl AsRef<Path>,
) -> anyhow::Result<IcsExportReport> {
    let source = store
        .source_by_external_ref(source_external_ref)?
        .ok_or_else(|| anyhow::anyhow!("iCalendar source {source_external_ref} is not imported"))?;
    if source.kind != SourceKind::Ics {
        return Err(anyhow::anyhow!(
            "source {source_external_ref} is {}, not an ICS source",
            source.kind.as_str()
        ));
    }

    let events = store.events_for_source(source.id)?;
    let mut calendar = export_temporal_events_vcalendar(&events, "-//Ephemeris//EN")
        .context("failed to project canonical source events into VCALENDAR")?;
    preserve_source_calendar_properties(&source, &mut calendar)?;
    let encoded = format_vcalendar(&calendar).context("failed to serialize VCALENDAR")?;

    let output_path = output_path.as_ref().to_path_buf();
    write_calendar_atomically(&output_path, &encoded)?;

    Ok(IcsExportReport {
        source_id: source.id,
        source_external_ref: source_external_ref.to_string(),
        total_events: events.len(),
        output_path,
    })
}

fn canonical_ics_file_path(path: &Path) -> anyhow::Result<PathBuf> {
    std::fs::canonicalize(path)
        .with_context(|| format!("failed to canonicalize iCalendar file {}", path.display()))
}

fn ics_external_ref_for_canonical_path(path: &Path) -> String {
    format!("ics:file:{}", path.display())
}

fn preserve_source_calendar_properties(
    source: &TemporalSource,
    calendar: &mut IcalVcalendar,
) -> anyhow::Result<()> {
    let Some(properties) = source
        .properties
        .get("ical")
        .and_then(|value| value.get("calendar_properties"))
        .and_then(|value| value.as_array())
    else {
        return Ok(());
    };

    for raw in properties {
        let Some(raw) = raw.as_str() else {
            continue;
        };
        let property = parse_ical_content_line(raw)
            .with_context(|| format!("invalid preserved VCALENDAR property {raw:?}"))?;
        if matches!(property.name.as_str(), "PRODID" | "VERSION" | "CALSCALE") {
            continue;
        }
        calendar.properties.push(property);
    }
    Ok(())
}

fn write_calendar_atomically(path: &Path, encoded: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("calendar.ics");
    let temporary = path.with_file_name(format!(".{file_name}.ephemeris.tmp"));
    std::fs::write(&temporary, encoded)
        .with_context(|| format!("failed to write {}", temporary.display()))?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error).with_context(|| format!("failed to replace {}", path.display()));
    }
    Ok(())
}

fn calendar_display_name(calendar: &IcalVcalendar, locator: Option<&str>) -> String {
    if let Some(property) = calendar
        .properties
        .iter()
        .find(|property| property.name == "X-WR-CALNAME")
    {
        let decoded =
            unescape_ical_text(&property.value).unwrap_or_else(|_| property.value.clone());
        if !decoded.trim().is_empty() {
            return decoded;
        }
    }

    if let Some(locator) = locator
        && let Some(name) = Path::new(locator)
            .file_name()
            .and_then(|name| name.to_str())
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
        let first =
            import_ics_text(&store, FIXTURE, "ics:test:update", None).expect("first import");
        let before = store
            .event_by_source_record(first.source_id, "first@example.com")
            .expect("event query")
            .expect("event");

        let changed = FIXTURE.replace(
            "DTSTART:20261007T090000Z\r\nSUMMARY:First",
            "DTSTART:20261007T110000Z\r\nSUMMARY:First updated",
        );
        let report = import_ics_text(&store, &changed, "ics:test:update", None).expect("refresh");

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
        let first =
            import_ics_text(&store, FIXTURE, "ics:test:missing", None).expect("first import");

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
        let report = import_ics_text(&store, &reduced, "ics:test:missing", None).expect("refresh");

        assert_eq!(report.retained_missing, 1);
        assert!(
            store
                .event_by_source_record(first.source_id, "second@example.com")
                .expect("event query")
                .is_some()
        );
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
    fn imported_file_source_exports_canonical_changes_and_preserves_calendar_name() {
        let store = TemporalStore::open_in_memory().expect("store");
        let directory = tempfile::tempdir().expect("tempdir");
        let source_path = directory.path().join("source.ics");
        let output_path = directory.path().join("exported.ics");
        std::fs::write(&source_path, FIXTURE).expect("write fixture");

        let imported = import_ics_file(&store, &source_path).expect("import");
        let mut event = store
            .event_by_source_record(imported.source_id, "first@example.com")
            .expect("event query")
            .expect("event");
        event.normalized_title = "First edited".to_string();
        event.updated_at = Utc::now();
        store.upsert_event(&event).expect("save edit");

        let report =
            export_ics_source_file(&store, &source_path, &output_path).expect("export source");
        assert_eq!(report.source_id, imported.source_id);
        assert_eq!(report.total_events, 2);
        assert_eq!(report.output_path, output_path);

        let exported = std::fs::read_to_string(&report.output_path).expect("read export");
        assert!(exported.contains("PRODID:-//Ephemeris//EN"));
        assert!(exported.contains("X-WR-CALNAME:Fixture Calendar"));
        assert!(exported.contains("SUMMARY:First edited"));
        assert!(!report
            .output_path
            .with_file_name(".exported.ics.ephemeris.tmp")
            .exists());

        let reparsed = parse_vcalendar(&exported)
            .expect("parse export")
            .canonical_events()
            .expect("canonical export");
        assert_eq!(reparsed.len(), 2);
        assert!(reparsed
            .iter()
            .any(|event| event.normalized_title == "First edited"));
    }

    #[test]
    fn source_export_rejects_missing_source_without_writing_output() {
        let store = TemporalStore::open_in_memory().expect("store");
        let directory = tempfile::tempdir().expect("tempdir");
        let output_path = directory.path().join("missing.ics");

        let error = export_ics_source(&store, "ics:test:missing-source", &output_path)
            .expect_err("missing source must fail");
        assert!(error.to_string().contains("is not imported"));
        assert!(!output_path.exists());
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
        assert!(
            store
                .source_by_external_ref("ics:test:invalid")
                .expect("source query")
                .is_none()
        );
    }
}
