use std::path::Path;

use anyhow::{Context, anyhow};
use chrono::{DateTime, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::domain::{
    EventStatus, SourceAuthority, SourceKind, TemporalEvent, TemporalSource, TimeSpec,
};
use crate::store::TemporalStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TariaImportReport {
    pub projection_ref: String,
    pub reconciled_set_ref: Option<String>,
    pub source_id: Uuid,
    pub total_events: usize,
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub retained_missing: usize,
    pub imprecise: usize,
    pub unplaced: usize,
    pub blocked_or_undated: usize,
    pub retained_missing: usize,
    pub suggested_focus: Option<NaiveDate>,
}

pub fn import_reconciled_event_set_file(
    store: &TemporalStore,
    path: impl AsRef<Path>,
) -> anyhow::Result<TariaImportReport> {
    let path = path.as_ref();
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read Taria artifact {}", path.display()))?;
    import_reconciled_event_set_json(store, &raw, Some(&path.display().to_string()))
}

pub fn import_reconciled_event_set_json(
    store: &TemporalStore,
    raw: &str,
    locator: Option<&str>,
) -> anyhow::Result<TariaImportReport> {
    let root: Value =
        serde_json::from_str(raw).context("failed to decode Taria reconciled event set JSON")?;
    let object = root
        .as_object()
        .ok_or_else(|| anyhow!("Taria reconciled event set must be a JSON object"))?;

    let schema_version = object
        .get("schema_version")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    if schema_version != 1 {
        return Err(anyhow!(
            "unsupported Taria reconciled event set schema version {schema_version}"
        ));
    }

    let projection_ref = required_string(object, "projection_ref")?;
    let reconciled_set_ref = optional_string(object, "reconciled_projection_event_set_id");
    let events = object
        .get("events")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Taria reconciled event set is missing events[]"))?;
    let blocked_events = object
        .get("blocked_events")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    let mut source = match store.source_by_external_ref(&projection_ref)? {
        Some(source) => source,
        None => {
            let mut source = TemporalSource::new(
                projection_display_name(object, &projection_ref),
                SourceKind::Taria,
                SourceAuthority::Derived,
            );
            source.external_ref = Some(projection_ref.clone());
            source
        }
    };

    source.name = projection_display_name(object, &projection_ref);
    source.kind = SourceKind::Taria;
    source.authority = SourceAuthority::Derived;
    source.read_only = true;
    source.enabled = true;
    source.locator = locator.map(ToOwned::to_owned);
    source.updated_at = Utc::now();
    source.properties = source_properties(object);

    let mut normalized = Vec::with_capacity(events.len() + blocked_events.len());
    for raw_event in events {
        let event_object = raw_event
            .as_object()
            .ok_or_else(|| anyhow!("Taria events[] contains a non-object value"))?;
        let record_key = reconciled_record_key(event_object)?;
        normalized.push(normalized_event(
            event_object,
            source.id,
            &record_key,
            &projection_ref,
            reconciled_set_ref.as_deref(),
        )?);
    }

    for raw_event in blocked_events {
        let event_object = raw_event
            .as_object()
            .ok_or_else(|| anyhow!("Taria blocked_events[] contains a non-object value"))?;
        let record_key = reconciled_record_key(event_object)?;
        normalized.push(normalized_blocked_event(
            event_object,
            source.id,
            &record_key,
            &projection_ref,
            reconciled_set_ref.as_deref(),
        ));
    }

    let imprecise = normalized.iter().filter(|event| event.time.is_imprecise()).count();
    let unplaced = normalized
        .iter()
        .filter(|event| matches!(event.time, TimeSpec::Unknown { .. }))
        .count();
    let blocked_or_undated = normalized
        .iter()
        .filter(|event| {
            event
                .renderability
                .as_deref()
                .is_some_and(|state| state != "ready")
                || matches!(event.time, TimeSpec::Unknown { .. })
        })
        .count();
    let suggested_focus = normalized.iter().filter_map(event_focus_date).min();

    let batch = store.import_batch(&source, &mut normalized)?;

    Ok(TariaImportReport {
        projection_ref,
        reconciled_set_ref,
        source_id: source.id,
        total_events: normalized.len(),
        created: batch.created,
        updated: batch.updated,
        unchanged: batch.unchanged,
        imprecise,
        unplaced,
        blocked_or_undated,
        retained_missing: batch.retained_missing,
        suggested_focus,
    })
}

fn normalized_event(
    raw: &Map<String, Value>,
    source_id: Uuid,
    record_key: &str,
    projection_ref: &str,
    reconciled_set_ref: Option<&str>,
) -> anyhow::Result<TemporalEvent> {
    let display = raw
        .get("display_fields")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("Taria reconciled event {record_key} is missing display_fields"))?;

    let title = display
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or(record_key)
        .to_string();
    let time = parse_temporal_value(display.get("temporal_value"))?;
    let mut event = TemporalEvent::new(title, time);

    event.source_id = Some(source_id);
    event.source_record_key = Some(record_key.to_string());
    event.upstream_event_ref = optional_string(raw, "event_ref");
    event.upstream_reconciled_key = optional_string(raw, "reconciled_event_key");
    event.assertion_refs = string_array(raw.get("assertion_refs"));
    event.source_refs = {
        let refs = string_array(raw.get("source_refs"));
        if refs.is_empty() {
            source_refs_from_contexts(raw.get("source_contexts"))
        } else {
            refs
        }
    };
    event.provenance_refs = string_array(raw.get("retained_provenance_refs"));
    event.renderability = optional_string(raw, "renderability");
    event.description = optional_string(display, "description");
    event.event_type = optional_string(display, "event_class");
    event.domain = first_domain(raw.get("source_contexts"));
    event.jurisdiction = optional_string(display, "jurisdiction");
    event.institution = optional_string(display, "institution");
    event.status = display
        .get("schedule_status")
        .and_then(Value::as_str)
        .and_then(EventStatus::parse)
        .unwrap_or(EventStatus::Unknown);
    event.tags = string_array(display.get("categories"));

    let mut taria = Map::new();
    taria.insert(
        "raw_reconciled_event".to_string(),
        Value::Object(raw.clone()),
    );
    taria.insert(
        "projection_ref".to_string(),
        Value::String(projection_ref.to_string()),
    );
    if let Some(reconciled_set_ref) = reconciled_set_ref {
        taria.insert(
            "reconciled_projection_event_set_ref".to_string(),
            Value::String(reconciled_set_ref.to_string()),
        );
    }
    copy_if_present(raw, &mut taria, "grouping");
    copy_if_present(raw, &mut taria, "source_contexts");
    copy_if_present(raw, &mut taria, "field_resolutions");
    copy_if_present(raw, &mut taria, "presentation_directives");
    copy_if_present(raw, &mut taria, "render_blockers");
    if let Some(value) = display.get("geography") {
        taria.insert("geography".to_string(), value.clone());
    }
    if let Some(value) = display.get("jurisdiction_level") {
        taria.insert("jurisdiction_level".to_string(), value.clone());
    }
    if let Some(value) = display.get("competition_or_series") {
        taria.insert("competition_or_series".to_string(), value.clone());
    }
    if let Some(value) = display.get("source_subtype") {
        taria.insert("source_subtype".to_string(), value.clone());
    }
    if let Some(value) = display.get("temporal_value") {
        taria.insert("temporal_value".to_string(), value.clone());
    }
    event.properties = json!({ "taria": Value::Object(taria) });

    Ok(event)
}

