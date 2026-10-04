use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use chrono::{DateTime, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use rusqlite::{Connection, Row, named_params, params};
use serde_json::Value;
use uuid::Uuid;

use crate::domain::{
    EventStatus, SourceAuthority, SourceKind, TemporalEvent, TemporalSource, TimeSpec,
};

const SCHEMA_VERSION: i64 = 1;

pub struct TemporalStore {
    conn: Connection,
    path: Option<PathBuf>,
}

impl TemporalStore {
    pub fn open_default() -> anyhow::Result<Self> {
        let root = default_data_dir()?;
        std::fs::create_dir_all(&root)
            .with_context(|| format!("failed to create {}", root.display()))?;
        Self::open(root.join("ephemeris.sqlite3"))
    }

    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let mut conn = Connection::open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        configure_connection(&conn)?;
        migrate(&mut conn)?;

        Ok(Self {
            conn,
            path: Some(path),
        })
    }

    pub fn open_in_memory() -> anyhow::Result<Self> {
        let mut conn = Connection::open_in_memory().context("failed to open in-memory sqlite")?;
        configure_connection(&conn)?;
        migrate(&mut conn)?;
        Ok(Self { conn, path: None })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn schema_version(&self) -> anyhow::Result<i64> {
        self.conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .context("failed to read sqlite schema version")
    }

    pub fn event_count(&self) -> anyhow::Result<u64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM temporal_events", [], |row| row.get(0))
            .context("failed to count temporal events")?;
        u64::try_from(count).context("event count cannot be represented as u64")
    }

    pub fn upsert_source(&self, source: &TemporalSource) -> anyhow::Result<()> {
        self.conn
            .execute(
                r#"
            INSERT INTO temporal_sources (
                id, name, publisher, authority, kind, locator,
                enabled, read_only, created_at, updated_at
            ) VALUES (
                :id, :name, :publisher, :authority, :kind, :locator,
                :enabled, :read_only, :created_at, :updated_at
            )
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                publisher = excluded.publisher,
                authority = excluded.authority,
                kind = excluded.kind,
                locator = excluded.locator,
                enabled = excluded.enabled,
                read_only = excluded.read_only,
                updated_at = excluded.updated_at
            "#,
                named_params! {
                    ":id": source.id.to_string(),
                    ":name": source.name,
                    ":publisher": source.publisher,
                    ":authority": source.authority.as_str(),
                    ":kind": source.kind.as_str(),
                    ":locator": source.locator,
                    ":enabled": source.enabled,
                    ":read_only": source.read_only,
                    ":created_at": source.created_at.to_rfc3339(),
                    ":updated_at": source.updated_at.to_rfc3339(),
                },
            )
            .context("failed to upsert temporal source")?;
        Ok(())
    }

    pub fn list_sources(&self) -> anyhow::Result<Vec<TemporalSource>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, name, publisher, authority, kind, locator,
                   enabled, read_only, created_at, updated_at
            FROM temporal_sources
            ORDER BY name COLLATE NOCASE
            "#,
        )?;

        let mut rows = stmt.query([])?;
        let mut sources = Vec::new();
        while let Some(row) = rows.next()? {
            sources.push(decode_source(row)?);
        }
        Ok(sources)
    }

    pub fn upsert_event(&self, event: &TemporalEvent) -> anyhow::Result<()> {
        let encoded = EncodedTime::from_time_spec(&event.time);
        let tags_json =
            serde_json::to_string(&event.tags).context("failed to encode event tags")?;
        let properties_json = serde_json::to_string(&event.properties)
            .context("failed to encode event properties")?;

        self.conn
            .execute(
                r#"
            INSERT INTO temporal_events (
                id, source_id, source_record_key,
                normalized_title, raw_title, description,
                event_type, domain, jurisdiction, institution,
                status, confidence, importance, personal_relevance,
                time_kind, start_utc, end_utc, source_timezone,
                start_date, end_date_exclusive,
                start_local, end_local,
                tags_json, properties_json,
                created_at, updated_at
            ) VALUES (
                :id, :source_id, :source_record_key,
                :normalized_title, :raw_title, :description,
                :event_type, :domain, :jurisdiction, :institution,
                :status, :confidence, :importance, :personal_relevance,
                :time_kind, :start_utc, :end_utc, :source_timezone,
                :start_date, :end_date_exclusive,
                :start_local, :end_local,
                :tags_json, :properties_json,
                :created_at, :updated_at
            )
            ON CONFLICT(id) DO UPDATE SET
                source_id = excluded.source_id,
                source_record_key = excluded.source_record_key,
                normalized_title = excluded.normalized_title,
                raw_title = excluded.raw_title,
                description = excluded.description,
                event_type = excluded.event_type,
                domain = excluded.domain,
                jurisdiction = excluded.jurisdiction,
                institution = excluded.institution,
                status = excluded.status,
                confidence = excluded.confidence,
                importance = excluded.importance,
                personal_relevance = excluded.personal_relevance,
                time_kind = excluded.time_kind,
                start_utc = excluded.start_utc,
                end_utc = excluded.end_utc,
                source_timezone = excluded.source_timezone,
                start_date = excluded.start_date,
                end_date_exclusive = excluded.end_date_exclusive,
                start_local = excluded.start_local,
                end_local = excluded.end_local,
                tags_json = excluded.tags_json,
                properties_json = excluded.properties_json,
                updated_at = excluded.updated_at
            "#,
                named_params! {
                    ":id": event.id.to_string(),
                    ":source_id": event.source_id.map(|value| value.to_string()),
                    ":source_record_key": event.source_record_key,
                    ":normalized_title": event.normalized_title,
                    ":raw_title": event.raw_title,
                    ":description": event.description,
                    ":event_type": event.event_type,
                    ":domain": event.domain,
                    ":jurisdiction": event.jurisdiction,
                    ":institution": event.institution,
                    ":status": event.status.as_str(),
                    ":confidence": event.confidence,
                    ":importance": event.importance,
                    ":personal_relevance": event.personal_relevance,
                    ":time_kind": encoded.kind,
                    ":start_utc": encoded.start_utc,
                    ":end_utc": encoded.end_utc,
                    ":source_timezone": encoded.source_timezone,
                    ":start_date": encoded.start_date,
                    ":end_date_exclusive": encoded.end_date_exclusive,
                    ":start_local": encoded.start_local,
                    ":end_local": encoded.end_local,
                    ":tags_json": tags_json,
                    ":properties_json": properties_json,
                    ":created_at": event.created_at.to_rfc3339(),
                    ":updated_at": event.updated_at.to_rfc3339(),
                },
            )
            .context("failed to upsert temporal event")?;

        Ok(())
    }

    pub fn event_by_id(&self, id: Uuid) -> anyhow::Result<Option<TemporalEvent>> {
        let mut stmt = self.conn.prepare(&event_select_sql("WHERE id = ?1"))?;
        let mut rows = stmt.query(params![id.to_string()])?;
        match rows.next()? {
            Some(row) => Ok(Some(decode_event(row)?)),
            None => Ok(None),
        }
    }

    pub fn events_in_window(
        &self,
        start: NaiveDate,
        end_exclusive: NaiveDate,
        timezone: Tz,
    ) -> anyhow::Result<Vec<TemporalEvent>> {
        if end_exclusive <= start {
            return Ok(Vec::new());
        }

        let start_utc = date_boundary_utc(start, timezone)?;
        let end_utc = date_boundary_utc(end_exclusive, timezone)?;
        let start_local = start
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| anyhow!("invalid local start boundary"))?;
        let end_local = end_exclusive
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| anyhow!("invalid local end boundary"))?;

        let where_clause = r#"
            WHERE
                (
                    time_kind = 'all_day'
                    AND start_date < ?1
                    AND (
                        (end_date_exclusive IS NULL AND start_date >= ?2)
                        OR (end_date_exclusive IS NOT NULL AND end_date_exclusive > ?2)
                    )
                )
                OR
                (
                    time_kind = 'instant'
                    AND (
                        (start_utc >= ?3 AND start_utc < ?4)
                        OR (
                            end_utc IS NOT NULL
                            AND start_utc < ?4
                            AND end_utc > ?3
                        )
                    )
                )
                OR
                (
                    time_kind = 'floating'
                    AND (
                        (start_local >= ?5 AND start_local < ?6)
                        OR (
                            end_local IS NOT NULL
                            AND start_local < ?6
                            AND end_local > ?5
                        )
                    )
                )
            ORDER BY
                COALESCE(start_date, start_local, start_utc),
                normalized_title COLLATE NOCASE
        "#;

        let sql = event_select_sql(where_clause);
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(params![
            end_exclusive.format("%Y-%m-%d").to_string(),
            start.format("%Y-%m-%d").to_string(),
            start_utc.to_rfc3339(),
            end_utc.to_rfc3339(),
            format_naive(start_local),
            format_naive(end_local),
        ])?;

        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            events.push(decode_event(row)?);
        }

        events.sort_by(|left, right| {
            let left_date = left.display_date(timezone);
            let right_date = right.display_date(timezone);
            left_date
                .cmp(&right_date)
                .then_with(|| left.normalized_title.cmp(&right.normalized_title))
        });

        Ok(events)
    }
}

