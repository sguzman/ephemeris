use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use chrono::{DateTime, Datelike, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use rusqlite::{Connection, OptionalExtension, Row, named_params, params};
use serde_json::Value;
use uuid::Uuid;

use crate::calendar::CalendarLayout;
use crate::domain::{
    EventStatus, SourceAuthority, SourceKind, TemporalEvent, TemporalSource, TimeSpec,
};
use crate::query::SavedView;

const SCHEMA_VERSION: i64 = 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportBatchResult {
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub retained_missing: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TariaReleaseRecord {
    pub release_id: String,
    pub channel: String,
    pub status: String,
    pub production_complete: bool,
    pub manifest_path: String,
    pub manifest_sha256: String,
    pub generated_at: Option<String>,
    pub coverage_json: String,
    pub manifest_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TariaCalendarSetRecord {
    pub calendar_set_id: String,
    pub release_id: String,
    pub bundle_ref: String,
    pub projection_ref: String,
    pub input_reconciled_event_set_ref: String,
    pub source_path: String,
    pub content_sha256: String,
    pub raw_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TariaProjectedCalendarRecord {
    pub calendar_id: String,
    pub name: String,
    pub kind: String,
    pub metadata_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TariaCalendarMembershipRecord {
    pub reconciled_event_ref: String,
    pub calendar_ref: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TariaCalendarSetImportResult {
    pub calendars: usize,
    pub memberships: usize,
    pub resolved_memberships: usize,
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

    pub fn list_saved_views(&self) -> anyhow::Result<Vec<SavedView>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                id, name, query_json, hidden_source_ids_json,
                calendar_view_json, calendar_layout,
                group_by_json, sort_rules_json, color_by_json, color_rules_json,
                composition_layers_json, overlays_json,
                display_timezone, week_start_monday
            FROM saved_views
            ORDER BY name COLLATE NOCASE, id
            "#,
        )?;

        let mut rows = stmt.query([])?;
        let mut views = Vec::new();
        while let Some(row) = rows.next()? {
            views.push(decode_saved_view(row)?);
        }
        Ok(views)
    }

    pub fn upsert_saved_view(&self, view: &SavedView) -> anyhow::Result<()> {
        let query_json =
            serde_json::to_string(&view.query).context("failed to encode saved-view query")?;
        let hidden_source_ids_json = serde_json::to_string(&view.hidden_source_ids)
            .context("failed to encode saved-view source visibility")?;
        let calendar_view_json = serde_json::to_string(&view.calendar_view)
            .context("failed to encode saved-view calendar range")?;
        let group_by_json = serde_json::to_string(&view.group_by)
            .context("failed to encode saved-view grouping")?;
        let sort_rules_json = serde_json::to_string(&view.sort_rules)
            .context("failed to encode saved-view sort rules")?;
        let color_by_json = serde_json::to_string(&view.color_by)
            .context("failed to encode saved-view color strategy")?;
        let color_rules_json = serde_json::to_string(&view.color_rules)
            .context("failed to encode saved-view color rules")?;
        let composition_layers_json = serde_json::to_string(&view.composition_layers)
            .context("failed to encode saved-view composition layers")?;
        let overlays_json = serde_json::to_string(&view.overlays)
            .context("failed to encode saved-view overlays")?;

        self.conn
            .execute(
                r#"
                INSERT INTO saved_views (
                    id, name, query_json, hidden_source_ids_json,
                    calendar_view_json, calendar_layout,
                    group_by_json, sort_rules_json, color_by_json, color_rules_json,
                    composition_layers_json, overlays_json,
                    display_timezone, week_start_monday
                ) VALUES (
                    :id, :name, :query_json, :hidden_source_ids_json,
                    :calendar_view_json, :calendar_layout,
                    :group_by_json, :sort_rules_json, :color_by_json, :color_rules_json,
                    :composition_layers_json, :overlays_json,
                    :display_timezone, :week_start_monday
                )
                ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    query_json = excluded.query_json,
                    hidden_source_ids_json = excluded.hidden_source_ids_json,
                    calendar_view_json = excluded.calendar_view_json,
                    calendar_layout = excluded.calendar_layout,
                    group_by_json = excluded.group_by_json,
                    sort_rules_json = excluded.sort_rules_json,
                    color_by_json = excluded.color_by_json,
                    color_rules_json = excluded.color_rules_json,
                    composition_layers_json = excluded.composition_layers_json,
                    overlays_json = excluded.overlays_json,
                    display_timezone = excluded.display_timezone,
                    week_start_monday = excluded.week_start_monday
                "#,
                named_params! {
                    ":id": view.id.to_string(),
                    ":name": view.name,
                    ":query_json": query_json,
                    ":hidden_source_ids_json": hidden_source_ids_json,
                    ":calendar_view_json": calendar_view_json,
                    ":calendar_layout": view.calendar_layout.as_str(),
                    ":group_by_json": group_by_json,
                    ":sort_rules_json": sort_rules_json,
                    ":color_by_json": color_by_json,
                    ":color_rules_json": color_rules_json,
                    ":composition_layers_json": composition_layers_json,
                    ":overlays_json": overlays_json,
                    ":display_timezone": view.display_timezone,
                    ":week_start_monday": view.week_start_monday,
                },
            )
            .context("failed to upsert saved view")?;
        Ok(())
    }

    pub fn delete_saved_view(&self, id: Uuid) -> anyhow::Result<()> {
        self.conn
            .execute(
                "DELETE FROM saved_views WHERE id = ?1",
                params![id.to_string()],
            )
            .context("failed to delete saved view")?;
        Ok(())
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

    pub fn event_by_import_record(
        &self,
        source_id: Uuid,
        source_record_key: &str,
    ) -> anyhow::Result<Option<TemporalEvent>> {
        let sql = event_select_sql(
            "WHERE id = (
                SELECT event_id
                FROM temporal_event_import_records
                WHERE source_id = ?1 AND source_record_key = ?2
            )",
        );
        let mut stmt = self.conn.prepare(&sql)?;
        stmt.query_row(
            params![source_id.to_string(), source_record_key],
            decode_event,
        )
        .optional()
        .context("failed to query temporal event by import record")
    }

    pub fn event_by_upstream_identity(
        &self,
        reconciled_ref: Option<&str>,
        event_ref: Option<&str>,
    ) -> anyhow::Result<Option<TemporalEvent>> {
        if let Some(reconciled_ref) = reconciled_ref {
            let sql = event_select_sql(
                "WHERE upstream_reconciled_key = ?1
                 ORDER BY created_at, id
                 LIMIT 1",
            );
            let mut stmt = self.conn.prepare(&sql)?;
            if let Some(event) = stmt
                .query_row(params![reconciled_ref], decode_event)
                .optional()
                .context("failed to query event by upstream reconciled identity")?
            {
                return Ok(Some(event));
            }
        }

        if let Some(event_ref) = event_ref {
            let sql = event_select_sql(
                "WHERE upstream_event_ref = ?1
                 ORDER BY created_at, id
                 LIMIT 1",
            );
            let mut stmt = self.conn.prepare(&sql)?;
            return stmt
                .query_row(params![event_ref], decode_event)
                .optional()
                .context("failed to query event by upstream event identity");
        }

        Ok(None)
    }

    pub fn upsert_taria_release(&self, release: &TariaReleaseRecord) -> anyhow::Result<()> {
        let existing_hash: Option<String> = self
            .conn
            .query_row(
                "SELECT manifest_sha256 FROM taria_releases WHERE release_id = ?1",
                params![release.release_id],
                |row| row.get(0),
            )
            .optional()
            .context("failed to check Taria release immutability")?;
        if let Some(existing_hash) = existing_hash
            && existing_hash != release.manifest_sha256
        {
            return Err(anyhow!(
                "Taria release {} changed content: immutable release hash {} != {}",
                release.release_id,
                existing_hash,
                release.manifest_sha256
            ));
        }

        self.conn
            .execute(
                r#"
                INSERT INTO taria_releases (
                    release_id, channel, status, production_complete,
                    manifest_path, manifest_sha256, generated_at,
                    coverage_json, manifest_json, adopted_at
                ) VALUES (
                    :release_id, :channel, :status, :production_complete,
                    :manifest_path, :manifest_sha256, :generated_at,
                    :coverage_json, :manifest_json, :adopted_at
                )
                ON CONFLICT(release_id) DO UPDATE SET
                    channel = excluded.channel,
                    manifest_path = excluded.manifest_path,
                    adopted_at = excluded.adopted_at
                "#,
                named_params! {
                    ":release_id": release.release_id,
                    ":channel": release.channel,
                    ":status": release.status,
                    ":production_complete": release.production_complete,
                    ":manifest_path": release.manifest_path,
                    ":manifest_sha256": release.manifest_sha256,
                    ":generated_at": release.generated_at,
                    ":coverage_json": release.coverage_json,
                    ":manifest_json": release.manifest_json,
                    ":adopted_at": Utc::now().to_rfc3339(),
                },
            )
            .context("failed to upsert Taria release metadata")?;
        Ok(())
    }

    pub fn replace_taria_calendar_set(
        &self,
        set: &TariaCalendarSetRecord,
        calendars: &[TariaProjectedCalendarRecord],
        memberships: &[TariaCalendarMembershipRecord],
    ) -> anyhow::Result<TariaCalendarSetImportResult> {
        let existing_hash: Option<String> = self
            .conn
            .query_row(
                "SELECT content_sha256 FROM taria_calendar_sets WHERE calendar_set_id = ?1",
                params![set.calendar_set_id],
                |row| row.get(0),
            )
            .optional()
            .context("failed to check CalendarSet immutability")?;
        if let Some(existing_hash) = existing_hash
            && existing_hash != set.content_sha256
        {
            return Err(anyhow!(
                "Taria CalendarSet {} changed content: immutable content hash {} != {}",
                set.calendar_set_id,
                existing_hash,
                set.content_sha256
            ));
        }

        let tx = self
            .conn
            .unchecked_transaction()
            .context("failed to begin CalendarSet transaction")?;

        let result = (|| -> anyhow::Result<TariaCalendarSetImportResult> {
            tx.execute(
                r#"
                INSERT INTO taria_calendar_sets (
                    calendar_set_id, projection_ref,
                    input_reconciled_event_set_ref, source_path,
                    content_sha256, raw_json
                ) VALUES (
                    :calendar_set_id, :projection_ref,
                    :input_reconciled_event_set_ref, :source_path,
                    :content_sha256, :raw_json
                )
                ON CONFLICT(calendar_set_id) DO NOTHING
                "#,
                named_params! {
                    ":calendar_set_id": set.calendar_set_id,
                    ":projection_ref": set.projection_ref,
                    ":input_reconciled_event_set_ref": set.input_reconciled_event_set_ref,
                    ":source_path": set.source_path,
                    ":content_sha256": set.content_sha256,
                    ":raw_json": set.raw_json,
                },
            )
            .context("failed to insert immutable Taria CalendarSet")?;

            tx.execute(
                r#"
                INSERT INTO taria_release_calendar_sets (
                    release_id, calendar_set_id, bundle_ref
                ) VALUES (?1, ?2, ?3)
                ON CONFLICT(release_id, calendar_set_id, bundle_ref) DO NOTHING
                "#,
                params![set.release_id, set.calendar_set_id, set.bundle_ref],
            )
            .context("failed to associate CalendarSet with Taria release")?;

            tx.execute(
                "DELETE FROM taria_calendar_memberships WHERE calendar_set_id = ?1",
                params![set.calendar_set_id],
            )
            .context("failed to clear prior CalendarSet memberships")?;
            tx.execute(
                "DELETE FROM taria_projected_calendars WHERE calendar_set_id = ?1",
                params![set.calendar_set_id],
            )
            .context("failed to clear prior projected calendars")?;

            for calendar in calendars {
                tx.execute(
                    r#"
                    INSERT INTO taria_projected_calendars (
                        calendar_set_id, calendar_id, name, kind, metadata_json
                    ) VALUES (?1, ?2, ?3, ?4, ?5)
                    "#,
                    params![
                        set.calendar_set_id,
                        calendar.calendar_id,
                        calendar.name,
                        calendar.kind,
                        calendar.metadata_json,
                    ],
                )
                .context("failed to insert projected calendar")?;
            }

            let mut resolved_memberships = 0usize;
            for membership in memberships {
                let event_id: Option<String> = tx
                    .query_row(
                        r#"
                        SELECT id
                        FROM temporal_events
                        WHERE upstream_reconciled_key = ?1
                        ORDER BY created_at, id
                        LIMIT 1
                        "#,
                        params![membership.reconciled_event_ref],
                        |row| row.get(0),
                    )
                    .optional()
                    .context("failed to resolve CalendarSet event membership")?;

                if event_id.is_some() {
                    resolved_memberships += 1;
                }

                tx.execute(
                    r#"
                    INSERT OR REPLACE INTO taria_calendar_memberships (
                        calendar_set_id, calendar_id, reconciled_event_ref, event_id
                    ) VALUES (?1, ?2, ?3, ?4)
                    "#,
                    params![
                        set.calendar_set_id,
                        membership.calendar_ref,
                        membership.reconciled_event_ref,
                        event_id,
                    ],
                )
                .context("failed to insert CalendarSet membership")?;
            }

            Ok(TariaCalendarSetImportResult {
                calendars: calendars.len(),
                memberships: memberships.len(),
                resolved_memberships,
            })
        })();

        match result {
            Ok(result) => {
                tx.commit()
                    .context("failed to commit CalendarSet transaction")?;
                Ok(result)
            }
            Err(error) => {
                let _ = tx.rollback();
                Err(error)
            }
        }
    }

    pub fn resolve_taria_calendar_memberships(&self) -> anyhow::Result<usize> {
        let changed = self
            .conn
            .execute(
                r#"
                UPDATE taria_calendar_memberships
                SET event_id = (
                    SELECT id
                    FROM temporal_events
                    WHERE upstream_reconciled_key =
                        taria_calendar_memberships.reconciled_event_ref
                    ORDER BY created_at, id
                    LIMIT 1
                )
                WHERE event_id IS NULL
                  AND EXISTS (
                    SELECT 1
                    FROM temporal_events
                    WHERE upstream_reconciled_key =
                        taria_calendar_memberships.reconciled_event_ref
                  )
                "#,
                [],
            )
            .context("failed to resolve Taria CalendarSet memberships")?;
        Ok(changed)
    }

    pub fn taria_release_count(&self) -> anyhow::Result<u64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM taria_releases", [], |row| row.get(0))
            .context("failed to count Taria releases")?;
        u64::try_from(count).context("Taria release count cannot be represented as u64")
    }

    pub fn taria_calendar_membership_count(&self) -> anyhow::Result<u64> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM taria_calendar_memberships",
                [],
                |row| row.get(0),
            )
            .context("failed to count Taria CalendarSet memberships")?;
        u64::try_from(count).context("Taria membership count cannot be represented as u64")
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
            let mut unchanged = 0usize;

            for event in events {
                event.source_id = Some(source.id);
                if let Some(source_record_key) = event.source_record_key.as_deref()
                    && let Some(existing) =
                        self.event_by_source_record(source.id, source_record_key)?
                {
                    event.id = existing.id;
                    event.created_at = existing.created_at;
                    event.updated_at = existing.updated_at;
                    if *event == existing {
                        unchanged += 1;
                        continue;
                    }
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
                unchanged,
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

    pub fn import_taria_batch(
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
            .context("failed to begin Taria import transaction")?;

        let result = (|| -> anyhow::Result<ImportBatchResult> {
            self.upsert_source(source)?;

            let mut created = 0usize;
            let mut updated = 0usize;
            let mut unchanged = 0usize;

            for event in events {
                event.source_id = Some(source.id);
                let source_record_key = event
                    .source_record_key
                    .clone()
                    .ok_or_else(|| anyhow!("Taria event is missing source-record identity"))?;

                let mapped = self.event_by_import_record(source.id, &source_record_key)?;
                let existing = match mapped {
                    Some(existing) => Some((existing, true)),
                    None => self
                        .event_by_upstream_identity(
                            event.upstream_reconciled_key.as_deref(),
                            event.upstream_event_ref.as_deref(),
                        )?
                        .map(|existing| (existing, false)),
                };

                if let Some((existing, mapped_to_this_source)) = existing {
                    let mut candidate =
                        if mapped_to_this_source && existing.source_id == Some(source.id) {
                            let mut candidate = event.clone();
                            candidate.id = existing.id;
                            candidate.created_at = existing.created_at;
                            candidate.updated_at = existing.updated_at;
                            candidate
                        } else {
                            merge_taria_projection_event(&existing, event)
                        };

                    if candidate == existing {
                        *event = candidate;
                        unchanged += 1;
                    } else {
                        candidate.updated_at = Utc::now();
                        self.upsert_event(&candidate)?;
                        *event = candidate;
                        updated += 1;
                    }
                } else {
                    self.upsert_event(event)?;
                    created += 1;
                }

                tx.execute(
                    r#"
                    INSERT INTO temporal_event_import_records (
                        source_id, source_record_key, event_id
                    ) VALUES (?1, ?2, ?3)
                    ON CONFLICT(source_id, source_record_key) DO UPDATE SET
                        event_id = excluded.event_id
                    "#,
                    params![
                        source.id.to_string(),
                        source_record_key,
                        event.id.to_string(),
                    ],
                )
                .context("failed to record Taria event import identity")?;
            }

            Ok(ImportBatchResult {
                created,
                updated,
                unchanged,
                retained_missing: existing_keys.difference(&incoming_keys).count(),
            })
        })();

        match result {
            Ok(result) => {
                tx.commit()
                    .context("failed to commit Taria import transaction")?;
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
             FROM temporal_event_import_records
             WHERE source_id = ?1
             UNION
             SELECT source_record_key
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

fn merge_taria_projection_event(
    existing: &TemporalEvent,
    incoming: &TemporalEvent,
) -> TemporalEvent {
    let mut merged = incoming.clone();
    merged.id = existing.id;
    merged.source_id = existing.source_id;
    merged
        .source_record_key
        .clone_from(&existing.source_record_key);
    merged.created_at = existing.created_at;
    merged.updated_at = existing.updated_at;

    merged.assertion_refs =
        merge_unique_strings(&existing.assertion_refs, &incoming.assertion_refs);
    merged.source_refs = merge_unique_strings(&existing.source_refs, &incoming.source_refs);
    merged.provenance_refs =
        merge_unique_strings(&existing.provenance_refs, &incoming.provenance_refs);
    merged.tags = merge_unique_strings(&existing.tags, &incoming.tags);

    if merged.description.is_none() {
        merged.description.clone_from(&existing.description);
    }
    if merged.event_type.is_none() {
        merged.event_type.clone_from(&existing.event_type);
    }
    if existing.domain.is_some() {
        merged.domain.clone_from(&existing.domain);
    }
    if merged.jurisdiction.is_none() {
        merged.jurisdiction.clone_from(&existing.jurisdiction);
    }
    if merged.institution.is_none() {
        merged.institution.clone_from(&existing.institution);
    }
    if merged.confidence.is_none() {
        merged.confidence = existing.confidence;
    }
    if merged.importance.is_none() {
        merged.importance = existing.importance;
    }
    if merged.personal_relevance.is_none() {
        merged.personal_relevance = existing.personal_relevance;
    }

    if !existing
        .properties
        .as_object()
        .is_none_or(serde_json::Map::is_empty)
    {
        merged.properties.clone_from(&existing.properties);
    }

    merged
}

fn merge_unique_strings(left: &[String], right: &[String]) -> Vec<String> {
    left.iter()
        .chain(right.iter())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
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
    let mut current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if current > SCHEMA_VERSION {
        return Err(anyhow!(
            "database schema version {current} is newer than supported version {SCHEMA_VERSION}"
        ));
    }

    if current == 0 {
        let tx = conn
            .transaction()
            .context("failed to start schema migration")?;
        create_schema_v2(&tx)?;
        create_saved_views_schema_current(&tx)?;
        create_taria_release_schema_current(&tx)?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)
            .context("failed to set schema version")?;
        tx.commit().context("failed to commit schema migration")?;
        return Ok(());
    }

    if current == 1 {
        migrate_v1_to_v2(conn)?;
        current = 2;
    }

    if current == 2 {
        migrate_v2_to_v3(conn)?;
        current = 3;
    }

    if current == 3 {
        migrate_v3_to_v4(conn)?;
        current = 4;
    }

    if current == 4 {
        migrate_v4_to_v5(conn)?;
        current = 5;
    }

    if current == 5 {
        migrate_v5_to_v6(conn)?;
        current = 6;
    }

    if current == 6 {
        migrate_v6_to_v7(conn)?;
        current = 7;
    }

    if current == 7 {
        migrate_v7_to_v8(conn)?;
        current = 8;
    }

    if current == 8 {
        migrate_v8_to_v9(conn)?;
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

fn create_taria_release_schema_current(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE temporal_event_import_records (
            source_id TEXT NOT NULL REFERENCES temporal_sources(id) ON DELETE CASCADE,
            source_record_key TEXT NOT NULL,
            event_id TEXT NOT NULL REFERENCES temporal_events(id) ON DELETE CASCADE,
            PRIMARY KEY (source_id, source_record_key)
        );

        CREATE INDEX temporal_event_import_records_event
            ON temporal_event_import_records(event_id);

        CREATE INDEX temporal_events_upstream_reconciled_key
            ON temporal_events(upstream_reconciled_key);

        CREATE TABLE taria_releases (
            release_id TEXT PRIMARY KEY,
            channel TEXT NOT NULL,
            status TEXT NOT NULL,
            production_complete INTEGER NOT NULL DEFAULT 0,
            manifest_path TEXT NOT NULL,
            manifest_sha256 TEXT NOT NULL,
            generated_at TEXT,
            coverage_json TEXT NOT NULL DEFAULT '{}',
            manifest_json TEXT NOT NULL,
            adopted_at TEXT NOT NULL
        );

        CREATE INDEX taria_releases_channel
            ON taria_releases(channel, adopted_at);

        CREATE TABLE taria_calendar_sets (
            calendar_set_id TEXT PRIMARY KEY,
            projection_ref TEXT NOT NULL,
            input_reconciled_event_set_ref TEXT NOT NULL,
            source_path TEXT NOT NULL,
            content_sha256 TEXT NOT NULL,
            raw_json TEXT NOT NULL
        );

        CREATE TABLE taria_release_calendar_sets (
            release_id TEXT NOT NULL REFERENCES taria_releases(release_id) ON DELETE CASCADE,
            calendar_set_id TEXT NOT NULL REFERENCES taria_calendar_sets(calendar_set_id)
                ON DELETE CASCADE,
            bundle_ref TEXT NOT NULL,
            PRIMARY KEY (release_id, calendar_set_id, bundle_ref)
        );

        CREATE INDEX taria_release_calendar_sets_bundle
            ON taria_release_calendar_sets(bundle_ref, release_id);

        CREATE TABLE taria_projected_calendars (
            calendar_set_id TEXT NOT NULL REFERENCES taria_calendar_sets(calendar_set_id)
                ON DELETE CASCADE,
            calendar_id TEXT NOT NULL,
            name TEXT NOT NULL,
            kind TEXT NOT NULL,
            metadata_json TEXT NOT NULL DEFAULT '{}',
            PRIMARY KEY (calendar_set_id, calendar_id)
        );

        CREATE TABLE taria_calendar_memberships (
            calendar_set_id TEXT NOT NULL REFERENCES taria_calendar_sets(calendar_set_id)
                ON DELETE CASCADE,
            calendar_id TEXT NOT NULL,
            reconciled_event_ref TEXT NOT NULL,
            event_id TEXT REFERENCES temporal_events(id) ON DELETE SET NULL,
            PRIMARY KEY (calendar_set_id, calendar_id, reconciled_event_ref)
        );

        CREATE INDEX taria_calendar_memberships_event
            ON taria_calendar_memberships(event_id);

        CREATE INDEX taria_calendar_memberships_reconciled
            ON taria_calendar_memberships(reconciled_event_ref);
        "#,
    )
    .context("failed to create Taria release/membership schema")?;

    conn.execute(
        r#"
        INSERT OR IGNORE INTO temporal_event_import_records (
            source_id, source_record_key, event_id
        )
        SELECT source_id, source_record_key, id
        FROM temporal_events
        WHERE source_id IS NOT NULL AND source_record_key IS NOT NULL
        "#,
        [],
    )
    .context("failed to backfill temporal import-record identities")?;

    Ok(())
}

fn create_saved_views_schema_v3(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE saved_views (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            query_json TEXT NOT NULL,
            hidden_source_ids_json TEXT NOT NULL DEFAULT '[]',
            calendar_view_json TEXT NOT NULL,
            display_timezone TEXT NOT NULL,
            week_start_monday INTEGER NOT NULL DEFAULT 0
        );

        CREATE INDEX saved_views_name
            ON saved_views(name COLLATE NOCASE);
        "#,
    )
    .context("failed to create v3 saved-view schema")
}

fn create_saved_views_schema_current(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE saved_views (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            query_json TEXT NOT NULL,
            hidden_source_ids_json TEXT NOT NULL DEFAULT '[]',
            calendar_view_json TEXT NOT NULL,
            calendar_layout TEXT NOT NULL DEFAULT 'grid',
            group_by_json TEXT NOT NULL DEFAULT '"date"',
            sort_rules_json TEXT NOT NULL DEFAULT '[{"field":"time","direction":"ascending"}]',
            color_by_json TEXT NOT NULL DEFAULT '"status"',
            color_rules_json TEXT NOT NULL DEFAULT '[]',
            composition_layers_json TEXT NOT NULL DEFAULT '[]',
            overlays_json TEXT NOT NULL DEFAULT '[]',
            display_timezone TEXT NOT NULL,
            week_start_monday INTEGER NOT NULL DEFAULT 0
        );

        CREATE INDEX saved_views_name
            ON saved_views(name COLLATE NOCASE);
        "#,
    )
    .context("failed to create saved-view schema")
}

fn migrate_v2_to_v3(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context("failed to start v2 to v3 migration")?;
    create_saved_views_schema_v3(&tx)?;
    tx.pragma_update(None, "user_version", 3)
        .context("failed to set schema version 3")?;
    tx.commit()
        .context("failed to commit v2 to v3 schema migration")
}

fn migrate_v8_to_v9(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context("failed to start v8 to v9 migration")?;
    create_taria_release_schema_current(&tx)?;
    tx.pragma_update(None, "user_version", 9)
        .context("failed to set schema version 9")?;
    tx.commit()
        .context("failed to commit v8 to v9 schema migration")
}

fn migrate_v7_to_v8(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context("failed to start v7 to v8 migration")?;
    tx.execute_batch(
        "ALTER TABLE saved_views ADD COLUMN composition_layers_json TEXT NOT NULL DEFAULT '[]';",
    )
    .context("failed to add saved-view composition layers")?;
    tx.pragma_update(None, "user_version", 8)
        .context("failed to set schema version 8")?;
    tx.commit()
        .context("failed to commit v7 to v8 schema migration")
}

fn migrate_v6_to_v7(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context("failed to start v6 to v7 migration")?;
    tx.execute_batch(
        "ALTER TABLE saved_views ADD COLUMN overlays_json TEXT NOT NULL DEFAULT '[]';",
    )
    .context("failed to add saved-view overlays")?;
    tx.pragma_update(None, "user_version", 7)
        .context("failed to set schema version 7")?;
    tx.commit()
        .context("failed to commit v6 to v7 schema migration")
}

fn migrate_v5_to_v6(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context("failed to start v5 to v6 migration")?;
    tx.execute_batch(
        "ALTER TABLE saved_views ADD COLUMN color_rules_json TEXT NOT NULL DEFAULT '[]';",
    )
    .context("failed to add saved-view color rules")?;
    tx.pragma_update(None, "user_version", 6)
        .context("failed to set schema version 6")?;
    tx.commit()
        .context("failed to commit v5 to v6 schema migration")
}

fn migrate_v4_to_v5(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context("failed to start v4 to v5 migration")?;
    tx.execute_batch(
        r#"
        ALTER TABLE saved_views ADD COLUMN group_by_json TEXT NOT NULL DEFAULT '"date"';
        ALTER TABLE saved_views ADD COLUMN sort_rules_json TEXT NOT NULL
            DEFAULT '[{"field":"time","direction":"ascending"}]';
        ALTER TABLE saved_views ADD COLUMN color_by_json TEXT NOT NULL DEFAULT '"status"';
        "#,
    )
    .context("failed to add saved-view presentation dimensions")?;
    tx.pragma_update(None, "user_version", 5)
        .context("failed to set schema version 5")?;
    tx.commit()
        .context("failed to commit v4 to v5 schema migration")
}

fn migrate_v3_to_v4(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context("failed to start v3 to v4 migration")?;
    tx.execute_batch(
        "ALTER TABLE saved_views ADD COLUMN calendar_layout TEXT NOT NULL DEFAULT 'grid';",
    )
    .context("failed to add saved-view calendar layout")?;
    tx.pragma_update(None, "user_version", 4)
        .context("failed to set schema version 4")?;
    tx.commit()
        .context("failed to commit v3 to v4 schema migration")
}

fn migrate_v1_to_v2(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context("failed to start v1 to v2 migration")?;

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

    tx.pragma_update(None, "user_version", 2)
        .context("failed to set schema version 2")?;
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

fn decode_saved_view(row: &Row<'_>) -> rusqlite::Result<SavedView> {
    let id_raw: String = row.get("id")?;
    let query_raw: String = row.get("query_json")?;
    let hidden_raw: String = row.get("hidden_source_ids_json")?;
    let calendar_view_raw: String = row.get("calendar_view_json")?;
    let group_by_raw: String = row.get("group_by_json")?;
    let sort_rules_raw: String = row.get("sort_rules_json")?;
    let color_by_raw: String = row.get("color_by_json")?;
    let color_rules_raw: String = row.get("color_rules_json")?;
    let composition_layers_raw: String = row.get("composition_layers_json")?;
    let overlays_raw: String = row.get("overlays_json")?;

    Ok(SavedView {
        id: Uuid::parse_str(&id_raw).map_err(to_sql_decode_error)?,
        name: row.get("name")?,
        query: serde_json::from_str(&query_raw).map_err(to_sql_decode_error)?,
        hidden_source_ids: serde_json::from_str(&hidden_raw).map_err(to_sql_decode_error)?,
        calendar_view: serde_json::from_str(&calendar_view_raw).map_err(to_sql_decode_error)?,
        calendar_layout: CalendarLayout::parse(&row.get::<_, String>("calendar_layout")?),
        group_by: serde_json::from_str(&group_by_raw).map_err(to_sql_decode_error)?,
        sort_rules: serde_json::from_str(&sort_rules_raw).map_err(to_sql_decode_error)?,
        color_by: serde_json::from_str(&color_by_raw).map_err(to_sql_decode_error)?,
        color_rules: serde_json::from_str(&color_rules_raw).map_err(to_sql_decode_error)?,
        composition_layers: serde_json::from_str(&composition_layers_raw)
            .map_err(to_sql_decode_error)?,
        overlays: serde_json::from_str(&overlays_raw).map_err(to_sql_decode_error)?,
        display_timezone: row.get("display_timezone")?,
        week_start_monday: row.get("week_start_monday")?,
    })
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

fn to_sql_decode_error(error: impl std::fmt::Display) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        )),
    )
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
                end_date_exclusive: end_exclusive.map(|value| value.format("%Y-%m-%d").to_string()),
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
                end_date_exclusive: end_exclusive.map(|value| value.format("%Y-%m-%d").to_string()),
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
    fn schema_v2_migrates_cleanly_to_current_saved_view_shape() {
        let mut conn = Connection::open_in_memory().expect("connection");
        configure_connection(&conn).expect("configure");
        create_schema_v2(&conn).expect("v2 temporal schema");
        conn.pragma_update(None, "user_version", 2).expect("set v2");

        migrate(&mut conn).expect("migrate");

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("version");
        assert_eq!(version, SCHEMA_VERSION);

        let columns = {
            let mut stmt = conn
                .prepare("PRAGMA table_info(saved_views)")
                .expect("table info");
            stmt.query_map([], |row| row.get::<_, String>(1))
                .expect("columns")
                .collect::<Result<Vec<_>, _>>()
                .expect("column names")
        };

        assert!(columns.contains(&"calendar_layout".to_string()));
        assert!(columns.contains(&"group_by_json".to_string()));
        assert!(columns.contains(&"sort_rules_json".to_string()));
        assert!(columns.contains(&"color_by_json".to_string()));
        assert!(columns.contains(&"color_rules_json".to_string()));
        assert!(columns.contains(&"overlays_json".to_string()));
        assert!(columns.contains(&"composition_layers_json".to_string()));
    }

    #[test]
    fn schema_bootstraps_at_current_version() {
        let store = TemporalStore::open_in_memory().expect("store");
        assert_eq!(store.schema_version().expect("version"), SCHEMA_VERSION);
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
        let mut first = TemporalEvent::new(
            "One",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        first.source_id = Some(source.id);
        first.source_record_key = Some("uid-1".to_string());
        store.upsert_event(&first).expect("first");

        let mut second = TemporalEvent::new(
            "Two",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        second.source_id = Some(source.id);
        second.source_record_key = Some("uid-1".to_string());

        assert!(store.upsert_event(&second).is_err());
    }

    #[test]
    fn direct_source_import_reconciliation_survives_taria_import_mapping_schema() {
        let store = TemporalStore::open_in_memory().expect("store");
        let source = TemporalSource::new("ICS fixture", SourceKind::Ics, SourceAuthority::Official);
        let day = NaiveDate::from_ymd_opt(2026, 10, 4).expect("date");

        let mut first = TemporalEvent::new(
            "First",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        first.source_record_key = Some("uid-1".to_string());

        let mut second = TemporalEvent::new(
            "Second",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        second.source_record_key = Some("uid-2".to_string());

        let first_result = store
            .import_batch(&source, &mut [first.clone(), second.clone()])
            .expect("first import");
        assert_eq!(first_result.created, 2);
        assert_eq!(first_result.retained_missing, 0);

        let second_result = store
            .import_batch(&source, &mut [first])
            .expect("second import");
        assert_eq!(second_result.created, 0);
        assert_eq!(second_result.unchanged, 1);
        assert_eq!(second_result.retained_missing, 1);
    }

    #[test]
    fn saved_view_roundtrips_through_database() {
        use crate::calendar::CalendarView;
        use crate::query::{
            ColorBy, ColorRule, CompositionLayer, CompositionOperator, EventQuery, GroupBy,
            IntegerField, IntegerOperator, Overlay, QueryExpr, QueryPredicate, RgbColor, SavedView,
            SortDirection, SortField, SortRule,
        };

        let store = TemporalStore::open_in_memory().expect("store");
        let view = SavedView {
            id: Uuid::new_v4(),
            name: "US Elections".to_string(),
            query: EventQuery {
                domain: Some("elections".to_string()),
                jurisdiction: Some("US".to_string()),
                ..EventQuery::default()
            },
            hidden_source_ids: std::collections::BTreeSet::new(),
            calendar_view: CalendarView::Year,
            calendar_layout: CalendarLayout::Agenda,
            group_by: GroupBy::Jurisdiction,
            sort_rules: vec![SortRule {
                field: SortField::Importance,
                direction: SortDirection::Descending,
            }],
            color_by: ColorBy::EventType,
            color_rules: vec![ColorRule {
                id: Uuid::new_v4(),
                name: "Important".to_string(),
                enabled: true,
                when: QueryExpr::Predicate(QueryPredicate::Integer {
                    field: IntegerField::Importance,
                    operator: IntegerOperator::GreaterThanOrEqual,
                    value: 80,
                }),
                color: RgbColor::new(255, 80, 80),
            }],
            composition_layers: vec![CompositionLayer {
                id: Uuid::new_v4(),
                name: "Exclude cancelled".to_string(),
                enabled: true,
                operator: CompositionOperator::Subtract,
                query: EventQuery {
                    status: Some(EventStatus::Cancelled),
                    ..EventQuery::default()
                },
            }],
            overlays: vec![Overlay {
                id: Uuid::new_v4(),
                name: "California".to_string(),
                enabled: true,
                query: EventQuery {
                    jurisdiction: Some("US-CA".to_string()),
                    ..EventQuery::default()
                },
                color_by: ColorBy::Jurisdiction,
                color_rules: Vec::new(),
            }],
            display_timezone: "America/Mexico_City".to_string(),
            week_start_monday: false,
        };

        store.upsert_saved_view(&view).expect("save");
        let loaded = store.list_saved_views().expect("list");

        assert_eq!(loaded, vec![view.clone()]);
        assert_eq!(loaded[0].calendar_layout, CalendarLayout::Agenda);
        assert_eq!(loaded[0].group_by, GroupBy::Jurisdiction);
        assert_eq!(loaded[0].color_by, ColorBy::EventType);
        assert_eq!(loaded[0].color_rules.len(), 1);
        assert_eq!(loaded[0].composition_layers.len(), 1);
        assert_eq!(loaded[0].overlays.len(), 1);
        assert_eq!(loaded[0].sort_rules.len(), 1);
        store.delete_saved_view(view.id).expect("delete");
        assert!(store.list_saved_views().expect("list").is_empty());
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