fn reconciled_record_key(raw: &Map<String, Value>) -> anyhow::Result<String> {
    raw.get("reconciled_event_key")
        .and_then(Value::as_str)
        .or_else(|| raw.get("event_ref").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("Taria event is missing reconciled_event_key/event_ref"))
}

fn normalized_blocked_event(
    raw: &Map<String, Value>,
    source_id: Uuid,
    record_key: &str,
    projection_ref: &str,
    reconciled_set_ref: Option<&str>,
) -> TemporalEvent {
    let title = raw
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| raw.get("event_ref").and_then(Value::as_str))
        .unwrap_or(record_key);
    let mut event = TemporalEvent::new(
        title,
        TimeSpec::Unknown {
            original_value: None,
        },
    );
    event.source_id = Some(source_id);
    event.source_record_key = Some(record_key.to_string());
    event.upstream_event_ref = optional_string(raw, "event_ref");
    event.upstream_reconciled_key = optional_string(raw, "reconciled_event_key");
    event.renderability = optional_string(raw, "renderability")
        .or_else(|| Some("blocked".to_string()));
    event.status = EventStatus::Unknown;

    let mut taria = Map::new();
    taria.insert(
        "projection_ref".to_string(),
        Value::String(projection_ref.to_string()),
    );
    if let Some(reconciled_set_ref) = reconciled_set_ref {
        taria.insert(
            "reconciled_projection_event_set_ref".to_string(),
            Value::String(reconciled_set_ref.to_string()),
        );
    }
    taria.insert(
        "raw_blocked_event".to_string(),
        Value::Object(raw.clone()),
    );
    event.properties = json!({ "taria": Value::Object(taria) });
    event
}