fn configure_connection(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        "#,
    )
    .context("failed to configure sqlite connection")
}

fn migrate(conn: &mut Connection) -> anyhow::Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if current > SCHEMA_VERSION {
        return Err(anyhow!(
            "database schema version {current} is newer than supported version {SCHEMA_VERSION}"
        ));
    }

    if current < 1 {
        let tx = conn
            .transaction()
            .context("failed to start schema migration")?;
        tx.execute_batch(
            r#"
            CREATE TABLE temporal_sources (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                publisher TEXT,
                authority TEXT NOT NULL,
                kind TEXT NOT NULL,
                locator TEXT,
                enabled INTEGER NOT NULL DEFAULT 1,
                read_only INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE temporal_events (
                id TEXT PRIMARY KEY,
                source_id TEXT REFERENCES temporal_sources(id) ON DELETE SET NULL,
                source_record_key TEXT,
                normalized_title TEXT NOT NULL,
                raw_title TEXT,
                description TEXT,
                event_type TEXT,
                domain TEXT,
                jurisdiction TEXT,
                institution TEXT,
                status TEXT NOT NULL,
                confidence REAL,
                importance INTEGER,
                personal_relevance INTEGER,

                time_kind TEXT NOT NULL
                    CHECK (time_kind IN ('all_day', 'instant', 'floating')),
                start_utc TEXT,
                end_utc TEXT,
                source_timezone TEXT,
                start_date TEXT,
                end_date_exclusive TEXT,
                start_local TEXT,
                end_local TEXT,

                tags_json TEXT NOT NULL DEFAULT '[]',
                properties_json TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,

                CHECK (
                    (time_kind = 'all_day' AND start_date IS NOT NULL)
                    OR (time_kind = 'instant' AND start_utc IS NOT NULL)
                    OR (time_kind = 'floating' AND start_local IS NOT NULL)
                )
            );

            CREATE UNIQUE INDEX temporal_events_source_record_key
                ON temporal_events(source_id, source_record_key)
                WHERE source_id IS NOT NULL AND source_record_key IS NOT NULL;

            CREATE INDEX temporal_events_start_date
                ON temporal_events(start_date)
                WHERE time_kind = 'all_day';

            CREATE INDEX temporal_events_start_utc
                ON temporal_events(start_utc)
                WHERE time_kind = 'instant';

            CREATE INDEX temporal_events_start_local
                ON temporal_events(start_local)
                WHERE time_kind = 'floating';

            CREATE INDEX temporal_events_source_id
                ON temporal_events(source_id);

            CREATE INDEX temporal_events_status
                ON temporal_events(status);

            CREATE INDEX temporal_events_domain
                ON temporal_events(domain);

            PRAGMA user_version = 1;
            "#,
        )
        .context("failed to create initial temporal schema")?;
        tx.commit().context("failed to commit schema migration")?;
    }

    Ok(())
}

