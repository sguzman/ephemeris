use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use chrono::{DateTime, Datelike, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use rusqlite::{Connection, OptionalExtension, Row, named_params, params};
use serde_json::Value;
use uuid::Uuid;

use crate::domain::{
    EventStatus, SourceAuthority, SourceKind, TemporalEvent, TemporalSource, TimeSpec,
};

const SCHEMA_VERSION: i64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportBatchResult {
    pub created: usize,
    pub updated: usize,
    pub retained_missing: usize,
}

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

    pub fn unplaced_event_count(&self) -> anyhow::Result<u64> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM temporal_events WHERE time_kind = 'unknown'",
                [],
                |row| row.get(0),
            )
            .context("failed to count unplaced temporal events")?;
        u64::try_from(count).context("unplaced event count cannot be represented as u64")
    }

    pub fn upsert_source(&self, source: &TemporalSource) -> anyhow::Result<()> {
        let properties_json = serde_json::to_string(&source.properties)
            .context("failed to encode source properties")?;

        self.conn
            .execute(
                r#"
                INSERT INTO temporal_sources (
                    id, external_ref, name, publisher, authority, kind, locator,
                    enabled, read_only, properties_json, created_at, updated_at
                ) VALUES (
                    :id, :external_ref, :name, :publisher, :authority, :kind, :locator,
                    :enabled, :read_only, :properties_json, :created_at, :updated_at
                )
                ON CONFLICT(id) DO UPDATE SET
                    external_ref = excluded.external_ref,
                    name = excluded.name,
                    publisher = excluded.publisher,
                    authority = excluded.authority,
                    kind = excluded.kind,
                    locator = excluded.locator,
                    enabled = excluded.enabled,
                    read_only = excluded.read_only,
                    properties_json = excluded.properties_json,
                    updated_at = excluded.updated_at
                "#,
                named_params! {
                    ":id": source.id.to_string(),
                    ":external_ref": source.external_ref,
                    ":name": source.name,
                    ":publisher": source.publisher,
                    ":authority": source.authority.as_str(),
                    ":kind": source.kind.as_str(),
                    ":locator": source.locator,
                    ":enabled": source.enabled,
                    ":read_only": source.read_only,
                    ":properties_json": properties_json,
                    ":created_at": source.created_at.to_rfc3339(),
                    ":updated_at": source.updated_at.to_rfc3339(),
                },
            )
            .context("failed to upsert temporal source")?;
        Ok(())
    }

    pub fn source_by_external_ref(
        &self,
        external_ref: &str,
    ) -> anyhow::Result<Option<TemporalSource>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, external_ref, name, publisher, authority, kind, locator,
                   enabled, read_only, properties_json, created_at, updated_at
            FROM temporal_sources
            WHERE external_ref = ?1
            "#,
        )?;

        stmt.query_row(params![external_ref], decode_source)
            .optional()
            .context("failed to query temporal source by external ref")
    }

    pub fn list_sources(&self) -> anyhow::Result<Vec<TemporalSource>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, external_ref, name, publisher, authority, kind, locator,
                   enabled, read_only, properties_json, created_at, updated_at
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
        let encoded = EncodedTime::from_time_spec(&event.time)?;
        let assertion_refs_json = encode_string_vec(&event.assertion_refs, "assertion refs")?;
        let source_refs_json = encode_string_vec(&event.source_refs, "source refs")?;
        let provenance_refs_json = encode_string_vec(&event.provenance_refs, "provenance refs")?;
        let tags_json = encode_string_vec(&event.tags, "event tags")?;
        let properties_json = serde_json::to_string(&event.properties)
            .context("failed to encode event properties")?;

        self.conn
            .execute(
                r#"
                INSERT INTO temporal_events (
                    id, source_id, source_record_key,
                    upstream_event_ref, upstream_reconciled_key,
                    assertion_refs_json, source_refs_json, provenance_refs_json, renderability,
                    normalized_title, raw_title, description,
                    event_type, domain, jurisdiction, institution,
                    status, confidence, importance, personal_relevance,
                    time_kind, start_utc, end_utc, source_timezone,
                    start_date, end_date_exclusive,
                    start_local, end_local, time_original_value,
                    tags_json, properties_json,
                    created_at, updated_at
                ) VALUES (
                    :id, :source_id, :source_record_key,
                    :upstream_event_ref, :upstream_reconciled_key,
                    :assertion_refs_json, :source_refs_json, :provenance_refs_json, :renderability,
                    :normalized_title, :raw_title, :description,
                    :event_type, :domain, :jurisdiction, :institution,
                    :status, :confidence, :importance, :personal_relevance,
                    :time_kind, :start_utc, :end_utc, :source_timezone,
                    :start_date, :end_date_exclusive,
                    :start_local, :end_local, :time_original_value,
                    :tags_json, :properties_json,
                    :created_at, :updated_at
                )
                ON CONFLICT(id) DO UPDATE SET
                    source_id = excluded.source_id,
                    source_record_key = excluded.source_record_key,
                    upstream_event_ref = excluded.upstream_event_ref,
                    upstream_reconciled_key = excluded.upstream_reconciled_key,
                    assertion_refs_json = excluded.assertion_refs_json,
                    source_refs_json = excluded.source_refs_json,
                    provenance_refs_json = excluded.provenance_refs_json,
                    renderability = excluded.renderability,
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
                    time_original_value = excluded.time_original_value,
                    tags_json = excluded.tags_json,
                    properties_json = excluded.properties_json,
                    updated_at = excluded.updated_at
                "#,
                named_params! {
                    ":id": event.id.to_string(),
                    ":source_id": event.source_id.map(|value| value.to_string()),
                    ":source_record_key": event.source_record_key,
                    ":upstream_event_ref": event.upstream_event_ref,
                    ":upstream_reconciled_key": event.upstream_reconciled_key,
                    ":assertion_refs_json": assertion_refs_json,
                    ":source_refs_json": source_refs_json,
                    ":provenance_refs_json": provenance_refs_json,
                    ":renderability": event.renderability,
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
                    ":time_original_value": encoded.original_value,
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
        let sql = event_select_sql("WHERE id = ?1");
        let mut stmt = self.conn.prepare(&sql)?;
        stmt.query_row(params![id.to_string()], decode_event)
            .optional()
            .context("failed to query temporal event by id")
    }

    pub fn event_by_source_record(
        &self,
        source_id: Uuid,
        source_record_key: &str,
    ) -> anyhow::Result<Option<TemporalEvent>> {
        let sql = event_select_sql("WHERE source_id = ?1 AND source_record_key = ?2");
        let mut stmt = self.conn.prepare(&sql)?;
        stmt.query_row(
            params![source_id.to_string(), source_record_key],
            decode_event,
        )
        .optional()
        .context("failed to query temporal event by source record")
    }

    pub fn events_in_window(
        &self,
        start: NaiveDate,
        end_exclusive: NaiveDate,
        timezone: Tz,
        include_imprecise: bool,
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
                    time_kind IN ('date_only', 'all_day', 'month', 'year')
                    AND (time_kind NOT IN ('month', 'year') OR ?7 = 1)
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
            include_imprecise,
        ])?;

        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            events.push(decode_event(row)?);
        }
        Ok(events)
    }

    pub fn import_batch(
        &self,
        source: &TemporalSource,
        events: &mut [TemporalEvent],
    ) -> anyhow::Result<ImportBatchResult> {
        let existing_keys = self.source_record_keys(source.id)?;
        let incoming_keys = events
            .iter()
            .filter_map(|event| event.source_record_key.clone())
            .collect::<std::collections::BTreeSet<_>>();

        let tx = self
            .conn
            .unchecked_transaction()
            .context("failed to begin temporal import transaction")?;

        let result = (|| -> anyhow::Result<ImportBatchResult> {
            self.upsert_source(source)?;

            let mut created = 0usize;
            let mut updated = 0usize;

            for event in events {
                event.source_id = Some(source.id);
                if let Some(source_record_key) = event.source_record_key.as_deref()
                    && let Some(existing) =
                        self.event_by_source_record(source.id, source_record_key)?
                {
                    event.id = existing.id;
                    event.created_at = existing.created_at;
                    event.updated_at = Utc::now();
                    updated += 1;
                } else {
                    created += 1;
                }
                self.upsert_event(event)?;
            }

            Ok(ImportBatchResult {
                created,
                updated,
                retained_missing: existing_keys.difference(&incoming_keys).count(),
            })
        })();

        match result {
            Ok(result) => {
                tx.commit()
                    .context("failed to commit temporal import transaction")?;
                Ok(result)
            }
            Err(error) => {
                let _ = tx.rollback();
                Err(error)
            }
        }
    }

    pub fn source_record_keys(
        &self,
        source_id: Uuid,
    ) -> anyhow::Result<std::collections::BTreeSet<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_record_key
             FROM temporal_events
             WHERE source_id = ?1 AND source_record_key IS NOT NULL",
        )?;
        let mut rows = stmt.query(params![source_id.to_string()])?;
        let mut keys = std::collections::BTreeSet::new();
        while let Some(row) = rows.next()? {
            keys.insert(row.get::<_, String>(0)?);
        }
        Ok(keys)
    }

    pub fn unplaced_events(&self) -> anyhow::Result<Vec<TemporalEvent>> {
        let sql = event_select_sql(
            "WHERE time_kind = 'unknown' OR renderability IS NOT NULL AND renderability != 'ready'
             ORDER BY normalized_title COLLATE NOCASE",
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query([])?;
        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            events.push(decode_event(row)?);
        }
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

    if current == 0 {
        let tx = conn\n            .transaction()\n            .context("failed to start schema migration")?;
        create_schema_v2(&tx)?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)
            .context("failed to set schema version")?;
        tx.commit().context("failed to commit schema migration")?;
        return Ok(());
    }

    if current == 1 {
        migrate_v1_to_v2(conn)?;
    }

    Ok(())
}