fn event_focus_date(event: &TemporalEvent) -> Option<NaiveDate> {
    match event.time {
        TimeSpec::DateOnly { start, .. } | TimeSpec::AllDay { start, .. } => Some(start),
        TimeSpec::Instant { start_utc, .. } => Some(start_utc.date_naive()),
        TimeSpec::Floating { start, .. } => Some(start.date()),
        TimeSpec::Month { year, month } => NaiveDate::from_ymd_opt(year, month, 1),
        TimeSpec::Year { year } => NaiveDate::from_ymd_opt(year, 1, 1),
        TimeSpec::Unknown { .. } => None,
    }
}

fn parse_temporal_value(value: Option<&Value>) -> anyhow::Result<TimeSpec> {
    let Some(value) = value else {
        return Ok(TimeSpec::Unknown {
            original_value: None,
        });
    };
    let Some(object) = value.as_object() else {
        return Ok(TimeSpec::Unknown {
            original_value: Some(value.to_string()),
        });
    };

    let kind = object
        .get("kind")
        .or_else(|| object.get("value_kind"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .trim()
        .to_ascii_lowercase();

    match kind.as_str() {
        "date" | "date-only" | "date_only" => {
            let raw = temporal_start_string(object)?;
            let start = parse_date(&raw)?;
            let end_exclusive = object
                .get("end")
                .or_else(|| object.get("interval_end"))
                .and_then(Value::as_str)
                .map(parse_date)
                .transpose()?;
            Ok(TimeSpec::DateOnly {
                start,
                end_exclusive,
            })
        }
        "all-day-date" | "all_day_date" | "all-day" | "all_day" => {
            let raw = temporal_start_string(object)?;
            let start = parse_date(&raw)?;
            let end_exclusive = object
                .get("end")
                .or_else(|| object.get("interval_end"))
                .and_then(Value::as_str)
                .map(parse_date)
                .transpose()?;
            Ok(TimeSpec::AllDay {
                start,
                end_exclusive,
            })
        }
        "year" => {
            let year = object
                .get("year")
                .and_then(Value::as_i64)
                .ok_or_else(|| anyhow!("year-precision temporal value is missing year"))?;
            Ok(TimeSpec::Year {
                year: i32::try_from(year).context("year is outside i32 range")?,
            })
        }
        "month" => {
            let year = object
                .get("year")
                .and_then(Value::as_i64)
                .ok_or_else(|| anyhow!("month-precision temporal value is missing year"))?;
            let month = object
                .get("month")
                .and_then(Value::as_u64)
                .ok_or_else(|| anyhow!("month-precision temporal value is missing month"))?;
            let year = i32::try_from(year).context("year is outside i32 range")?;
            let month = u32::try_from(month).context("month is outside u32 range")?;
            if !(1..=12).contains(&month) {
                return Err(anyhow!("invalid month precision value {year}-{month:02}"));
            }
            Ok(TimeSpec::Month { year, month })
        }
        "date-time" | "date_time" | "datetime" | "instant" => parse_datetime_temporal_value(object),
        "local-datetime" | "local_datetime" => parse_local_datetime_temporal_value(object),
        "interval" => Ok(TimeSpec::Unknown {
            original_value: Some(Value::Object(object.clone()).to_string()),
        }),
        _ => Ok(TimeSpec::Unknown {
            original_value: object
                .get("original_value")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| Some(Value::Object(object.clone()).to_string())),
        }),
    }
}

fn parse_datetime_temporal_value(object: &Map<String, Value>) -> anyhow::Result<TimeSpec> {
    let raw = temporal_start_string(object)?;
    let end_raw = object
        .get("end")
        .or_else(|| object.get("interval_end"))
        .and_then(Value::as_str);
    let source_timezone = object
        .get("timezone")
        .or_else(|| object.get("timezone_id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    if let Ok(parsed) = DateTime::parse_from_rfc3339(&raw) {
        let end_utc = end_raw
            .map(DateTime::parse_from_rfc3339)
            .transpose()
            .with_context(|| format!("invalid RFC3339 temporal end for {raw}"))?
            .map(|value| value.with_timezone(&Utc));
        return Ok(TimeSpec::Instant {
            start_utc: parsed.with_timezone(&Utc),
            end_utc,
            source_timezone,
        });
    }

    let naive = parse_naive_datetime(&raw)?;
    if let Some(timezone_id) = source_timezone.as_deref()
        && let Ok(timezone) = timezone_id.parse::<Tz>()
    {
        let start_utc = local_to_utc(timezone, naive)?;
        let end_utc = end_raw
            .map(parse_naive_datetime)
            .transpose()?
            .map(|end| local_to_utc(timezone, end))
            .transpose()?;
        return Ok(TimeSpec::Instant {
            start_utc,
            end_utc,
            source_timezone,
        });
    }

    Ok(TimeSpec::Floating {
        start: naive,
        end: end_raw.map(parse_naive_datetime).transpose()?,
        source_timezone,
    })
}

fn parse_local_datetime_temporal_value(object: &Map<String, Value>) -> anyhow::Result<TimeSpec> {
    let raw = temporal_start_string(object)?;
    let clock_basis = object
        .get("clock_basis")
        .and_then(Value::as_str)
        .unwrap_or("floating-local");
    if clock_basis == "timezone-unknown" {
        return Ok(TimeSpec::Unknown {
            original_value: Some(raw),
        });
    }

    let start = parse_naive_datetime(&raw)?;
    let end = object
        .get("end")
        .or_else(|| object.get("interval_end"))
        .and_then(Value::as_str)
        .map(parse_naive_datetime)
        .transpose()?;
    let source_timezone = object
        .get("timezone")
        .or_else(|| object.get("timezone_id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    Ok(TimeSpec::Floating {
        start,
        end,
        source_timezone,
    })
}

fn temporal_start_string(object: &Map<String, Value>) -> anyhow::Result<String> {
    object
        .get("start")
        .or_else(|| object.get("original_value"))
        .or_else(|| object.get("normalized_value"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("temporal value is missing a string start/original_value"))
}

fn parse_date(raw: &str) -> anyhow::Result<NaiveDate> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d").with_context(|| format!("invalid Taria date {raw}"))
}

fn parse_naive_datetime(raw: &str) -> anyhow::Result<NaiveDateTime> {
    for format in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(value) = NaiveDateTime::parse_from_str(raw, format) {
            return Ok(value);
        }
    }
    Err(anyhow!("invalid Taria local datetime {raw}"))
}

fn local_to_utc(timezone: Tz, value: NaiveDateTime) -> anyhow::Result<DateTime<Utc>> {
    match timezone.from_local_datetime(&value) {
        LocalResult::Single(value) => Ok(value.with_timezone(&Utc)),
        LocalResult::Ambiguous(first, second) => Ok(first.min(second).with_timezone(&Utc)),
        LocalResult::None => Err(anyhow!(
            "Taria local datetime {value} does not exist in timezone {timezone}"
        )),
    }
}

fn source_properties(root: &Map<String, Value>) -> Value {
    let mut properties = Map::new();
    for key in [
        "schema_version",
        "reconciled_projection_event_set_id",
        "generated_at",
        "projection_ref",
        "input_projection_event_set_ref",
        "input_projection_content_sha256",
        "reconciliation_policy",
        "reconciliation_policy_sha256",
        "presentation_policy",
        "presentation_policy_sha256",
        "calendar_set_policy",
        "content_fingerprint",
        "summary",
        "requested_outputs",
        "sync_policy",
    ] {
        copy_if_present(root, &mut properties, key);
    }
    json!({ "taria": Value::Object(properties) })
}

fn projection_display_name(root: &Map<String, Value>, projection_ref: &str) -> String {
    if let Some(name) = root
        .get("calendar_set_policy")
        .and_then(Value::as_object)
        .and_then(|policy| policy.get("merged_calendar_name"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        return name.to_string();
    }

    projection_ref
        .strip_prefix("projection:")
        .unwrap_or(projection_ref)
        .replace(['-', '_'], " ")
        .split_whitespace()
        .map(capitalize_token)
        .collect::<Vec<_>>()
        .join(" ")
}

fn capitalize_token(token: &str) -> String {
    if token.chars().all(|ch| ch.is_ascii_digit()) {
        return token.to_string();
    }
    let mut chars = token.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn required_string(object: &Map<String, Value>, key: &str) -> anyhow::Result<String> {
    optional_string(object, key).ok_or_else(|| anyhow!("missing required string field {key}"))
}

fn optional_string(object: &Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn first_domain(source_contexts: Option<&Value>) -> Option<String> {
    source_contexts
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter_map(|context| context.get("source_facets"))
        .filter_map(Value::as_object)
        .filter_map(|facets| facets.get("domains"))
        .filter_map(Value::as_array)
        .flatten()
        .find_map(Value::as_str)
        .map(ToOwned::to_owned)
}

fn source_refs_from_contexts(source_contexts: Option<&Value>) -> Vec<String> {
    source_contexts
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter_map(|context| context.get("resource_ref"))
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn copy_if_present(source: &Map<String, Value>, target: &mut Map<String, Value>, key: &str) {
    if let Some(value) = source.get(key) {
        target.insert(key.to_string(), value.clone());
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    const FIXTURE: &str = r#"
    {
      "schema_version": 1,
      "reconciled_projection_event_set_id": "reconciled-projection-event-set:fixture",
      "generated_at": "2026-09-30T00:00:00Z",
      "projection_ref": "projection:fixture-calendar",
      "calendar_set_policy": {
        "merged_calendar_name": "Fixture Calendar"
      },
      "events": [
        {
          "reconciled_event_key": "reconciled-event:date",
          "event_ref": "event:date",
          "assertion_refs": ["assertion:date"],
          "source_refs": ["resource:date"],
          "retained_provenance_refs": ["trace:date"],
          "renderability": "ready",
          "display_fields": {
            "title": "Known Date",
            "temporal_value": {
              "kind": "date",
              "precision": "date",
              "start": "2027-10-24"
            },
            "schedule_status": "confirmed",
            "event_class": "election",
            "jurisdiction": "CH",
            "categories": ["elections"]
          }
        },
        {
          "reconciled_event_key": "reconciled-event:month",
          "event_ref": "event:month",
          "renderability": "ready",
          "display_fields": {
            "title": "Known Month",
            "temporal_value": {
              "kind": "month",
              "precision": "month",
              "year": 2027,
              "month": 11
            },
            "schedule_status": "tentative"
          }
        },
        {
          "reconciled_event_key": "reconciled-event:year",
          "event_ref": "event:year",
          "renderability": "ready",
          "display_fields": {
            "title": "Known Year",
            "temporal_value": {
              "kind": "year",
              "precision": "year",
              "year": 2027
            },
            "schedule_status": "tentative"
          }
        },
        {
          "reconciled_event_key": "reconciled-event:blocked",
          "event_ref": "event:blocked",
          "renderability": "blocked-temporal-conflict",
          "render_blockers": ["temporal-conflict"],
          "display_fields": {
            "title": "Blocked Event",
            "event_class": "meeting"
          }
        }
      ]
    }
    "#;

    #[test]
    fn imports_real_reconciled_shape_without_inventing_dates() {
        let store = TemporalStore::open_in_memory().expect("store");
        let report = import_reconciled_event_set_json(&store, FIXTURE, Some("fixture.json"))
            .expect("import");

        assert_eq!(report.total_events, 4);
        assert_eq!(report.created, 4);
        assert_eq!(report.imprecise, 3);
        assert_eq!(report.unplaced, 1);
        assert_eq!(report.blocked_or_undated, 1);
        assert_eq!(store.event_count().expect("count"), 4);
        assert_eq!(store.unplaced_event_count().expect("unplaced"), 1);

        let october_start = NaiveDate::from_ymd_opt(2027, 10, 1).expect("start");
        let december_start = NaiveDate::from_ymd_opt(2027, 12, 1).expect("end");
        let visible = store
            .events_in_window(october_start, december_start, chrono_tz::UTC, true)
            .expect("events");
        assert_eq!(visible.len(), 3);
        assert!(
            visible
                .iter()
                .any(|event| matches!(event.time, TimeSpec::DateOnly { .. }))
        );
        assert!(
            visible
                .iter()
                .any(|event| matches!(event.time, TimeSpec::Month { .. }))
        );
        assert!(
            visible
                .iter()
                .any(|event| matches!(event.time, TimeSpec::Year { .. }))
        );
    }

    #[test]
    fn reimport_updates_same_upstream_event_in_place() {
        let store = TemporalStore::open_in_memory().expect("store");
        let first = import_reconciled_event_set_json(&store, FIXTURE, None).expect("first import");
        assert_eq!(first.created, 4);

        let changed = FIXTURE.replace("Known Date", "Known Date Updated");
        let second =
            import_reconciled_event_set_json(&store, &changed, None).expect("second import");
        assert_eq!(second.created, 0);
        assert_eq!(second.updated, 1);
        assert_eq!(second.unchanged, 3);
        assert_eq!(store.event_count().expect("count"), 4);
    }

    #[test]
    fn parses_offset_datetime_as_instant() {
        let value = json!({
            "kind": "date-time",
            "start": "2026-03-25T16:00:00-04:00",
            "timezone": "America/New_York",
            "precision": "datetime"
        });
        let parsed = parse_temporal_value(Some(&value)).expect("parse");

        match parsed {
            TimeSpec::Instant {
                start_utc,
                source_timezone,
                ..
            } => {
                assert_eq!(start_utc.to_rfc3339(), "2026-03-25T20:00:00+00:00");
                assert_eq!(source_timezone.as_deref(), Some("America/New_York"));
            }
            other => panic!("expected instant, got {other:?}"),
        }
    }
}