fn event_select_sql(suffix: &str) -> String {
    format!(
        r#"
        SELECT
            id, source_id, source_record_key,
            normalized_title, raw_title, description,
            event_type, domain, jurisdiction, institution,
            status, confidence, importance, personal_relevance,
            time_kind, start_utc, end_utc, source_timezone,
            start_date, end_date_exclusive,
            start_local, end_local,
            tags_json, properties_json,
            created_at, updated_at
        FROM temporal_events
        {suffix}
        "#
    )
}

fn decode_source(row: &Row<'_>) -> anyhow::Result<TemporalSource> {
    Ok(TemporalSource {
        id: parse_uuid(row.get::<_, String>("id")?)?,
        name: row.get("name")?,
        publisher: row.get("publisher")?,
        authority: SourceAuthority::parse(&row.get::<_, String>("authority")?),
        kind: SourceKind::parse(&row.get::<_, String>("kind")?),
        locator: row.get("locator")?,
        enabled: row.get("enabled")?,
        read_only: row.get("read_only")?,
        created_at: parse_datetime(&row.get::<_, String>("created_at")?)?,
        updated_at: parse_datetime(&row.get::<_, String>("updated_at")?)?,
    })
}

fn decode_event(row: &Row<'_>) -> anyhow::Result<TemporalEvent> {
    let time_kind: String = row.get("time_kind")?;
    let time = match time_kind.as_str() {
        "all_day" => TimeSpec::AllDay {
            start: parse_date(&required_text(row, "start_date")?)?,
            end_exclusive: optional_text(row, "end_date_exclusive")?
                .map(|raw| parse_date(&raw))
                .transpose()?,
        },
        "instant" => TimeSpec::Instant {
            start_utc: parse_datetime(&required_text(row, "start_utc")?)?,
            end_utc: optional_text(row, "end_utc")?
                .map(|raw| parse_datetime(&raw))
                .transpose()?,
            source_timezone: row.get("source_timezone")?,
        },
        "floating" => TimeSpec::Floating {
            start: parse_naive_datetime(&required_text(row, "start_local")?)?,
            end: optional_text(row, "end_local")?
                .map(|raw| parse_naive_datetime(&raw))
                .transpose()?,
        },
        other => return Err(anyhow!("unknown time kind {other}")),
    };

    let status_raw: String = row.get("status")?;
    let status = EventStatus::parse(&status_raw)
        .ok_or_else(|| anyhow!("unknown event status {status_raw}"))?;

    let tags_raw: String = row.get("tags_json")?;
    let tags: Vec<String> =
        serde_json::from_str(&tags_raw).context("failed to decode event tags")?;

    let properties_raw: String = row.get("properties_json")?;
    let properties: Value =
        serde_json::from_str(&properties_raw).context("failed to decode event properties")?;

    Ok(TemporalEvent {
        id: parse_uuid(row.get::<_, String>("id")?)?,
        source_id: optional_text(row, "source_id")?
            .map(|raw| parse_uuid(raw))
            .transpose()?,
        source_record_key: row.get("source_record_key")?,
        normalized_title: row.get("normalized_title")?,
        raw_title: row.get("raw_title")?,
        description: row.get("description")?,
        event_type: row.get("event_type")?,
        domain: row.get("domain")?,
        jurisdiction: row.get("jurisdiction")?,
        institution: row.get("institution")?,
        status,
        confidence: row.get("confidence")?,
        importance: row.get("importance")?,
        personal_relevance: row.get("personal_relevance")?,
        time,
        tags,
        properties,
        created_at: parse_datetime(&row.get::<_, String>("created_at")?)?,
        updated_at: parse_datetime(&row.get::<_, String>("updated_at")?)?,
    })
}