fn create_schema_v2(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE temporal_sources (
            id TEXT PRIMARY KEY,
            external_ref TEXT UNIQUE,
            name TEXT NOT NULL,
            publisher TEXT,
            authority TEXT NOT NULL,
            kind TEXT NOT NULL,
            locator TEXT,
            enabled INTEGER NOT NULL DEFAULT 1,
            read_only INTEGER NOT NULL DEFAULT 1,
            properties_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE temporal_events (
            id TEXT PRIMARY KEY,
            source_id TEXT REFERENCES temporal_sources(id) ON DELETE SET NULL,
            source_record_key TEXT,

            upstream_event_ref TEXT,
            upstream_reconciled_key TEXT,
            assertion_refs_json TEXT NOT NULL DEFAULT '[]',
            source_refs_json TEXT NOT NULL DEFAULT '[]',
            provenance_refs_json TEXT NOT NULL DEFAULT '[]',
            renderability TEXT,

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
                CHECK (
                    time_kind IN (
                        'date_only', 'all_day', 'instant', 'floating',
                        'month', 'year', 'unknown'
                    )
                ),
            start_utc TEXT,
            end_utc TEXT,
            source_timezone TEXT,
            start_date TEXT,
            end_date_exclusive TEXT,
            start_local TEXT,
            end_local TEXT,
            time_original_value TEXT,

            tags_json TEXT NOT NULL DEFAULT '[]',
            properties_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,

            CHECK (
                (time_kind = 'date_only' AND start_date IS NOT NULL)
                OR (time_kind = 'all_day' AND start_date IS NOT NULL)
                OR (time_kind = 'instant' AND start_utc IS NOT NULL)
                OR (time_kind = 'floating' AND start_local IS NOT NULL)
                OR (time_kind = 'month' AND start_date IS NOT NULL AND end_date_exclusive IS NOT NULL)
                OR (time_kind = 'year' AND start_date IS NOT NULL AND end_date_exclusive IS NOT NULL)
                OR time_kind = 'unknown'
            )
        );

        CREATE UNIQUE INDEX temporal_events_source_record_key
            ON temporal_events(source_id, source_record_key)
            WHERE source_id IS NOT NULL AND source_record_key IS NOT NULL;

        CREATE INDEX temporal_events_start_date
            ON temporal_events(start_date)
            WHERE time_kind IN ('date_only', 'all_day', 'month', 'year');

        CREATE INDEX temporal_events_start_utc
            ON temporal_events(start_utc)
            WHERE time_kind = 'instant';

        CREATE INDEX temporal_events_start_local
            ON temporal_events(start_local)
            WHERE time_kind = 'floating';

        CREATE INDEX temporal_events_source_id
            ON temporal_events(source_id);

        CREATE INDEX temporal_events_upstream_event_ref
            ON temporal_events(upstream_event_ref);

        CREATE INDEX temporal_events_status
            ON temporal_events(status);

        CREATE INDEX temporal_events_domain
            ON temporal_events(domain);

        CREATE INDEX temporal_events_renderability
            ON temporal_events(renderability);
        "#,
    )
    .context("failed to create temporal schema")
}

