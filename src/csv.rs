use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ::csv as csv_crate;
use anyhow::{Context, anyhow};
use chrono::{DateTime, NaiveDate, NaiveDateTime, SecondsFormat, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::{
    EventStatus, SourceAuthority, SourceKind, TemporalEvent, TemporalSource, TimeSpec,
};
use crate::store::TemporalStore;

pub const EPHEMERIS_CSV_VERSION: &str = "1";

const CSV_HEADERS: [&str; 20] = [
    "schema_version",
    "record_key",
    "title",
    "raw_title",
    "description",
    "event_type",
    "domain",
    "jurisdiction",
    "institution",
    "status",
    "confidence",
    "importance",
    "personal_relevance",
    "upstream_event_ref",
    "upstream_reconciled_key",
    "renderability",
    "time_kind",
    "start",
    "end",
    "source_timezone",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct CsvEventRow {
    schema_version: String,
    record_key: String,
    title: String,
    raw_title: Option<String>,
    description: Option<String>,
    event_type: Option<String>,
    domain: Option<String>,
    jurisdiction: Option<String>,
    institution: Option<String>,
    status: String,
    confidence: Option<f32>,
    importance: Option<i32>,
    personal_relevance: Option<i32>,
    upstream_event_ref: Option<String>,
    upstream_reconciled_key: Option<String>,
    renderability: Option<String>,
    time_kind: String,
    start: String,
    end: Option<String>,
    #[serde(default)]
    source_timezone: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvImportReport {
    pub source_id: Uuid,
    pub source_external_ref: String,
    pub source_name: String,
    pub total_events: usize,
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub retained_missing: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvExportReport {
    pub source_id: Uuid,
    pub total_events: usize,
    pub output_path: PathBuf,
}

pub fn import_csv_file(
    store: &TemporalStore,
    path: impl AsRef<Path>,
) -> anyhow::Result<CsvImportReport> {
    let path = path.as_ref();
    let history_target = std::fs::canonicalize(path).map_or_else(
        |_| format!("csv:file:{}", path.display()),
        |canonical| format!("csv:file:{}", canonical.display()),
    );

    run_recorded_import(store, &history_target, || {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read CSV file {}", path.display()))?;
        let canonical = std::fs::canonicalize(path)
            .with_context(|| format!("failed to canonicalize CSV file {}", path.display()))?;
        let locator = canonical.display().to_string();
        let external_ref = format!("csv:file:{locator}");
        let fallback_name = canonical
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("CSV source");
        import_csv_text(store, &raw, &external_ref, Some(&locator), fallback_name)
    })
}

fn run_recorded_import(
    store: &TemporalStore,
    target: &str,
    operation: impl FnOnce() -> anyhow::Result<CsvImportReport>,
) -> anyhow::Result<CsvImportReport> {
    let attempt_id = store.begin_refresh_attempt("csv_file", target)?;
    let result = operation();

    let history_result = match &result {
        Ok(report) => {
            let summary = format!(
                "{} events · {} created · {} updated · {} unchanged · {} retained missing",
                report.total_events,
                report.created,
                report.updated,
                report.unchanged,
                report.retained_missing
            );
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
            .context("CSV import succeeded but refresh history could not be completed"),
        (Err(error), Err(history_error)) => Err(error).context(format!(
            "refresh history also failed to complete: {history_error:#}"
        )),
    }
}

pub fn import_csv_text(
    store: &TemporalStore,
    raw: &str,
    external_ref: &str,
    locator: Option<&str>,
    source_name: &str,
) -> anyhow::Result<CsvImportReport> {
    let mut events = parse_csv_events(raw)?;

    let mut source = match store.source_by_external_ref(external_ref)? {
        Some(source) => source,
        None => {
            let mut source = TemporalSource::new(
                source_name.to_string(),
                SourceKind::Csv,
                SourceAuthority::Unknown,
            );
            source.external_ref = Some(external_ref.to_string());
            source
        }
    };
    source.external_ref = Some(external_ref.to_string());
    source.name = source_name.to_string();
    source.kind = SourceKind::Csv;
    source.authority = SourceAuthority::Unknown;
    source.locator = locator.map(ToOwned::to_owned);
    source.enabled = true;
    source.read_only = true;
    source.updated_at = Utc::now();
    source.properties = serde_json::json!({
        "csv": {
            "schema": "ephemeris.events",
            "version": EPHEMERIS_CSV_VERSION,
        }
    });

    let batch = store.import_batch(&source, &mut events)?;
    Ok(CsvImportReport {
        source_id: source.id,
        source_external_ref: external_ref.to_string(),
        source_name: source.name,
        total_events: events.len(),
        created: batch.created,
        updated: batch.updated,
        unchanged: batch.unchanged,
        retained_missing: batch.retained_missing,
    })
}

pub fn export_source_csv_by_id(
    store: &TemporalStore,
    source_id: Uuid,
    output_path: impl AsRef<Path>,
) -> anyhow::Result<CsvExportReport> {
    let source = store
        .source_by_id(source_id)?
        .ok_or_else(|| anyhow!("temporal source {source_id} does not exist"))?;
    let events = store.events_for_source(source.id)?;
    let encoded = format_csv_events(&events)?;
    let output_path = output_path.as_ref().to_path_buf();
    write_atomically(&output_path, &encoded)?;
    Ok(CsvExportReport {
        source_id,
        total_events: events.len(),
        output_path,
    })
}

pub fn parse_csv_events(raw: &str) -> anyhow::Result<Vec<TemporalEvent>> {
    let mut reader = csv_crate::ReaderBuilder::new()
        .flexible(false)
        .trim(csv_crate::Trim::All)
        .from_reader(raw.as_bytes());

    let headers = reader
        .headers()
        .context("failed to read Ephemeris CSV header")?
        .clone();
    let expected = CSV_HEADERS.to_vec();
    let actual = headers.iter().collect::<Vec<_>>();
    if actual != expected {
        return Err(anyhow!(
            "unsupported Ephemeris CSV header; expected {:?}",
            expected
        ));
    }

    let mut record_keys = HashSet::new();
    let mut events = Vec::new();
    for (index, row) in reader.deserialize::<CsvEventRow>().enumerate() {
        let row = row.with_context(|| format!("failed to parse CSV row {}", index + 2))?;
        if row.schema_version != EPHEMERIS_CSV_VERSION {
            return Err(anyhow!(
                "CSV row {} uses schema version {:?}; expected {:?}",
                index + 2,
                row.schema_version,
                EPHEMERIS_CSV_VERSION
            ));
        }
        let record_key = row.record_key.trim();
        if record_key.is_empty() {
            return Err(anyhow!("CSV row {} has an empty record_key", index + 2));
        }
        if !record_keys.insert(record_key.to_string()) {
            return Err(anyhow!("duplicate CSV record_key {record_key:?}"));
        }
        events.push(row_into_event(row).with_context(|| format!("invalid CSV row {}", index + 2))?);
    }
    Ok(events)
}

pub fn format_csv_events(events: &[TemporalEvent]) -> anyhow::Result<String> {
    let mut encoded = Vec::new();
    {
        let mut writer = csv_crate::WriterBuilder::new()
            .has_headers(false)
            .from_writer(&mut encoded);
        writer
            .write_record(CSV_HEADERS)
            .context("failed to write Ephemeris CSV header")?;
        for event in events {
            writer
                .serialize(event_into_row(event)?)
                .with_context(|| format!("failed to serialize event {}", event.id))?;
        }
        writer.flush().context("failed to flush Ephemeris CSV")?;
    }
    String::from_utf8(encoded).context("Ephemeris CSV serializer produced invalid UTF-8")
}

fn row_into_event(row: CsvEventRow) -> anyhow::Result<TemporalEvent> {
    if row.title.trim().is_empty() {
        return Err(anyhow!("title is empty"));
    }
    let status = EventStatus::parse(&row.status)
        .ok_or_else(|| anyhow!("unknown event status {:?}", row.status))?;
    let source_timezone = nonempty(row.source_timezone);
    validate_timezone(source_timezone.as_deref())?;

    let time = parse_time(
        &row.time_kind,
        &row.start,
        row.end.as_deref(),
        source_timezone,
    )?;
    let mut event = TemporalEvent::new(row.title, time);
    event.source_record_key = Some(row.record_key);
    event.raw_title = nonempty(row.raw_title);
    event.description = nonempty(row.description);
    event.event_type = nonempty(row.event_type);
    event.domain = nonempty(row.domain);
    event.jurisdiction = nonempty(row.jurisdiction);
    event.institution = nonempty(row.institution);
    event.status = status;
    event.confidence = row.confidence;
    event.importance = row.importance;
    event.personal_relevance = row.personal_relevance;
    event.upstream_event_ref = nonempty(row.upstream_event_ref);
    event.upstream_reconciled_key = nonempty(row.upstream_reconciled_key);
    event.renderability = nonempty(row.renderability);
    Ok(event)
}

fn event_into_row(event: &TemporalEvent) -> anyhow::Result<CsvEventRow> {
    if event.recurrence.is_some() {
        return Err(anyhow!(
            "event {} has recurrence, which Ephemeris CSV v1 cannot represent",
            event.id
        ));
    }
    if !event.assertion_refs.is_empty()
        || !event.source_refs.is_empty()
        || !event.provenance_refs.is_empty()
        || !event.tags.is_empty()
    {
        return Err(anyhow!(
            "event {} has list-valued metadata, which Ephemeris CSV v1 cannot represent",
            event.id
        ));
    }
    if event
        .properties
        .as_object()
        .is_none_or(|properties| !properties.is_empty())
    {
        return Err(anyhow!(
            "event {} has arbitrary properties, which Ephemeris CSV v1 cannot represent",
            event.id
        ));
    }

    let (time_kind, start, end, source_timezone) = format_time(&event.time)?;
    validate_timezone(source_timezone.as_deref())?;

    Ok(CsvEventRow {
        schema_version: EPHEMERIS_CSV_VERSION.to_string(),
        record_key: event
            .source_record_key
            .clone()
            .unwrap_or_else(|| event.id.to_string()),
        title: event.normalized_title.clone(),
        raw_title: event.raw_title.clone(),
        description: event.description.clone(),
        event_type: event.event_type.clone(),
        domain: event.domain.clone(),
        jurisdiction: event.jurisdiction.clone(),
        institution: event.institution.clone(),
        status: event.status.as_str().to_string(),
        confidence: event.confidence,
        importance: event.importance,
        personal_relevance: event.personal_relevance,
        upstream_event_ref: event.upstream_event_ref.clone(),
        upstream_reconciled_key: event.upstream_reconciled_key.clone(),
        renderability: event.renderability.clone(),
        time_kind,
        start,
        end,
        source_timezone,
    })
}

fn parse_time(
    kind: &str,
    start: &str,
    end: Option<&str>,
    source_timezone: Option<String>,
) -> anyhow::Result<TimeSpec> {
    match kind.trim() {
        "instant" => Ok(TimeSpec::Instant {
            start_utc: parse_instant(start)?,
            end_utc: end
                .filter(|value| !value.is_empty())
                .map(parse_instant)
                .transpose()?,
            source_timezone,
        }),
        "floating" => Ok(TimeSpec::Floating {
            start: parse_floating(start)?,
            end: end
                .filter(|value| !value.is_empty())
                .map(parse_floating)
                .transpose()?,
            source_timezone,
        }),
        "all_day" => {
            if source_timezone.is_some() {
                return Err(anyhow!("all_day rows cannot declare source_timezone"));
            }
            Ok(TimeSpec::AllDay {
                start: parse_date(start)?,
                end_exclusive: end
                    .filter(|value| !value.is_empty())
                    .map(parse_date)
                    .transpose()?,
            })
        }
        "date_only" => {
            if source_timezone.is_some() {
                return Err(anyhow!("date_only rows cannot declare source_timezone"));
            }
            Ok(TimeSpec::DateOnly {
                start: parse_date(start)?,
                end_exclusive: end
                    .filter(|value| !value.is_empty())
                    .map(parse_date)
                    .transpose()?,
            })
        }
        other => Err(anyhow!(
            "unsupported time_kind {other:?}; CSV v1 supports instant, floating, all_day, and date_only"
        )),
    }
}

fn format_time(
    time: &TimeSpec,
) -> anyhow::Result<(String, String, Option<String>, Option<String>)> {
    match time {
        TimeSpec::Instant {
            start_utc,
            end_utc,
            source_timezone,
        } => Ok((
            "instant".to_string(),
            start_utc.to_rfc3339_opts(SecondsFormat::AutoSi, true),
            end_utc.map(|value| value.to_rfc3339_opts(SecondsFormat::AutoSi, true)),
            source_timezone.clone(),
        )),
        TimeSpec::Floating {
            start,
            end,
            source_timezone,
        } => Ok((
            "floating".to_string(),
            format_floating(*start),
            end.map(format_floating),
            source_timezone.clone(),
        )),
        TimeSpec::AllDay {
            start,
            end_exclusive,
        } => Ok((
            "all_day".to_string(),
            start.format("%Y-%m-%d").to_string(),
            end_exclusive.map(|value| value.format("%Y-%m-%d").to_string()),
            None,
        )),
        TimeSpec::DateOnly {
            start,
            end_exclusive,
        } => Ok((
            "date_only".to_string(),
            start.format("%Y-%m-%d").to_string(),
            end_exclusive.map(|value| value.format("%Y-%m-%d").to_string()),
            None,
        )),
        TimeSpec::Month { .. } | TimeSpec::Year { .. } | TimeSpec::Unknown { .. } => Err(anyhow!(
            "time kind {} is not representable by Ephemeris CSV v1",
            time.kind_name()
        )),
    }
}

fn parse_instant(value: &str) -> anyhow::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .with_context(|| format!("invalid RFC3339 instant {value:?}"))
}

fn parse_floating(value: &str) -> anyhow::Result<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f")
        .with_context(|| format!("invalid floating DATE-TIME {value:?}"))
}

fn format_floating(value: NaiveDateTime) -> String {
    value.format("%Y-%m-%dT%H:%M:%S%.f").to_string()
}

fn parse_date(value: &str) -> anyhow::Result<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").with_context(|| format!("invalid DATE {value:?}"))
}

fn validate_timezone(value: Option<&str>) -> anyhow::Result<()> {
    if let Some(value) = value {
        value
            .parse::<Tz>()
            .with_context(|| format!("invalid IANA source timezone {value:?}"))?;
    }
    Ok(())
}

fn nonempty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

fn write_atomically(path: &Path, encoded: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("ephemeris.csv");
    let temporary = path.with_file_name(format!(".{file_name}.ephemeris.tmp"));
    std::fs::write(&temporary, encoded)
        .with_context(|| format!("failed to write {}", temporary.display()))?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error).with_context(|| format!("failed to replace {}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use super::*;
    use crate::domain::{RecurrenceFrequency, RecurrenceRule};

    fn simple_event(title: &str, record_key: &str, time: TimeSpec) -> TemporalEvent {
        let mut event = TemporalEvent::new(title, time);
        event.source_record_key = Some(record_key.to_string());
        event.description = Some("Description".to_string());
        event.domain = Some("test".to_string());
        event.status = EventStatus::Confirmed;
        event.confidence = Some(0.9);
        event
    }

    #[test]
    fn csv_empty_export_keeps_a_reimportable_schema_header() {
        let encoded = format_csv_events(&[]).expect("format empty CSV");
        let decoded = parse_csv_events(&encoded).expect("parse empty CSV");
        assert!(decoded.is_empty());
        assert!(encoded.starts_with("schema_version,record_key,title,"));
    }

    #[test]
    fn csv_roundtrips_supported_time_kinds_and_scalar_metadata() {
        let instant_start = Utc
            .with_ymd_and_hms(2026, 10, 7, 9, 0, 0)
            .single()
            .expect("instant");
        let floating_start = NaiveDate::from_ymd_opt(2026, 10, 8)
            .expect("date")
            .and_hms_opt(14, 30, 0)
            .expect("floating");
        let events = vec![
            simple_event(
                "Instant",
                "instant",
                TimeSpec::Instant {
                    start_utc: instant_start,
                    end_utc: Some(instant_start + Duration::hours(1)),
                    source_timezone: Some("America/Mexico_City".to_string()),
                },
            ),
            simple_event(
                "Floating",
                "floating",
                TimeSpec::Floating {
                    start: floating_start,
                    end: Some(floating_start + Duration::minutes(45)),
                    source_timezone: None,
                },
            ),
            simple_event(
                "All day",
                "all-day",
                TimeSpec::AllDay {
                    start: NaiveDate::from_ymd_opt(2026, 10, 9).expect("date"),
                    end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 10, 10).expect("end date")),
                },
            ),
            simple_event(
                "Date only",
                "date-only",
                TimeSpec::DateOnly {
                    start: NaiveDate::from_ymd_opt(2026, 10, 11).expect("date"),
                    end_exclusive: None,
                },
            ),
        ];

        let encoded = format_csv_events(&events).expect("format CSV");
        let decoded = parse_csv_events(&encoded).expect("parse CSV");
        assert_eq!(decoded.len(), events.len());
        for (decoded, original) in decoded.iter().zip(events.iter()) {
            assert_eq!(decoded.source_record_key, original.source_record_key);
            assert_eq!(decoded.normalized_title, original.normalized_title);
            assert_eq!(decoded.description, original.description);
            assert_eq!(decoded.domain, original.domain);
            assert_eq!(decoded.status, original.status);
            assert_eq!(decoded.confidence, original.confidence);
            assert_eq!(decoded.time, original.time);
        }
    }

    #[test]
    fn csv_import_refresh_preserves_source_and_event_identity() {
        let store = TemporalStore::open_in_memory().expect("store");
        let raw = concat!(
            "schema_version,record_key,title,raw_title,description,event_type,domain,jurisdiction,institution,status,confidence,importance,personal_relevance,upstream_event_ref,upstream_reconciled_key,renderability,time_kind,start,end,source_timezone\n",
            "1,a,Alpha,,First,,test,,,scheduled,,,,,,,date_only,2026-10-07,,\n",
            "1,b,Beta,,,,,,,confirmed,,,,,,,all_day,2026-10-08,2026-10-09,\n"
        );
        let first =
            import_csv_text(&store, raw, "csv:test:fixture", None, "Fixture").expect("import");
        assert_eq!(first.created, 2);
        let alpha = store
            .event_by_source_record(first.source_id, "a")
            .expect("query")
            .expect("alpha");

        let changed = raw.replace("Alpha,,First", "Alpha updated,,First");
        let second = import_csv_text(&store, &changed, "csv:test:fixture", None, "Fixture")
            .expect("refresh");
        assert_eq!(second.source_id, first.source_id);
        assert_eq!(second.created, 0);
        assert_eq!(second.updated, 1);
        assert_eq!(second.unchanged, 1);
        let alpha_after = store
            .event_by_source_record(first.source_id, "a")
            .expect("query")
            .expect("alpha");
        assert_eq!(alpha_after.id, alpha.id);
        assert_eq!(alpha_after.normalized_title, "Alpha updated");
    }

    #[test]
    fn csv_import_retains_rows_missing_from_later_snapshot() {
        let store = TemporalStore::open_in_memory().expect("store");
        let raw = concat!(
            "schema_version,record_key,title,raw_title,description,event_type,domain,jurisdiction,institution,status,confidence,importance,personal_relevance,upstream_event_ref,upstream_reconciled_key,renderability,time_kind,start,end,source_timezone\n",
            "1,a,Alpha,,,,,,,scheduled,,,,,,,date_only,2026-10-07,,\n",
            "1,b,Beta,,,,,,,scheduled,,,,,,,date_only,2026-10-08,,\n"
        );
        let first =
            import_csv_text(&store, raw, "csv:test:missing", None, "Fixture").expect("first");
        let reduced = concat!(
            "schema_version,record_key,title,raw_title,description,event_type,domain,jurisdiction,institution,status,confidence,importance,personal_relevance,upstream_event_ref,upstream_reconciled_key,renderability,time_kind,start,end,source_timezone\n",
            "1,a,Alpha,,,,,,,scheduled,,,,,,,date_only,2026-10-07,,\n"
        );
        let second =
            import_csv_text(&store, reduced, "csv:test:missing", None, "Fixture").expect("refresh");
        assert_eq!(second.retained_missing, 1);
        assert!(
            store
                .event_by_source_record(first.source_id, "b")
                .expect("query")
                .is_some()
        );
    }

    #[test]
    fn csv_rejects_duplicate_record_keys_and_unknown_schema() {
        let duplicate = concat!(
            "schema_version,record_key,title,raw_title,description,event_type,domain,jurisdiction,institution,status,confidence,importance,personal_relevance,upstream_event_ref,upstream_reconciled_key,renderability,time_kind,start,end,source_timezone\n",
            "1,a,Alpha,,,,,,,scheduled,,,,,,,date_only,2026-10-07,,\n",
            "1,a,Beta,,,,,,,scheduled,,,,,,,date_only,2026-10-08,,\n"
        );
        assert!(parse_csv_events(duplicate).is_err());

        let wrong_version = duplicate.replacen("1,a,Alpha", "2,a,Alpha", 1);
        assert!(parse_csv_events(&wrong_version).is_err());
    }

    #[test]
    fn csv_export_rejects_nonrepresentable_semantics() {
        let mut recurrence = simple_event(
            "Recurring",
            "recurring",
            TimeSpec::DateOnly {
                start: NaiveDate::from_ymd_opt(2026, 10, 7).expect("date"),
                end_exclusive: None,
            },
        );
        recurrence.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Daily));
        assert!(format_csv_events(&[recurrence]).is_err());

        let month = simple_event(
            "Month precision",
            "month",
            TimeSpec::Month {
                year: 2026,
                month: 10,
            },
        );
        assert!(format_csv_events(&[month]).is_err());

        let mut properties = simple_event(
            "Properties",
            "properties",
            TimeSpec::DateOnly {
                start: NaiveDate::from_ymd_opt(2026, 10, 7).expect("date"),
                end_exclusive: None,
            },
        );
        properties.properties = serde_json::json!({"not": "representable"});
        assert!(format_csv_events(&[properties]).is_err());
    }

    #[test]
    fn csv_file_import_records_durable_success_and_failure_attempts() {
        let store = TemporalStore::open_in_memory().expect("store");
        let directory = tempfile::tempdir().expect("tempdir");
        let valid = directory.path().join("valid.csv");
        let invalid = directory.path().join("invalid.csv");
        let raw = concat!(
            "schema_version,record_key,title,raw_title,description,event_type,domain,jurisdiction,institution,status,confidence,importance,personal_relevance,upstream_event_ref,upstream_reconciled_key,renderability,time_kind,start,end,source_timezone\n",
            "1,a,Alpha,,,,,,,scheduled,,,,,,,date_only,2026-10-07,,\n"
        );
        std::fs::write(&valid, raw).expect("write valid CSV");
        std::fs::write(&invalid, "not,a,valid,ephemeris,csv\n").expect("write invalid CSV");

        import_csv_file(&store, &valid).expect("valid import");
        assert!(import_csv_file(&store, &invalid).is_err());

        let attempts = store.source_refresh_attempts(10).expect("refresh attempts");
        assert_eq!(attempts.len(), 2);
        assert!(attempts.iter().all(|attempt| attempt.refresh_kind == "csv_file"));
        assert!(attempts.iter().any(|attempt| attempt.success == Some(true)));
        assert!(attempts.iter().any(|attempt| attempt.success == Some(false)));
        assert!(attempts
            .iter()
            .filter(|attempt| attempt.success == Some(true))
            .all(|attempt| attempt.summary.as_deref().is_some_and(|summary| summary.contains("1 events"))));
    }

    #[test]
    fn csv_file_import_and_source_export_are_atomic_and_reparseable() {
        let store = TemporalStore::open_in_memory().expect("store");
        let directory = tempfile::tempdir().expect("tempdir");
        let input = directory.path().join("events.csv");
        let output = directory.path().join("events-export.csv");
        let raw = concat!(
            "schema_version,record_key,title,raw_title,description,event_type,domain,jurisdiction,institution,status,confidence,importance,personal_relevance,upstream_event_ref,upstream_reconciled_key,renderability,time_kind,start,end,source_timezone\n",
            "1,a,Alpha,,,,,,,scheduled,,,,,,,date_only,2026-10-07,,\n"
        );
        std::fs::write(&input, raw).expect("write CSV");

        let imported = import_csv_file(&store, &input).expect("import CSV");
        let exported =
            export_source_csv_by_id(&store, imported.source_id, &output).expect("export CSV");
        assert_eq!(exported.total_events, 1);
        let reparsed = parse_csv_events(&std::fs::read_to_string(&output).expect("read export"))
            .expect("parse export");
        assert_eq!(reparsed.len(), 1);
        assert_eq!(reparsed[0].normalized_title, "Alpha");
    }
}
