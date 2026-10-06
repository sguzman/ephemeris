use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::Utc;
use serde_json::json;
use sha2::{Digest, Sha256};
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
    pub not_modified: bool,
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
    let target = format!("ics:file:{}", path.display());
    run_recorded_import(store, "ics_file", &target, || {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read iCalendar file {}", path.display()))?;
        let canonical = canonical_ics_file_path(path)?;
        let locator = canonical.display().to_string();
        let external_ref = ics_external_ref_for_canonical_path(&canonical);
        import_ics_text(store, &raw, &external_ref, Some(&locator))
    })
}

pub fn import_remote_ics(store: &TemporalStore, url: &str) -> anyhow::Result<IcsImportReport> {
    let normalized_url = normalize_remote_ics_url(url)?;
    let external_ref = remote_ics_external_ref(&normalized_url);
    run_recorded_import(store, "webcal", &external_ref, || {
        let existing = store.source_by_external_ref(&external_ref)?;
        let validators = existing
            .as_ref()
            .map(remote_http_validators)
            .unwrap_or_default();

        match fetch_remote_ics(&normalized_url, &validators)? {
            RemoteIcsFetch::Modified {
                raw,
                etag,
                last_modified,
            } => {
                let report = import_remote_ics_text(store, &raw, &normalized_url)?;
                persist_remote_http_validators(
                    store,
                    report.source_id,
                    etag.as_deref(),
                    last_modified.as_deref(),
                )?;
                Ok(report)
            }
            RemoteIcsFetch::NotModified => {
                let mut source = existing.ok_or_else(|| {
                    anyhow::anyhow!(
                        "remote calendar returned 304 before Ephemeris had a stored source"
                    )
                })?;
                source.updated_at = Utc::now();
                store.upsert_source(&source)?;
                let events = store.events_for_source(source.id)?;
                let event_ids = events.iter().map(|event| event.id).collect();
                Ok(IcsImportReport {
                    source_id: source.id,
                    source_external_ref: external_ref.clone(),
                    source_name: source.name,
                    event_ids,
                    total_events: events.len(),
                    created: 0,
                    updated: 0,
                    unchanged: events.len(),
                    retained_missing: 0,
                    not_modified: true,
                })
            }
        }
    })
}

pub fn import_remote_ics_text(
    store: &TemporalStore,
    raw: &str,
    url: &str,
) -> anyhow::Result<IcsImportReport> {
    let normalized_url = normalize_remote_ics_url(url)?;
    let external_ref = remote_ics_external_ref(&normalized_url);
    import_ics_text_with_source(
        store,
        raw,
        &external_ref,
        Some(&normalized_url),
        SourceKind::Webcal,
        Some("Remote iCalendar"),
    )
}

pub fn normalize_remote_ics_url(raw: &str) -> anyhow::Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("remote iCalendar URL is empty"));
    }

    let normalized = if let Some(rest) = trimmed.strip_prefix("webcal://") {
        format!("https://{rest}")
    } else if let Some(rest) = trimmed.strip_prefix("webcals://") {
        format!("https://{rest}")
    } else {
        trimmed.to_string()
    };

    if !(normalized.starts_with("https://") || normalized.starts_with("http://")) {
        return Err(anyhow::anyhow!(
            "remote iCalendar URL must use http, https, webcal, or webcals"
        ));
    }
    if normalized.chars().any(char::is_whitespace) {
        return Err(anyhow::anyhow!(
            "remote iCalendar URL contains unescaped whitespace"
        ));
    }

    Ok(normalized)
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
    let fallback_name = locator
        .and_then(|value| Path::new(value).file_name())
        .and_then(|value| value.to_str());
    import_ics_text_with_source(
        store,
        raw,
        external_ref,
        locator,
        SourceKind::Ics,
        fallback_name,
    )
}