fn migrate_v1_to_v2(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn\n        .transaction()\n        .context("failed to start v1 to v2 migration")?;

    tx.execute_batch(
        r#"
        ALTER TABLE temporal_sources ADD COLUMN external_ref TEXT;
        ALTER TABLE temporal_sources ADD COLUMN properties_json TEXT NOT NULL DEFAULT '{}';
        CREATE UNIQUE INDEX temporal_sources_external_ref
            ON temporal_sources(external_ref)
            WHERE external_ref IS NOT NULL;

        CREATE TABLE temporal_events_v2 (
            id TEXT PRIMARY KEY,
            source_id TEXT REFERENCES temporal_sources(id) ON DELETE SET NULL,
            source_record_key TEXT,

            upstream_event_ref TEXT,
            upstream_reconciled_key TEXT,
            assertion_refs_json TEXT NOT NULL DEFAULT '[]',
            source_refs_json TEXT NOT NULL DEFAULT '[]',
            provenance_refs_json TEXT NOT NULL DEFAULT '[]',
            renderability TEXT,

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
                CHECK (
                    time_kind IN (
                        'date_only', 'all_day', 'instant', 'floating',
                        'month', 'year', 'unknown'
                    )
                ),
            start_utc TEXT,
            end_utc TEXT,
            source_timezone TEXT,
            start_date TEXT,
            end_date_exclusive TEXT,
            start_local TEXT,
            end_local TEXT,
            time_original_value TEXT,

            tags_json TEXT NOT NULL DEFAULT '[]',
            properties_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,

            CHECK (
                (time_kind = 'date_only' AND start_date IS NOT NULL)
                OR (time_kind = 'all_day' AND start_date IS NOT NULL)
                OR (time_kind = 'instant' AND start_utc IS NOT NULL)
                OR (time_kind = 'floating' AND start_local IS NOT NULL)
                OR (time_kind = 'month' AND start_date IS NOT NULL AND end_date_exclusive IS NOT NULL)
                OR (time_kind = 'year' AND start_date IS NOT NULL AND end_date_exclusive IS NOT NULL)
                OR time_kind = 'unknown'
            )
        );

        INSERT INTO temporal_events_v2 (
            id, source_id, source_record_key,
            normalized_title, raw_title, description,
            event_type, domain, jurisdiction, institution,
            status, confidence, importance, personal_relevance,
            time_kind, start_utc, end_utc, source_timezone,
            start_date, end_date_exclusive,
            start_local, end_local,
            tags_json, properties_json,
            created_at, updated_at
        )
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
        FROM temporal_events;

        DROP TABLE temporal_events;
        ALTER TABLE temporal_events_v2 RENAME TO temporal_events;

        CREATE UNIQUE INDEX temporal_events_source_record_key
            ON temporal_events(source_id, source_record_key)
            WHERE source_id IS NOT NULL AND source_record_key IS NOT NULL;

        CREATE INDEX temporal_events_start_date
            ON temporal_events(start_date)
            WHERE time_kind IN ('date_only', 'all_day', 'month', 'year');

        CREATE INDEX temporal_events_start_utc
            ON temporal_events(start_utc)
            WHERE time_kind = 'instant';

        CREATE INDEX temporal_events_start_local
            ON temporal_events(start_local)
            WHERE time_kind = 'floating';

        CREATE INDEX temporal_events_source_id
            ON temporal_events(source_id);

        CREATE INDEX temporal_events_upstream_event_ref
            ON temporal_events(upstream_event_ref);

        CREATE INDEX temporal_events_status
            ON temporal_events(status);

        CREATE INDEX temporal_events_domain
            ON temporal_events(domain);

        CREATE INDEX temporal_events_renderability
            ON temporal_events(renderability);
        "#,
    )
    .context("failed to migrate schema from v1 to v2")?;

    tx.pragma_update(None, "user_version", SCHEMA_VERSION)
        .context("failed to set schema version")?;
    tx.commit()
        .context("failed to commit v1 to v2 schema migration")
}