fn required_text(row: &Row<'_>, column: &str) -> anyhow::Result<String> {
    optional_text(row, column)?.ok_or_else(|| anyhow!("missing required column {column}"))
}

fn optional_text(row: &Row<'_>, column: &str) -> anyhow::Result<Option<String>> {
    Ok(row.get(column)?)
}

fn parse_uuid(raw: impl AsRef<str>) -> anyhow::Result<Uuid> {
    Uuid::parse_str(raw.as_ref()).with_context(|| format!("invalid uuid {}", raw.as_ref()))
}

fn parse_datetime(raw: &str) -> anyhow::Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(raw)
        .with_context(|| format!("invalid RFC3339 datetime {raw}"))?
        .with_timezone(&Utc))
}

fn parse_date(raw: &str) -> anyhow::Result<NaiveDate> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d").with_context(|| format!("invalid date {raw}"))
}

fn parse_naive_datetime(raw: &str) -> anyhow::Result<NaiveDateTime> {
    NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S")
        .with_context(|| format!("invalid floating datetime {raw}"))
}

fn format_naive(value: NaiveDateTime) -> String {
    value.format("%Y-%m-%dT%H:%M:%S").to_string()
}

fn date_boundary_utc(day: NaiveDate, timezone: Tz) -> anyhow::Result<DateTime<Utc>> {
    let naive = day
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow!("invalid midnight for {day}"))?;
    match timezone.from_local_datetime(&naive) {
        LocalResult::Single(value) => Ok(value.with_timezone(&Utc)),
        LocalResult::Ambiguous(first, second) => Ok(first.min(second).with_timezone(&Utc)),
        LocalResult::None => Err(anyhow!(
            "local date boundary {day} does not exist in timezone {timezone}"
        )),
    }
}

fn default_data_dir() -> anyhow::Result<PathBuf> {
    if let Some(path) = dirs::data_local_dir() {
        return Ok(path.join("ephemeris"));
    }
    Ok(std::env::current_dir()
        .context("failed to resolve current directory")?
        .join(".ephemeris"))
}