fn import_ics_text_with_source(
    store: &TemporalStore,
    raw: &str,
    external_ref: &str,
    locator: Option<&str>,
    source_kind: SourceKind,
    fallback_name: Option<&str>,
) -> anyhow::Result<IcsImportReport> {
    let calendar = parse_vcalendar(raw).context("failed to parse iCalendar VCALENDAR payload")?;
    let mut events = calendar
        .canonical_events()
        .context("failed to project iCalendar VEVENTs into canonical events")?;

    let source_name = calendar_display_name(&calendar, fallback_name);
    let mut source = match store.source_by_external_ref(external_ref)? {
        Some(source) => source,
        None => {
            let mut source =
                TemporalSource::new(source_name.clone(), source_kind, SourceAuthority::Unknown);
            source.external_ref = Some(external_ref.to_string());
            source
        }
    };

    source.external_ref = Some(external_ref.to_string());
    source.name.clone_from(&source_name);
    source.kind = source_kind;
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
        not_modified: false,
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
    export_ics_source_record(store, &source, output_path)
}

pub fn export_ics_source_by_id(
    store: &TemporalStore,
    source_id: Uuid,
    output_path: impl AsRef<Path>,
) -> anyhow::Result<IcsExportReport> {
    let source = store
        .source_by_id(source_id)?
        .ok_or_else(|| anyhow::anyhow!("temporal source {source_id} does not exist"))?;
    export_ics_source_record(store, &source, output_path)
}

fn export_ics_source_record(
    store: &TemporalStore,
    source: &TemporalSource,
    output_path: impl AsRef<Path>,
) -> anyhow::Result<IcsExportReport> {
    if !matches!(source.kind, SourceKind::Ics | SourceKind::Webcal) {
        return Err(anyhow::anyhow!(
            "source {} is {}, not an iCalendar source",
            source.id,
            source.kind.as_str()
        ));
    }
    let source_external_ref = source
        .external_ref
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("ICS source {} is missing external identity", source.id))?;

    let events = store.events_for_source(source.id)?;
    let mut calendar = export_temporal_events_vcalendar(&events, "-//Ephemeris//EN")
        .context("failed to project canonical source events into VCALENDAR")?;
    preserve_source_calendar_properties(source, &mut calendar)?;
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