fn event_select_sql(suffix: &str) -> String {
    format!(
        r#"
        SELECT
            id, source_id, source_record_key,
            upstream_event_ref, upstream_reconciled_key,
            assertion_refs_json, source_refs_json, provenance_refs_json, renderability,
            normalized_title, raw_title, description,
            event_type, domain, jurisdiction, institution,
            status, confidence, importance, personal_relevance,
            time_kind, start_utc, end_utc, source_timezone,
            start_date, end_date_exclusive,
            start_local, end_local, time_original_value,
            tags_json, properties_json,
            created_at, updated_at
        FROM temporal_events
        {suffix}
        "#
    )
}

fn decode_source(row: &Row<'_>) -> rusqlite::Result<TemporalSource> {
    let id_raw: String = row.get("id")?;
    let created_raw: String = row.get("created_at")?;
    let updated_raw: String = row.get("updated_at")?;
    let properties_raw: String = row.get("properties_json")?;

    let id = Uuid::parse_str(&id_raw).map_err(to_sql_decode_error)?;
    let created_at = parse_datetime(&created_raw).map_err(to_sql_decode_error)?;
    let updated_at = parse_datetime(&updated_raw).map_err(to_sql_decode_error)?;
    let properties = serde_json::from_str(&properties_raw).map_err(to_sql_decode_error)?;

    Ok(TemporalSource {
        id,
        external_ref: row.get("external_ref")?,
        name: row.get("name")?,
        publisher: row.get("publisher")?,
        authority: SourceAuthority::parse(&row.get::<_, String>("authority")?),
        kind: SourceKind::parse(&row.get::<_, String>("kind")?),
        locator: row.get("locator")?,
        enabled: row.get("enabled")?,
        read_only: row.get("read_only")?,
        properties,
        created_at,
        updated_at,
    })
}