struct EncodedTime {
    kind: &'static str,
    start_utc: Option<String>,
    end_utc: Option<String>,
    source_timezone: Option<String>,
    start_date: Option<String>,
    end_date_exclusive: Option<String>,
    start_local: Option<String>,
    end_local: Option<String>,
}

impl EncodedTime {
    fn from_time_spec(time: &TimeSpec) -> Self {
        match time {
            TimeSpec::AllDay {
                start,
                end_exclusive,
            } => Self {
                kind: "all_day",
                start_utc: None,
                end_utc: None,
                source_timezone: None,
                start_date: Some(start.format("%Y-%m-%d").to_string()),
                end_date_exclusive: end_exclusive.map(|value| value.format("%Y-%m-%d").to_string()),
                start_local: None,
                end_local: None,
            },
            TimeSpec::Instant {
                start_utc,
                end_utc,
                source_timezone,
            } => Self {
                kind: "instant",
                start_utc: Some(start_utc.to_rfc3339()),
                end_utc: end_utc.map(|value| value.to_rfc3339()),
                source_timezone: source_timezone.clone(),
                start_date: None,
                end_date_exclusive: None,
                start_local: None,
                end_local: None,
            },
            TimeSpec::Floating { start, end } => Self {
                kind: "floating",
                start_utc: None,
                end_utc: None,
                source_timezone: None,
                start_date: None,
                end_date_exclusive: None,
                start_local: Some(format_naive(*start)),
                end_local: end.map(format_naive),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{SourceAuthority, SourceKind};

    #[test]
    fn schema_bootstraps_at_version_one() {
        let store = TemporalStore::open_in_memory().expect("store");
        assert_eq!(store.schema_version().expect("version"), 1);
    }

    #[test]
    fn window_query_preserves_three_time_kinds() {
        let store = TemporalStore::open_in_memory().expect("store");
        let source =
            TemporalSource::new("Test source", SourceKind::Taria, SourceAuthority::Official);
        store.upsert_source(&source).expect("source");

        let day = NaiveDate::from_ymd_opt(2026, 10, 4).expect("date");

        let mut all_day = TemporalEvent::new(
            "All day",
            TimeSpec::AllDay {
                start: day,
                end_exclusive: None,
            },
        );
        all_day.source_id = Some(source.id);
        store.upsert_event(&all_day).expect("all-day event");

        let instant_start = DateTime::parse_from_rfc3339("2026-10-05T01:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        let mut instant = TemporalEvent::new(
            "Instant",
            TimeSpec::Instant {
                start_utc: instant_start,
                end_utc: None,
                source_timezone: Some("UTC".to_string()),
            },
        );
        instant.source_id = Some(source.id);
        store.upsert_event(&instant).expect("instant event");

        let floating_start =
            NaiveDateTime::parse_from_str("2026-10-04T15:00:00", "%Y-%m-%dT%H:%M:%S")
                .expect("floating");
        let mut floating = TemporalEvent::new(
            "Floating",
            TimeSpec::Floating {
                start: floating_start,
                end: None,
            },
        );
        floating.source_id = Some(source.id);
        store.upsert_event(&floating).expect("floating event");

        let next_day = day.succ_opt().expect("next day");
        let events = store
            .events_in_window(day, next_day, chrono_tz::America::Mexico_City)
            .expect("window query");

        assert_eq!(events.len(), 3);
    }

    #[test]
    fn source_record_identity_is_unique_per_source() {
        let store = TemporalStore::open_in_memory().expect("store");
        let source = TemporalSource::new("Test source", SourceKind::Ics, SourceAuthority::Official);
        store.upsert_source(&source).expect("source");

        let day = NaiveDate::from_ymd_opt(2026, 10, 4).expect("date");
        let mut first = TemporalEvent::new(
            "One",
            TimeSpec::AllDay {
                start: day,
                end_exclusive: None,
            },
        );
        first.source_id = Some(source.id);
        first.source_record_key = Some("uid-1".to_string());
        store.upsert_event(&first).expect("first");

        let mut second = TemporalEvent::new(
            "Two",
            TimeSpec::AllDay {
                start: day,
                end_exclusive: None,
            },
        );
        second.source_id = Some(source.id);
        second.source_record_key = Some("uid-1".to_string());

        assert!(store.upsert_event(&second).is_err());
    }
}