fn run_recorded_import(
    store: &TemporalStore,
    refresh_kind: &str,
    target: &str,
    operation: impl FnOnce() -> anyhow::Result<IcsImportReport>,
) -> anyhow::Result<IcsImportReport> {
    let attempt_id = store.begin_refresh_attempt(refresh_kind, target)?;
    let result = operation();

    let history_result = match &result {
        Ok(report) => {
            let summary = if report.not_modified {
                format!(
                    "{} events · remote representation not modified (HTTP 304)",
                    report.total_events
                )
            } else {
                format!(
                    "{} events · {} created · {} updated · {} unchanged · {} retained missing",
                    report.total_events,
                    report.created,
                    report.updated,
                    report.unchanged,
                    report.retained_missing
                )
            };
            store.finish_refresh_attempt(attempt_id, true, None, Some(&summary), None)
        }
        Err(error) => {
            let safe_error = error.to_string();
            store.finish_refresh_attempt(attempt_id, false, None, None, Some(&safe_error))
        }
    };

    match (result, history_result) {
        (Ok(report), Ok(())) => Ok(report),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(history_error)) => Err(history_error)
            .context("iCalendar import succeeded but refresh history could not be completed"),
        (Err(error), Err(history_error)) => Err(error).context(format!(
            "refresh history also failed to complete: {history_error:#}"
        )),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RemoteHttpValidators {
    etag: Option<String>,
    last_modified: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RemoteIcsFetch {
    Modified {
        raw: String,
        etag: Option<String>,
        last_modified: Option<String>,
    },
    NotModified,
}

fn fetch_remote_ics(
    url: &str,
    validators: &RemoteHttpValidators,
) -> anyhow::Result<RemoteIcsFetch> {
    let mut request = ureq::get(url)
        .header("Accept", "text/calendar, text/plain;q=0.9, */*;q=0.1")
        .header(
            "User-Agent",
            concat!("Ephemeris/", env!("CARGO_PKG_VERSION")),
        );
    if let Some(etag) = validators.etag.as_deref() {
        request = request.header("If-None-Match", etag);
    }
    if let Some(last_modified) = validators.last_modified.as_deref() {
        request = request.header("If-Modified-Since", last_modified);
    }

    let mut response = request
        .call()
        .with_context(|| "failed to fetch remote iCalendar source")?;
    if response.status().as_u16() == 304 {
        return Ok(RemoteIcsFetch::NotModified);
    }

    let etag = response_header(&response, "ETag");
    let last_modified = response_header(&response, "Last-Modified");
    let raw = response
        .body_mut()
        .read_to_string()
        .context("failed to read remote iCalendar response body")?;
    Ok(RemoteIcsFetch::Modified {
        raw,
        etag,
        last_modified,
    })
}

fn response_header(response: &ureq::http::Response<ureq::Body>, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn remote_http_validators(source: &TemporalSource) -> RemoteHttpValidators {
    let http = source
        .properties
        .get("ical")
        .and_then(|value| value.get("http"));
    RemoteHttpValidators {
        etag: http
            .and_then(|value| value.get("etag"))
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned),
        last_modified: http
            .and_then(|value| value.get("last_modified"))
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned),
    }
}

fn persist_remote_http_validators(
    store: &TemporalStore,
    source_id: Uuid,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> anyhow::Result<()> {
    let mut source = store
        .source_by_id(source_id)?
        .ok_or_else(|| anyhow::anyhow!("remote iCalendar source {source_id} disappeared"))?;
    let ical = source
        .properties
        .as_object_mut()
        .and_then(|properties| properties.get_mut("ical"))
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| anyhow::anyhow!("remote iCalendar source is missing transport metadata"))?;
    ical.insert(
        "http".to_string(),
        json!({
            "etag": etag,
            "last_modified": last_modified,
        }),
    );
    store.upsert_source(&source)
}

fn remote_ics_external_ref(normalized_url: &str) -> String {
    let digest = Sha256::digest(normalized_url.as_bytes());
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    format!("webcal:url-sha256:{encoded}")
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

fn calendar_display_name(calendar: &IcalVcalendar, fallback_name: Option<&str>) -> String {
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

    if let Some(name) = fallback_name
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
    fn remote_http_validators_roundtrip_through_source_properties() {
        let mut source = TemporalSource::new(
            "Remote",
            SourceKind::Webcal,
            SourceAuthority::Unknown,
        );
        source.properties = json!({
            "ical": {
                "http": {
                    "etag": "\"v1\"",
                    "last_modified": "Tue, 06 Oct 2026 20:00:00 GMT"
                }
            }
        });

        assert_eq!(
            remote_http_validators(&source),
            RemoteHttpValidators {
                etag: Some("\"v1\"".to_string()),
                last_modified: Some("Tue, 06 Oct 2026 20:00:00 GMT".to_string()),
            }
        );
    }

    #[test]
    fn remote_url_normalization_accepts_http_https_and_webcal() {
        assert_eq!(
            normalize_remote_ics_url("https://example.com/calendar.ics").expect("https"),
            "https://example.com/calendar.ics"
        );
        assert_eq!(
            normalize_remote_ics_url("http://example.com/calendar.ics").expect("http"),
            "http://example.com/calendar.ics"
        );
        assert_eq!(
            normalize_remote_ics_url("webcal://example.com/calendar.ics").expect("webcal"),
            "https://example.com/calendar.ics"
        );
        assert_eq!(
            normalize_remote_ics_url("webcals://example.com/calendar.ics").expect("webcals"),
            "https://example.com/calendar.ics"
        );
        assert!(normalize_remote_ics_url("file:///tmp/calendar.ics").is_err());
        assert!(normalize_remote_ics_url("https://example.com/bad path.ics").is_err());
    }

    #[test]
    fn remote_import_uses_hashed_source_identity_and_webcal_kind() {
        let store = TemporalStore::open_in_memory().expect("store");
        let first = import_remote_ics_text(
            &store,
            FIXTURE,
            "webcal://example.com/private/calendar.ics?token=secret",
        )
        .expect("remote import");

        assert!(first.source_external_ref.starts_with("webcal:url-sha256:"));
        assert!(!first.source_external_ref.contains("secret"));

        let source = store
            .source_by_id(first.source_id)
            .expect("source query")
            .expect("source");
        assert_eq!(source.kind, SourceKind::Webcal);
        assert_eq!(
            source.locator.as_deref(),
            Some("https://example.com/private/calendar.ics?token=secret")
        );

        let second = import_remote_ics_text(
            &store,
            FIXTURE,
            "https://example.com/private/calendar.ics?token=secret",
        )
        .expect("remote refresh");
        assert_eq!(second.source_id, first.source_id);
        assert_eq!(second.unchanged, 2);
    }

    #[test]
    fn file_import_records_success_and_failure_refresh_attempts() {
        let store = TemporalStore::open_in_memory().expect("store");
        let directory = tempfile::tempdir().expect("tempdir");
        let valid_path = directory.path().join("valid.ics");
        std::fs::write(&valid_path, FIXTURE).expect("write valid fixture");

        import_ics_file(&store, &valid_path).expect("successful import");

        let invalid_path = directory.path().join("invalid.ics");
        std::fs::write(
            &invalid_path,
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nEND:VCALENDAR\r\n",
        )
        .expect("write invalid fixture");
        import_ics_file(&store, &invalid_path).expect_err("invalid import must fail");

        let attempts = store.source_refresh_attempts(10).expect("refresh attempts");
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].refresh_kind, "ics_file");
        assert_eq!(attempts[0].success, Some(false));
        assert!(attempts[0].error.is_some());
        assert_eq!(attempts[1].refresh_kind, "ics_file");
        assert_eq!(attempts[1].success, Some(true));
        assert!(
            attempts[1]
                .summary
                .as_deref()
                .is_some_and(|summary| summary.contains("2 events"))
        );
    }

    #[test]
    fn recorded_webcal_attempt_uses_hashed_target_and_safe_failure_text() {
        let store = TemporalStore::open_in_memory().expect("store");
        let normalized =
            normalize_remote_ics_url("webcal://example.com/private/feed.ics?token=secret")
                .expect("normalize");
        let target = remote_ics_external_ref(&normalized);

        run_recorded_import(&store, "webcal", &target, || {
            Err(anyhow::anyhow!("synthetic remote failure"))
        })
        .expect_err("synthetic failure");

        let attempts = store.source_refresh_attempts(10).expect("refresh attempts");
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].refresh_kind, "webcal");
        assert!(attempts[0].target.starts_with("webcal:url-sha256:"));
        assert!(!attempts[0].target.contains("secret"));
        assert_eq!(
            attempts[0].error.as_deref(),
            Some("synthetic remote failure")
        );
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
        assert!(
            !report
                .output_path
                .with_file_name(".exported.ics.ephemeris.tmp")
                .exists()
        );

        let reparsed = parse_vcalendar(&exported)
            .expect("parse export")
            .canonical_events()
            .expect("canonical export");
        assert_eq!(reparsed.len(), 2);
        assert!(
            reparsed
                .iter()
                .any(|event| event.normalized_title == "First edited")
        );
    }

    #[test]
    fn source_export_by_id_uses_the_same_canonical_source() {
        let store = TemporalStore::open_in_memory().expect("store");
        let directory = tempfile::tempdir().expect("tempdir");
        let source_path = directory.path().join("source-by-id.ics");
        let output_path = directory.path().join("source-by-id-export.ics");
        std::fs::write(&source_path, FIXTURE).expect("write fixture");

        let imported = import_ics_file(&store, &source_path).expect("import");
        let report = export_ics_source_by_id(&store, imported.source_id, &output_path)
            .expect("export source by id");

        assert_eq!(report.source_id, imported.source_id);
        assert_eq!(report.source_external_ref, imported.source_external_ref);
        assert_eq!(report.total_events, 2);
        assert!(output_path.exists());
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