fn decode_event(row: &Row<'_>) -> rusqlite::Result<TemporalEvent> {
    let time_kind: String = row.get("time_kind")?;
    let time = decode_time(row, &time_kind).map_err(to_sql_decode_error)?;

    let status_raw: String = row.get("status")?;
    let status = EventStatus::parse(&status_raw)
        .ok_or_else(|| to_sql_decode_error(anyhow!("unknown event status {status_raw}")))?;

    let id = Uuid::parse_str(&row.get::<_, String>("id")?).map_err(to_sql_decode_error)?;
    let source_id = row
        .get::<_, Option<String>>("source_id")?
        .map(|raw| Uuid::parse_str(&raw))
        .transpose()
        .map_err(to_sql_decode_error)?;

    Ok(TemporalEvent {
        id,
        source_id,
        source_record_key: row.get("source_record_key")?,
        upstream_event_ref: row.get("upstream_event_ref")?,
        upstream_reconciled_key: row.get("upstream_reconciled_key")?,
        assertion_refs: decode_string_vec(row, "assertion_refs_json")?,
        source_refs: decode_string_vec(row, "source_refs_json")?,
        provenance_refs: decode_string_vec(row, "provenance_refs_json")?,
        renderability: row.get("renderability")?,
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
        tags: decode_string_vec(row, "tags_json")?,
        properties: decode_json_value(row, "properties_json")?,
        created_at: parse_datetime(&row.get::<_, String>("created_at")?)
            .map_err(to_sql_decode_error)?,
        updated_at: parse_datetime(&row.get::<_, String>("updated_at")?)
            .map_err(to_sql_decode_error)?,
    })
}

fn decode_time(row: &Row<'_>, time_kind: &str) -> anyhow::Result<TimeSpec> {
    match time_kind {
        "date_only" => Ok(TimeSpec::DateOnly {
            start: parse_date(&required_text(row, "start_date")?)?,
            end_exclusive: optional_text(row, "end_date_exclusive")?
                .map(|raw| parse_date(&raw))
                .transpose()?,
        }),
        "all_day" => Ok(TimeSpec::AllDay {
            start: parse_date(&required_text(row, "start_date")?)?,
            end_exclusive: optional_text(row, "end_date_exclusive")?
                .map(|raw| parse_date(&raw))
                .transpose()?,
        }),
        "instant" => Ok(TimeSpec::Instant {
            start_utc: parse_datetime(&required_text(row, "start_utc")?)?,
            end_utc: optional_text(row, "end_utc")?
                .map(|raw| parse_datetime(&raw))
                .transpose()?,
            source_timezone: row.get("source_timezone")?,
        }),
        "floating" => Ok(TimeSpec::Floating {
            start: parse_naive_datetime(&required_text(row, "start_local")?)?,
            end: optional_text(row, "end_local")?
                .map(|raw| parse_naive_datetime(&raw))
                .transpose()?,
            source_timezone: row.get("source_timezone")?,
        }),
        "month" => {
            let start = parse_date(&required_text(row, "start_date")?)?;
            Ok(TimeSpec::Month {
                year: start.year(),
                month: start.month(),
            })
        }
        "year" => {
            let start = parse_date(&required_text(row, "start_date")?)?;
            Ok(TimeSpec::Year { year: start.year() })
        }
        "unknown" => Ok(TimeSpec::Unknown {
            original_value: row.get("time_original_value")?,
        }),
        other => Err(anyhow!("unknown time kind {other}")),
    }
}

fn required_text(row: &Row<'_>, column: &str) -> anyhow::Result<String> {
    optional_text(row, column)?.ok_or_else(|| anyhow!("missing required column {column}"))
}

fn optional_text(row: &Row<'_>, column: &str) -> anyhow::Result<Option<String>> {
    Ok(row.get(column)?)
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

fn encode_string_vec(values: &[String], label: &str) -> anyhow::Result<String> {
    serde_json::to_string(values).with_context(|| format!("failed to encode {label}"))
}

fn decode_string_vec(row: &Row<'_>, column: &str) -> rusqlite::Result<Vec<String>> {
    let raw: String = row.get(column)?;
    serde_json::from_str(&raw).map_err(to_sql_decode_error)
}

fn decode_json_value(row: &Row<'_>, column: &str) -> rusqlite::Result<Value> {
    let raw: String = row.get(column)?;
    serde_json::from_str(&raw).map_err(to_sql_decode_error)
}

fn to_sql_decode_error(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
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
    original_value: Option<String>,
}

impl EncodedTime {
    fn from_time_spec(time: &TimeSpec) -> anyhow::Result<Self> {
        match time {
            TimeSpec::DateOnly {
                start,
                end_exclusive,
            } => Ok(Self {
                kind: "date_only",
                start_utc: None,
                end_utc: None,
                source_timezone: None,
                start_date: Some(start.format("%Y-%m-%d").to_string()),
                end_date_exclusive: end_exclusive
                    .map(|value| value.format("%Y-%m-%d").to_string()),
                start_local: None,
                end_local: None,
                original_value: None,
            }),
            TimeSpec::AllDay {
                start,
                end_exclusive,
            } => Ok(Self {
                kind: "all_day",
                start_utc: None,
                end_utc: None,
                source_timezone: None,
                start_date: Some(start.format("%Y-%m-%d").to_string()),
                end_date_exclusive: end_exclusive
                    .map(|value| value.format("%Y-%m-%d").to_string()),
                start_local: None,
                end_local: None,
                original_value: None,
            }),
            TimeSpec::Instant {
                start_utc,
                end_utc,
                source_timezone,
            } => Ok(Self {
                kind: "instant",
                start_utc: Some(start_utc.to_rfc3339()),
                end_utc: end_utc.map(|value| value.to_rfc3339()),
                source_timezone: source_timezone.clone(),
                start_date: None,
                end_date_exclusive: None,
                start_local: None,
                end_local: None,
                original_value: None,
            }),
            TimeSpec::Floating {
                start,
                end,
                source_timezone,
            } => Ok(Self {
                kind: "floating",
                start_utc: None,
                end_utc: None,
                source_timezone: source_timezone.clone(),
                start_date: None,
                end_date_exclusive: None,
                start_local: Some(format_naive(*start)),
                end_local: end.map(format_naive),
                original_value: None,
            }),
            TimeSpec::Month { year, month } => {
                let start = NaiveDate::from_ymd_opt(*year, *month, 1)
                    .ok_or_else(|| anyhow!("invalid month precision value {year}-{month:02}"))?;
                let end = next_month(start)?;
                Ok(Self {
                    kind: "month",
                    start_utc: None,
                    end_utc: None,
                    source_timezone: None,
                    start_date: Some(start.format("%Y-%m-%d").to_string()),
                    end_date_exclusive: Some(end.format("%Y-%m-%d").to_string()),
                    start_local: None,
                    end_local: None,
                    original_value: Some(format!("{year}-{month:02}")),
                })
            }
            TimeSpec::Year { year } => {
                let start = NaiveDate::from_ymd_opt(*year, 1, 1)
                    .ok_or_else(|| anyhow!("invalid year precision value {year}"))?;
                let end = NaiveDate::from_ymd_opt(
                    year.checked_add(1)
                        .ok_or_else(|| anyhow!("year precision overflow for {year}"))?,
                    1,
                    1,
                )
                .ok_or_else(|| anyhow!("invalid end year for {year}"))?;
                Ok(Self {
                    kind: "year",
                    start_utc: None,
                    end_utc: None,
                    source_timezone: None,
                    start_date: Some(start.format("%Y-%m-%d").to_string()),
                    end_date_exclusive: Some(end.format("%Y-%m-%d").to_string()),
                    start_local: None,
                    end_local: None,
                    original_value: Some(year.to_string()),
                })
            }
            TimeSpec::Unknown { original_value } => Ok(Self {
                kind: "unknown",
                start_utc: None,
                end_utc: None,
                source_timezone: None,
                start_date: None,
                end_date_exclusive: None,
                start_local: None,
                end_local: None,
                original_value: original_value.clone(),
            }),
        }
    }
}

fn next_month(start: NaiveDate) -> anyhow::Result<NaiveDate> {
    if start.month() == 12 {
        NaiveDate::from_ymd_opt(
            start
                .year()
                .checked_add(1)
                .ok_or_else(|| anyhow!("month precision year overflow"))?,
            1,
            1,
        )
        .ok_or_else(|| anyhow!("invalid next month"))
    } else {
        NaiveDate::from_ymd_opt(start.year(), start.month() + 1, 1)
            .ok_or_else(|| anyhow!("invalid next month"))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn schema_bootstraps_at_version_two() {
        let store = TemporalStore::open_in_memory().expect("store");
        assert_eq!(store.schema_version().expect("version"), 2);
    }

    #[test]
    fn window_query_preserves_precise_time_kinds() {
        let store = TemporalStore::open_in_memory().expect("store");
        let source =
            TemporalSource::new("Test source", SourceKind::Taria, SourceAuthority::Official);
        store.upsert_source(&source).expect("source");

        let day = NaiveDate::from_ymd_opt(2026, 10, 4).expect("date");

        let mut date_only = TemporalEvent::new(
            "Date only",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        date_only.source_id = Some(source.id);
        store.upsert_event(&date_only).expect("date-only event");

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
                source_timezone: None,
            },
        );
        floating.source_id = Some(source.id);
        store.upsert_event(&floating).expect("floating event");

        let next_day = day.succ_opt().expect("next day");
        let events = store
            .events_in_window(day, next_day, chrono_tz::America::Mexico_City, false)
            .expect("window query");

        assert_eq!(events.len(), 3);
    }

    #[test]
    fn imprecise_month_and_year_are_not_forced_into_day_query() {
        let store = TemporalStore::open_in_memory().expect("store");
        let month = TemporalEvent::new(
            "Month event",
            TimeSpec::Month {
                year: 2027,
                month: 11,
            },
        );
        let year = TemporalEvent::new("Year event", TimeSpec::Year { year: 2027 });
        store.upsert_event(&month).expect("month");
        store.upsert_event(&year).expect("year");

        let day = NaiveDate::from_ymd_opt(2027, 11, 15).expect("day");
        let precise = store
            .events_in_window(day, day.succ_opt().expect("next"), chrono_tz::UTC, false)
            .expect("precise query");
        assert!(precise.is_empty());

        let month_start = NaiveDate::from_ymd_opt(2027, 11, 1).expect("start");
        let month_end = NaiveDate::from_ymd_opt(2027, 12, 1).expect("end");
        let broad = store
            .events_in_window(month_start, month_end, chrono_tz::UTC, true)
            .expect("broad query");
        assert_eq!(broad.len(), 2);
    }

    #[test]
    fn source_record_identity_is_unique_per_source() {
        let store = TemporalStore::open_in_memory().expect("store");
        let source = TemporalSource::new("Test source", SourceKind::Ics, SourceAuthority::Official);
        store.upsert_source(&source).expect("source");

        let day = NaiveDate::from_ymd_opt(2026, 10, 4).expect("date");
        let mut first = TemporalEvent::new("One", TimeSpec::DateOnly { date: day });
        first.source_id = Some(source.id);
        first.source_record_key = Some("uid-1".to_string());
        store.upsert_event(&first).expect("first");

        let mut second = TemporalEvent::new("Two", TimeSpec::DateOnly { date: day });
        second.source_id = Some(source.id);
        second.source_record_key = Some("uid-1".to_string());

        assert!(store.upsert_event(&second).is_err());
    }

    #[test]
    fn source_external_ref_roundtrips() {
        let store = TemporalStore::open_in_memory().expect("store");
        let mut source = TemporalSource::new(
            "Taria projection",
            SourceKind::Taria,
            SourceAuthority::Derived,
        );
        source.external_ref = Some("projection:fixture".to_string());
        store.upsert_source(&source).expect("source");

        let loaded = store
            .source_by_external_ref("projection:fixture")
            .expect("query")
            .expect("found");
        assert_eq!(loaded.id, source.id);
        assert_eq!(loaded.external_ref.as_deref(), Some("projection:fixture"));
    }

    #[test]
    fn unknown_time_is_retained_as_unplaced() {
        let store = TemporalStore::open_in_memory().expect("store");
        let event = TemporalEvent::new(
            "Blocked",
            TimeSpec::Unknown {
                original_value: None,
            },
        );
        store.upsert_event(&event).expect("event");

        assert_eq!(store.unplaced_event_count().expect("count"), 1);
        assert_eq!(store.unplaced_events().expect("events").len(), 1);
    }
}
