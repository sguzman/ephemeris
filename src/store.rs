use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use chrono::{DateTime, Datelike, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use rusqlite::{Connection, OptionalExtension, Row, named_params, params};
use serde_json::Value;
use uuid::Uuid;

use crate::calendar::CalendarLayout;
use crate::domain::{
    EventAnnotation, EventCollection, EventCollectionMember, EventIdentityAssessment,
    EventIdentityState, EventProvenanceRecord, EventProvenanceRole, EventRelation, EventStatus,
    RecurrenceRule, SourceAuthority, SourceKind, TemporalEvent, TemporalSource, TimeSpec,
    TimeUncertainty,
};
use crate::query::{EventMembership, SavedView, saved_view_reference_cycle};

const SCHEMA_VERSION: i64 = 19;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportBatchResult {
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub retained_missing: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalSnapshotMergeResult {
    pub sources_created: usize,
    pub sources_updated: usize,
    pub sources_unchanged: usize,
    pub events_created: usize,
    pub events_updated: usize,
    pub events_unchanged: usize,
    pub relations_created: usize,
    pub relations_updated: usize,
    pub relations_unchanged: usize,
    pub collections_created: usize,
    pub collections_updated: usize,
    pub collections_unchanged: usize,
    pub collection_memberships_replaced: usize,
    pub collection_memberships_unchanged: usize,
    pub identity_assessments_created: usize,
    pub identity_assessments_updated: usize,
    pub identity_assessments_unchanged: usize,
    pub annotations_created: usize,
    pub annotations_updated: usize,
    pub annotations_unchanged: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct CanonicalSnapshotMergeInput<'a> {
    pub sources: &'a [TemporalSource],
    pub events: &'a [TemporalEvent],
    pub relations: &'a [EventRelation],
    pub collections: &'a [EventCollection],
    pub collection_members: &'a [EventCollectionMember],
    pub identity_assessments: &'a [EventIdentityAssessment],
    pub annotations: &'a [EventAnnotation],
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TariaReleaseStatusRecord {
    pub release_id: String,
    pub channel: String,
    pub status: String,
    pub production_complete: bool,
    pub generated_at: Option<String>,
    pub adopted_at: String,
    pub coverage_json: String,
    pub manifest_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TariaProjectedCalendarChoice {
    pub bundle_ref: String,
    pub calendar_set_id: String,
    pub calendar_id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRefreshAttempt {
    pub id: Uuid,
    pub refresh_kind: String,
    pub target: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub success: Option<bool>,
    pub release_id: Option<String>,
    pub summary: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TariaReleaseHistoryEntry {
    pub release_id: String,
    pub channel: String,
    pub status: String,
    pub production_complete: bool,
    pub generated_at: Option<String>,
    pub adopted_at: String,
    pub bundle_count: u64,
    pub calendar_set_count: u64,
    pub projected_calendar_count: u64,
    pub source_count: u64,
    pub snapshot_event_count: u64,
    pub resolved_member_event_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TariaReleaseDiff {
    pub from_release_id: String,
    pub to_release_id: String,
    pub added_source_projection_refs: Vec<String>,
    pub removed_source_projection_refs: Vec<String>,
    pub added_bundle_refs: Vec<String>,
    pub removed_bundle_refs: Vec<String>,
    pub added_calendar_ids: Vec<String>,
    pub removed_calendar_ids: Vec<String>,
    pub added_snapshot_event_ids: Vec<Uuid>,
    pub removed_snapshot_event_ids: Vec<Uuid>,
    pub moved_event_ids: Vec<Uuid>,
    pub status_changed_event_ids: Vec<Uuid>,
    pub newly_cancelled_event_ids: Vec<Uuid>,
    pub event_changes: Vec<TariaEventChangeDetail>,
    pub added_member_event_ids: Vec<Uuid>,
    pub removed_member_event_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TariaEventChangeDetail {
    pub event_id: Uuid,
    pub title: String,
    pub added: bool,
    pub removed: bool,
    pub title_changed: bool,
    pub moved: bool,
    pub status_changed: bool,
    pub newly_cancelled: bool,
    pub from_title: Option<String>,
    pub to_title: Option<String>,
    pub from_status: Option<String>,
    pub to_status: Option<String>,
    pub from_time_json: Option<String>,
    pub to_time_json: Option<String>,
}

#[derive(Debug, Clone)]
struct StoredTariaEventSnapshot {
    normalized_title: String,
    status: String,
    time_json: String,
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

    pub fn source_by_id(&self, id: Uuid) -> anyhow::Result<Option<TemporalSource>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, external_ref, name, publisher, authority, kind, locator,
                   enabled, read_only, properties_json, created_at, updated_at
            FROM temporal_sources
            WHERE id = ?1
            "#,
        )?;

        stmt.query_row(params![id.to_string()], decode_source)
            .optional()
            .context("failed to query temporal source by id")
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

    pub fn list_events(&self) -> anyhow::Result<Vec<TemporalEvent>> {
        let sql = event_select_sql("ORDER BY id");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query([])?;
        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            events.push(decode_event(row)?);
        }
        Ok(events)
    }

    pub fn events_for_source(&self, source_id: Uuid) -> anyhow::Result<Vec<TemporalEvent>> {
        let sql = event_select_sql(
            "WHERE id IN (
                SELECT id
                FROM temporal_events
                WHERE source_id = ?1
                UNION
                SELECT event_id
                FROM temporal_event_import_records
                WHERE source_id = ?1
            )
            ORDER BY normalized_title COLLATE NOCASE, id",
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(params![source_id.to_string()])?;
        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            events.push(decode_event(row)?);
        }
        Ok(events)
    }

    pub fn source_event_counts(&self) -> anyhow::Result<HashMap<Uuid, u64>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT source_id, COUNT(DISTINCT event_id)
            FROM (
                SELECT source_id, event_id
                FROM temporal_event_import_records
                UNION
                SELECT source_id, id AS event_id
                FROM temporal_events
                WHERE source_id IS NOT NULL
            )
            GROUP BY source_id
            "#,
        )?;

        let mut rows = stmt.query([])?;
        let mut counts = HashMap::new();
        while let Some(row) = rows.next()? {
            let raw_source_id: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            let source_id = Uuid::parse_str(&raw_source_id).with_context(|| {
                format!("invalid source id in temporal_events: {raw_source_id}")
            })?;
            let count =
                u64::try_from(count).context("source event count cannot be represented as u64")?;
            counts.insert(source_id, count);
        }
        Ok(counts)
    }

    pub fn begin_refresh_attempt(&self, refresh_kind: &str, target: &str) -> anyhow::Result<Uuid> {
        let id = Uuid::new_v4();
        self.conn
            .execute(
                r#"
                INSERT INTO source_refresh_attempts (
                    id, refresh_kind, target, started_at
                ) VALUES (?1, ?2, ?3, ?4)
                "#,
                params![
                    id.to_string(),
                    refresh_kind,
                    target,
                    Utc::now().to_rfc3339(),
                ],
            )
            .context("failed to begin source refresh attempt")?;
        Ok(id)
    }

    pub fn finish_refresh_attempt(
        &self,
        id: Uuid,
        success: bool,
        release_id: Option<&str>,
        summary: Option<&str>,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        let changed = self
            .conn
            .execute(
                r#"
                UPDATE source_refresh_attempts
                SET completed_at = ?2,
                    success = ?3,
                    release_id = ?4,
                    summary = ?5,
                    error = ?6
                WHERE id = ?1
                "#,
                params![
                    id.to_string(),
                    Utc::now().to_rfc3339(),
                    success,
                    release_id,
                    summary,
                    error,
                ],
            )
            .context("failed to finish source refresh attempt")?;
        if changed == 0 {
            return Err(anyhow!("source refresh attempt {id} does not exist"));
        }
        Ok(())
    }

    pub fn event_relation_collection_memberships(
        &self,
    ) -> anyhow::Result<HashMap<Uuid, EventMembership>> {
        let mut memberships = HashMap::<Uuid, EventMembership>::new();

        let mut collection_stmt = self.conn.prepare(
            r#"
            SELECT collection_id, event_id
            FROM event_collection_members
            ORDER BY collection_id, event_id
            "#,
        )?;
        let mut collection_rows = collection_stmt.query([])?;
        while let Some(row) = collection_rows.next()? {
            let raw_collection_id: String = row.get(0)?;
            let raw_event_id: String = row.get(1)?;
            let collection_id = Uuid::parse_str(&raw_collection_id).with_context(|| {
                format!("invalid event collection id in membership index: {raw_collection_id}")
            })?;
            let event_id = Uuid::parse_str(&raw_event_id).with_context(|| {
                format!("invalid event id in collection membership index: {raw_event_id}")
            })?;
            memberships
                .entry(event_id)
                .or_default()
                .collection_ids
                .insert(collection_id);
        }

        let mut relation_stmt = self.conn.prepare(
            r#"
            SELECT from_event_id, to_event_id, relation_type
            FROM event_relations
            ORDER BY id
            "#,
        )?;
        let mut relation_rows = relation_stmt.query([])?;
        while let Some(row) = relation_rows.next()? {
            let raw_from_event_id: String = row.get(0)?;
            let raw_to_event_id: String = row.get(1)?;
            let relation_type: String = row.get(2)?;
            let from_event_id = Uuid::parse_str(&raw_from_event_id).with_context(|| {
                format!("invalid relation source event id: {raw_from_event_id}")
            })?;
            let to_event_id = Uuid::parse_str(&raw_to_event_id)
                .with_context(|| format!("invalid relation target event id: {raw_to_event_id}"))?;
            memberships
                .entry(from_event_id)
                .or_default()
                .outgoing_relation_types
                .insert(relation_type.clone());
            memberships
                .entry(to_event_id)
                .or_default()
                .incoming_relation_types
                .insert(relation_type);
        }

        Ok(memberships)
    }

    pub fn source_refresh_attempts(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<SourceRefreshAttempt>> {
        let limit = i64::try_from(limit).context("refresh attempt limit is too large")?;
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                id, refresh_kind, target, started_at, completed_at,
                success, release_id, summary, error
            FROM source_refresh_attempts
            ORDER BY started_at DESC, id DESC
            LIMIT ?1
            "#,
        )?;

        let mut rows = stmt.query(params![limit])?;
        let mut attempts = Vec::new();
        while let Some(row) = rows.next()? {
            let raw_id: String = row.get(0)?;
            attempts.push(SourceRefreshAttempt {
                id: Uuid::parse_str(&raw_id)
                    .with_context(|| format!("invalid source refresh attempt id: {raw_id}"))?,
                refresh_kind: row.get(1)?,
                target: row.get(2)?,
                started_at: row.get(3)?,
                completed_at: row.get(4)?,
                success: row.get(5)?,
                release_id: row.get(6)?,
                summary: row.get(7)?,
                error: row.get(8)?,
            });
        }
        Ok(attempts)
    }

    pub fn list_saved_views(&self) -> anyhow::Result<Vec<SavedView>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                id, name, query_json, hidden_source_ids_json,
                calendar_view_json, calendar_layout,
                group_by_json, sort_rules_json, color_by_json, color_rules_json,
                composition_layers_json, overlays_json, table_columns_json,
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
        let mut prospective = self.list_saved_views()?;
        if let Some(index) = prospective.iter().position(|saved| saved.id == view.id) {
            prospective[index] = view.clone();
        } else {
            prospective.push(view.clone());
        }

        if let Some(cycle) = saved_view_reference_cycle(&prospective, view.id) {
            let names = cycle
                .into_iter()
                .map(|id| {
                    prospective
                        .iter()
                        .find(|saved| saved.id == id)
                        .map_or_else(|| id.to_string(), |saved| saved.name.clone())
                })
                .collect::<Vec<_>>();
            anyhow::bail!(
                "saved-view composition cycle rejected: {}",
                names.join(" -> ")
            );
        }

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
        let table_columns_json = serde_json::to_string(&view.table_columns)
            .context("failed to encode saved-view table columns")?;

        self.conn
            .execute(
                r#"
                INSERT INTO saved_views (
                    id, name, query_json, hidden_source_ids_json,
                    calendar_view_json, calendar_layout,
                    group_by_json, sort_rules_json, color_by_json, color_rules_json,
                    composition_layers_json, overlays_json, table_columns_json,
                    display_timezone, week_start_monday
                ) VALUES (
                    :id, :name, :query_json, :hidden_source_ids_json,
                    :calendar_view_json, :calendar_layout,
                    :group_by_json, :sort_rules_json, :color_by_json, :color_rules_json,
                    :composition_layers_json, :overlays_json, :table_columns_json,
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
                    table_columns_json = excluded.table_columns_json,
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
                    ":table_columns_json": table_columns_json,
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

    pub fn upsert_event_provenance_record(
        &self,
        record: &EventProvenanceRecord,
    ) -> anyhow::Result<()> {
        record
            .validate()
            .context("invalid event provenance record")?;
        let properties_json = serde_json::to_string(&record.properties)
            .context("failed to encode event provenance properties")?;
        self.conn
            .execute(
                r#"
                INSERT INTO event_provenance_records (
                    id, event_id, role, reference, source_id,
                    note, properties_json, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                ON CONFLICT(id) DO UPDATE SET
                    event_id = excluded.event_id,
                    role = excluded.role,
                    reference = excluded.reference,
                    source_id = excluded.source_id,
                    note = excluded.note,
                    properties_json = excluded.properties_json,
                    updated_at = excluded.updated_at
                "#,
                params![
                    record.id.to_string(),
                    record.event_id.to_string(),
                    record.role.as_str(),
                    record.reference,
                    record.source_id.map(|value| value.to_string()),
                    record.note,
                    properties_json,
                    record.created_at.to_rfc3339(),
                    record.updated_at.to_rfc3339(),
                ],
            )
            .context("failed to upsert event provenance record")?;
        Ok(())
    }

    pub fn event_provenance_record_by_id(
        &self,
        id: Uuid,
    ) -> anyhow::Result<Option<EventProvenanceRecord>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, event_id, role, reference, source_id,
                   note, properties_json, created_at, updated_at
            FROM event_provenance_records
            WHERE id = ?1
            "#,
        )?;
        stmt.query_row(params![id.to_string()], decode_event_provenance_record)
            .optional()
            .context("failed to query event provenance record by id")
    }

    pub fn event_provenance_records_for_event(
        &self,
        event_id: Uuid,
    ) -> anyhow::Result<Vec<EventProvenanceRecord>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, event_id, role, reference, source_id,
                   note, properties_json, created_at, updated_at
            FROM event_provenance_records
            WHERE event_id = ?1
            ORDER BY role, reference COLLATE NOCASE, created_at, id
            "#,
        )?;
        let mut rows = stmt.query(params![event_id.to_string()])?;
        let mut records = Vec::new();
        while let Some(row) = rows.next()? {
            records.push(decode_event_provenance_record(row)?);
        }
        Ok(records)
    }

    pub fn event_provenance_records_for_source(
        &self,
        source_id: Uuid,
    ) -> anyhow::Result<Vec<EventProvenanceRecord>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, event_id, role, reference, source_id,
                   note, properties_json, created_at, updated_at
            FROM event_provenance_records
            WHERE source_id = ?1
            ORDER BY event_id, role, reference COLLATE NOCASE, id
            "#,
        )?;
        let mut rows = stmt.query(params![source_id.to_string()])?;
        let mut records = Vec::new();
        while let Some(row) = rows.next()? {
            records.push(decode_event_provenance_record(row)?);
        }
        Ok(records)
    }

    pub fn list_event_provenance_records(&self) -> anyhow::Result<Vec<EventProvenanceRecord>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, event_id, role, reference, source_id,
                   note, properties_json, created_at, updated_at
            FROM event_provenance_records
            ORDER BY event_id, role, reference COLLATE NOCASE, id
            "#,
        )?;
        let mut rows = stmt.query([])?;
        let mut records = Vec::new();
        while let Some(row) = rows.next()? {
            records.push(decode_event_provenance_record(row)?);
        }
        Ok(records)
    }

    pub fn delete_event_provenance_record(&self, id: Uuid) -> anyhow::Result<bool> {
        let changed = self
            .conn
            .execute(
                "DELETE FROM event_provenance_records WHERE id = ?1",
                params![id.to_string()],
            )
            .context("failed to delete event provenance record")?;
        Ok(changed != 0)
    }

    pub fn upsert_event_annotation(&self, annotation: &EventAnnotation) -> anyhow::Result<()> {
        annotation.validate().context("invalid event annotation")?;
        let value_json = serde_json::to_string(&annotation.value)
            .context("failed to encode annotation value")?;
        self.conn
            .execute(
                r#"
                INSERT INTO event_annotations (
                    id, event_id, kind, value_json, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ON CONFLICT(id) DO UPDATE SET
                    event_id = excluded.event_id,
                    kind = excluded.kind,
                    value_json = excluded.value_json,
                    updated_at = excluded.updated_at
                "#,
                params![
                    annotation.id.to_string(),
                    annotation.event_id.to_string(),
                    annotation.kind,
                    value_json,
                    annotation.created_at.to_rfc3339(),
                    annotation.updated_at.to_rfc3339(),
                ],
            )
            .context("failed to upsert event annotation")?;
        Ok(())
    }

    pub fn event_annotation_by_id(&self, id: Uuid) -> anyhow::Result<Option<EventAnnotation>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, event_id, kind, value_json, created_at, updated_at
            FROM event_annotations
            WHERE id = ?1
            "#,
        )?;
        stmt.query_row(params![id.to_string()], decode_event_annotation)
            .optional()
            .context("failed to query event annotation by id")
    }

    pub fn event_annotations_for_event(
        &self,
        event_id: Uuid,
    ) -> anyhow::Result<Vec<EventAnnotation>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, event_id, kind, value_json, created_at, updated_at
            FROM event_annotations
            WHERE event_id = ?1
            ORDER BY kind COLLATE NOCASE, created_at, id
            "#,
        )?;
        let mut rows = stmt.query(params![event_id.to_string()])?;
        let mut annotations = Vec::new();
        while let Some(row) = rows.next()? {
            annotations.push(decode_event_annotation(row)?);
        }
        Ok(annotations)
    }

    pub fn list_event_annotations(&self) -> anyhow::Result<Vec<EventAnnotation>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, event_id, kind, value_json, created_at, updated_at
            FROM event_annotations
            ORDER BY event_id, kind COLLATE NOCASE, created_at, id
            "#,
        )?;
        let mut rows = stmt.query([])?;
        let mut annotations = Vec::new();
        while let Some(row) = rows.next()? {
            annotations.push(decode_event_annotation(row)?);
        }
        Ok(annotations)
    }

    pub fn delete_event_annotation(&self, id: Uuid) -> anyhow::Result<bool> {
        let changed = self
            .conn
            .execute(
                "DELETE FROM event_annotations WHERE id = ?1",
                params![id.to_string()],
            )
            .context("failed to delete event annotation")?;
        Ok(changed != 0)
    }

    pub fn upsert_event_identity_assessment(
        &self,
        assessment: &EventIdentityAssessment,
    ) -> anyhow::Result<()> {
        assessment
            .validate()
            .context("invalid event identity assessment")?;
        let properties_json = serde_json::to_string(&assessment.properties)
            .context("failed to encode event identity assessment properties")?;
        self.conn
            .execute(
                r#"
                INSERT INTO event_identity_assessments (
                    id, left_event_id, right_event_id, state,
                    confidence, rationale, properties_json,
                    created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                ON CONFLICT(id) DO UPDATE SET
                    left_event_id = excluded.left_event_id,
                    right_event_id = excluded.right_event_id,
                    state = excluded.state,
                    confidence = excluded.confidence,
                    rationale = excluded.rationale,
                    properties_json = excluded.properties_json,
                    updated_at = excluded.updated_at
                "#,
                params![
                    assessment.id.to_string(),
                    assessment.left_event_id.to_string(),
                    assessment.right_event_id.to_string(),
                    assessment.state.as_str(),
                    assessment.confidence,
                    assessment.rationale,
                    properties_json,
                    assessment.created_at.to_rfc3339(),
                    assessment.updated_at.to_rfc3339(),
                ],
            )
            .context("failed to upsert event identity assessment")?;
        Ok(())
    }

    pub fn event_identity_assessment_by_id(
        &self,
        id: Uuid,
    ) -> anyhow::Result<Option<EventIdentityAssessment>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, left_event_id, right_event_id, state,
                   confidence, rationale, properties_json,
                   created_at, updated_at
            FROM event_identity_assessments
            WHERE id = ?1
            "#,
        )?;
        stmt.query_row(params![id.to_string()], decode_event_identity_assessment)
            .optional()
            .context("failed to query event identity assessment by id")
    }

    pub fn event_identity_assessment_between(
        &self,
        first_event_id: Uuid,
        second_event_id: Uuid,
    ) -> anyhow::Result<Option<EventIdentityAssessment>> {
        if first_event_id == second_event_id {
            return Ok(None);
        }
        let (left, right) = if first_event_id <= second_event_id {
            (first_event_id, second_event_id)
        } else {
            (second_event_id, first_event_id)
        };
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, left_event_id, right_event_id, state,
                   confidence, rationale, properties_json,
                   created_at, updated_at
            FROM event_identity_assessments
            WHERE left_event_id = ?1 AND right_event_id = ?2
            "#,
        )?;
        stmt.query_row(
            params![left.to_string(), right.to_string()],
            decode_event_identity_assessment,
        )
        .optional()
        .context("failed to query event identity assessment pair")
    }

    pub fn event_identity_assessments_for_event(
        &self,
        event_id: Uuid,
    ) -> anyhow::Result<Vec<EventIdentityAssessment>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, left_event_id, right_event_id, state,
                   confidence, rationale, properties_json,
                   created_at, updated_at
            FROM event_identity_assessments
            WHERE left_event_id = ?1 OR right_event_id = ?1
            ORDER BY state, left_event_id, right_event_id, id
            "#,
        )?;
        let mut rows = stmt.query(params![event_id.to_string()])?;
        let mut assessments = Vec::new();
        while let Some(row) = rows.next()? {
            assessments.push(decode_event_identity_assessment(row)?);
        }
        Ok(assessments)
    }

    pub fn list_event_identity_assessments(&self) -> anyhow::Result<Vec<EventIdentityAssessment>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, left_event_id, right_event_id, state,
                   confidence, rationale, properties_json,
                   created_at, updated_at
            FROM event_identity_assessments
            ORDER BY state, left_event_id, right_event_id, id
            "#,
        )?;
        let mut rows = stmt.query([])?;
        let mut assessments = Vec::new();
        while let Some(row) = rows.next()? {
            assessments.push(decode_event_identity_assessment(row)?);
        }
        Ok(assessments)
    }

    pub fn delete_event_identity_assessment(&self, id: Uuid) -> anyhow::Result<bool> {
        let changed = self
            .conn
            .execute(
                "DELETE FROM event_identity_assessments WHERE id = ?1",
                params![id.to_string()],
            )
            .context("failed to delete event identity assessment")?;
        Ok(changed != 0)
    }

    pub fn upsert_event_relation(&self, relation: &EventRelation) -> anyhow::Result<()> {
        relation.validate().context("invalid event relation")?;
        let properties_json = serde_json::to_string(&relation.properties)
            .context("failed to encode event relation properties")?;
        self.conn
            .execute(
                r#"
                INSERT INTO event_relations (
                    id, from_event_id, to_event_id, relation_type,
                    properties_json, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ON CONFLICT(id) DO UPDATE SET
                    from_event_id = excluded.from_event_id,
                    to_event_id = excluded.to_event_id,
                    relation_type = excluded.relation_type,
                    properties_json = excluded.properties_json,
                    updated_at = excluded.updated_at
                "#,
                params![
                    relation.id.to_string(),
                    relation.from_event_id.to_string(),
                    relation.to_event_id.to_string(),
                    relation.relation_type,
                    properties_json,
                    relation.created_at.to_rfc3339(),
                    relation.updated_at.to_rfc3339(),
                ],
            )
            .context("failed to upsert event relation")?;
        Ok(())
    }

    pub fn event_relation_by_id(&self, id: Uuid) -> anyhow::Result<Option<EventRelation>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, from_event_id, to_event_id, relation_type,
                   properties_json, created_at, updated_at
            FROM event_relations
            WHERE id = ?1
            "#,
        )?;
        stmt.query_row(params![id.to_string()], decode_event_relation)
            .optional()
            .context("failed to query event relation by id")
    }

    pub fn list_event_relations(&self) -> anyhow::Result<Vec<EventRelation>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, from_event_id, to_event_id, relation_type,
                   properties_json, created_at, updated_at
            FROM event_relations
            ORDER BY relation_type COLLATE NOCASE, from_event_id, to_event_id, id
            "#,
        )?;
        let mut rows = stmt.query([])?;
        let mut relations = Vec::new();
        while let Some(row) = rows.next()? {
            relations.push(decode_event_relation(row)?);
        }
        Ok(relations)
    }

    pub fn search_event_titles(
        &self,
        query: &str,
        exclude_event_id: Option<Uuid>,
        limit: usize,
    ) -> anyhow::Result<Vec<(Uuid, String)>> {
        let query = query.trim();
        if query.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit).context("event search limit is too large")?;
        let exclude = exclude_event_id.map(|id| id.to_string());
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, normalized_title
            FROM temporal_events
            WHERE (
                instr(lower(normalized_title), lower(?1)) > 0
                OR instr(lower(COALESCE(upstream_event_ref, '')), lower(?1)) > 0
                OR instr(lower(COALESCE(source_record_key, '')), lower(?1)) > 0
            )
              AND (?2 IS NULL OR id != ?2)
            ORDER BY
                CASE WHEN lower(normalized_title) = lower(?1) THEN 0 ELSE 1 END,
                normalized_title COLLATE NOCASE,
                id
            LIMIT ?3
            "#,
        )?;
        let mut rows = stmt.query(params![query, exclude, limit])?;
        let mut matches = Vec::new();
        while let Some(row) = rows.next()? {
            let raw_id: String = row.get(0)?;
            let id = Uuid::parse_str(&raw_id)
                .with_context(|| format!("invalid event id in title search: {raw_id}"))?;
            matches.push((id, row.get(1)?));
        }
        Ok(matches)
    }

    pub fn event_relations_for_event(&self, event_id: Uuid) -> anyhow::Result<Vec<EventRelation>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, from_event_id, to_event_id, relation_type,
                   properties_json, created_at, updated_at
            FROM event_relations
            WHERE from_event_id = ?1 OR to_event_id = ?1
            ORDER BY relation_type COLLATE NOCASE, from_event_id, to_event_id, id
            "#,
        )?;
        let mut rows = stmt.query(params![event_id.to_string()])?;
        let mut relations = Vec::new();
        while let Some(row) = rows.next()? {
            relations.push(decode_event_relation(row)?);
        }
        Ok(relations)
    }

    pub fn delete_event_relation(&self, id: Uuid) -> anyhow::Result<bool> {
        let changed = self
            .conn
            .execute(
                "DELETE FROM event_relations WHERE id = ?1",
                params![id.to_string()],
            )
            .context("failed to delete event relation")?;
        Ok(changed != 0)
    }

    pub fn upsert_event_collection(&self, collection: &EventCollection) -> anyhow::Result<()> {
        collection.validate().context("invalid event collection")?;
        let properties_json = serde_json::to_string(&collection.properties)
            .context("failed to encode event collection properties")?;
        self.conn
            .execute(
                r#"
                INSERT INTO event_collections (
                    id, name, description, ordered,
                    properties_json, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    description = excluded.description,
                    ordered = excluded.ordered,
                    properties_json = excluded.properties_json,
                    updated_at = excluded.updated_at
                "#,
                params![
                    collection.id.to_string(),
                    collection.name,
                    collection.description,
                    collection.ordered,
                    properties_json,
                    collection.created_at.to_rfc3339(),
                    collection.updated_at.to_rfc3339(),
                ],
            )
            .context("failed to upsert event collection")?;
        Ok(())
    }

    pub fn event_collection_by_id(&self, id: Uuid) -> anyhow::Result<Option<EventCollection>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, name, description, ordered,
                   properties_json, created_at, updated_at
            FROM event_collections
            WHERE id = ?1
            "#,
        )?;
        stmt.query_row(params![id.to_string()], decode_event_collection)
            .optional()
            .context("failed to query event collection by id")
    }

    pub fn list_event_collections(&self) -> anyhow::Result<Vec<EventCollection>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, name, description, ordered,
                   properties_json, created_at, updated_at
            FROM event_collections
            ORDER BY name COLLATE NOCASE, id
            "#,
        )?;
        let mut rows = stmt.query([])?;
        let mut collections = Vec::new();
        while let Some(row) = rows.next()? {
            collections.push(decode_event_collection(row)?);
        }
        Ok(collections)
    }

    pub fn collections_for_event(&self, event_id: Uuid) -> anyhow::Result<Vec<EventCollection>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT c.id, c.name, c.description, c.ordered,
                   c.properties_json, c.created_at, c.updated_at
            FROM event_collections c
            JOIN event_collection_members m ON m.collection_id = c.id
            WHERE m.event_id = ?1
            ORDER BY c.name COLLATE NOCASE, c.id
            "#,
        )?;
        let mut rows = stmt.query(params![event_id.to_string()])?;
        let mut collections = Vec::new();
        while let Some(row) = rows.next()? {
            collections.push(decode_event_collection(row)?);
        }
        Ok(collections)
    }

    pub fn replace_event_collection_members(
        &self,
        collection_id: Uuid,
        event_ids: &[Uuid],
    ) -> anyhow::Result<()> {
        let collection = self
            .event_collection_by_id(collection_id)?
            .ok_or_else(|| anyhow!("event collection {collection_id} does not exist"))?;

        let unique = event_ids.iter().copied().collect::<BTreeSet<_>>();
        if unique.len() != event_ids.len() {
            return Err(anyhow!(
                "event collection {collection_id} cannot contain the same event twice"
            ));
        }

        let owns_transaction = self.conn.is_autocommit();
        if owns_transaction {
            self.conn
                .execute_batch("BEGIN IMMEDIATE")
                .context("failed to begin event collection membership transaction")?;
        }

        let result = (|| -> anyhow::Result<()> {
            self.conn
                .execute(
                    "DELETE FROM event_collection_members WHERE collection_id = ?1",
                    params![collection_id.to_string()],
                )
                .context("failed to clear event collection members")?;

            for (index, event_id) in event_ids.iter().enumerate() {
                let position = if collection.ordered {
                    Some(i64::try_from(index).context("collection position is too large")?)
                } else {
                    None
                };
                self.conn
                    .execute(
                        r#"
                        INSERT INTO event_collection_members (
                            collection_id, event_id, position
                        ) VALUES (?1, ?2, ?3)
                        "#,
                        params![collection_id.to_string(), event_id.to_string(), position,],
                    )
                    .with_context(|| {
                        format!("failed to add event {event_id} to collection {collection_id}")
                    })?;
            }
            Ok(())
        })();

        if !owns_transaction {
            return result;
        }

        match result {
            Ok(()) => self
                .conn
                .execute_batch("COMMIT")
                .context("failed to commit event collection membership transaction"),
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    pub fn list_event_collection_members(&self) -> anyhow::Result<Vec<EventCollectionMember>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT collection_id, event_id, position
            FROM event_collection_members
            ORDER BY
                collection_id,
                CASE WHEN position IS NULL THEN 1 ELSE 0 END,
                position,
                event_id
            "#,
        )?;
        let mut rows = stmt.query([])?;
        let mut members = Vec::new();
        while let Some(row) = rows.next()? {
            members.push(decode_event_collection_member(row)?);
        }
        Ok(members)
    }

    pub fn event_collection_members(
        &self,
        collection_id: Uuid,
    ) -> anyhow::Result<Vec<EventCollectionMember>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT collection_id, event_id, position
            FROM event_collection_members
            WHERE collection_id = ?1
            ORDER BY
                CASE WHEN position IS NULL THEN 1 ELSE 0 END,
                position,
                event_id
            "#,
        )?;
        let mut rows = stmt.query(params![collection_id.to_string()])?;
        let mut members = Vec::new();
        while let Some(row) = rows.next()? {
            members.push(decode_event_collection_member(row)?);
        }
        Ok(members)
    }

    pub fn events_for_collection(&self, collection_id: Uuid) -> anyhow::Result<Vec<TemporalEvent>> {
        let sql = event_select_sql(
            "WHERE id IN (
                SELECT event_id
                FROM event_collection_members
                WHERE collection_id = ?1
            )
            ORDER BY CASE WHEN (
                SELECT position
                FROM event_collection_members
                WHERE collection_id = ?1
                  AND event_id = temporal_events.id
            ) IS NULL THEN 1 ELSE 0 END,
            (
                SELECT position
                FROM event_collection_members
                WHERE collection_id = ?1
                  AND event_id = temporal_events.id
            ),
            normalized_title COLLATE NOCASE,
            id",
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(params![collection_id.to_string()])?;
        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            events.push(decode_event(row)?);
        }
        Ok(events)
    }

    pub fn delete_event_collection(&self, id: Uuid) -> anyhow::Result<bool> {
        let changed = self
            .conn
            .execute(
                "DELETE FROM event_collections WHERE id = ?1",
                params![id.to_string()],
            )
            .context("failed to delete event collection")?;
        Ok(changed != 0)
    }

    pub fn upsert_event(&self, event: &TemporalEvent) -> anyhow::Result<()> {
        event
            .validate_recurrence()
            .context("invalid event recurrence definition")?;
        event
            .validate_time_uncertainty()
            .context("invalid event temporal uncertainty")?;
        let encoded = EncodedTime::from_time_spec(&event.time)?;
        let assertion_refs_json = encode_string_vec(&event.assertion_refs, "assertion refs")?;
        let source_refs_json = encode_string_vec(&event.source_refs, "source refs")?;
        let provenance_refs_json = encode_string_vec(&event.provenance_refs, "provenance refs")?;
        let time_uncertainty_json = event
            .time_uncertainty
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("failed to encode event temporal uncertainty")?;
        let recurrence_json = event
            .recurrence
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("failed to encode event recurrence")?;
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
                    time_uncertainty_json, recurrence_json,
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
                    :time_uncertainty_json, :recurrence_json,
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
                    time_uncertainty_json = excluded.time_uncertainty_json,
                    recurrence_json = excluded.recurrence_json,
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
                    ":time_uncertainty_json": time_uncertainty_json,
                    ":recurrence_json": recurrence_json,
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
        for (kind, value) in [("reconciled", reconciled_ref), ("event", event_ref)] {
            let Some(value) = value else {
                continue;
            };
            let sql = event_select_sql(
                "WHERE id = (
                    SELECT event_id
                    FROM temporal_event_upstream_identities
                    WHERE identity_kind = ?1 AND identity_value = ?2
                )",
            );
            let mut stmt = self.conn.prepare(&sql)?;
            if let Some(event) = stmt
                .query_row(params![kind, value], decode_event)
                .optional()
                .context("failed to query event by upstream identity alias")?
            {
                return Ok(Some(event));
            }
        }

        // Compatibility fallback for databases populated before the alias table existed.
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

    pub fn begin_taria_release_adoption(&self) -> anyhow::Result<()> {
        if !self.conn.is_autocommit() {
            return Err(anyhow!(
                "cannot begin Taria release adoption inside another transaction"
            ));
        }
        self.conn
            .execute_batch("BEGIN IMMEDIATE")
            .context("failed to begin whole-release Taria adoption transaction")
    }

    pub fn commit_taria_release_adoption(&self) -> anyhow::Result<()> {
        self.conn
            .execute_batch("COMMIT")
            .context("failed to commit whole-release Taria adoption transaction")
    }

    pub fn rollback_taria_release_adoption(&self) -> anyhow::Result<()> {
        if self.conn.is_autocommit() {
            return Ok(());
        }
        self.conn
            .execute_batch("ROLLBACK")
            .context("failed to roll back whole-release Taria adoption transaction")
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

    pub fn link_taria_release_source(
        &self,
        release_id: &str,
        source_id: Uuid,
        projection_ref: &str,
    ) -> anyhow::Result<()> {
        self.conn
            .execute(
                r#"
                INSERT INTO taria_release_sources (
                    release_id, source_id, projection_ref
                ) VALUES (?1, ?2, ?3)
                ON CONFLICT(release_id, source_id) DO UPDATE SET
                    projection_ref = excluded.projection_ref
                "#,
                params![release_id, source_id.to_string(), projection_ref],
            )
            .context("failed to associate source with Taria release")?;
        Ok(())
    }

    pub fn taria_source_ids_for_release(
        &self,
        release_id: Option<&str>,
    ) -> anyhow::Result<BTreeSet<Uuid>> {
        let Some(release_id) = release_id else {
            return Ok(BTreeSet::new());
        };

        let mut stmt = self.conn.prepare(
            r#"
            SELECT source_id
            FROM taria_release_sources
            WHERE release_id = ?1
            ORDER BY source_id
            "#,
        )?;
        let mut rows = stmt.query(params![release_id])?;
        let mut source_ids = BTreeSet::new();
        while let Some(row) = rows.next()? {
            let raw: String = row.get(0)?;
            source_ids.insert(
                Uuid::parse_str(&raw)
                    .with_context(|| format!("invalid Taria release source id: {raw}"))?,
            );
        }
        Ok(source_ids)
    }

    pub fn capture_taria_release_event_snapshots(
        &self,
        release_id: &str,
        event_ids: &BTreeSet<Uuid>,
    ) -> anyhow::Result<usize> {
        let mut captured = 0usize;

        for event_id in event_ids {
            let event = self
                .event_by_id(*event_id)?
                .ok_or_else(|| anyhow!("cannot snapshot missing canonical event {event_id}"))?;
            let time_json = serde_json::to_string(&event.time)
                .context("failed to encode release event time")?;
            let snapshot_json = canonical_event_snapshot_json(&event)?;

            let existing: Option<String> = self
                .conn
                .query_row(
                    r#"
                    SELECT snapshot_json
                    FROM taria_release_event_snapshots
                    WHERE release_id = ?1 AND event_id = ?2
                    "#,
                    params![release_id, event_id.to_string()],
                    |row| row.get(0),
                )
                .optional()
                .context("failed to check release event snapshot immutability")?;

            if existing.is_some() {
                continue;
            }

            self.conn
                .execute(
                    r#"
                    INSERT INTO taria_release_event_snapshots (
                        release_id, event_id, normalized_title, status,
                        time_json, snapshot_json
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                    "#,
                    params![
                        release_id,
                        event_id.to_string(),
                        event.normalized_title,
                        event.status.as_str(),
                        time_json,
                        snapshot_json,
                    ],
                )
                .context("failed to insert Taria release event snapshot")?;
            captured += 1;
        }

        Ok(captured)
    }

    pub fn taria_release_event_snapshot_count(&self, release_id: &str) -> anyhow::Result<u64> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM taria_release_event_snapshots WHERE release_id = ?1",
                params![release_id],
                |row| row.get(0),
            )
            .context("failed to count Taria release event snapshots")?;
        i64_to_u64(count, "release event snapshot count")
    }

    fn taria_release_event_snapshots(
        &self,
        release_id: &str,
    ) -> anyhow::Result<HashMap<Uuid, StoredTariaEventSnapshot>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT event_id, normalized_title, status, time_json
            FROM taria_release_event_snapshots
            WHERE release_id = ?1
            ORDER BY event_id
            "#,
        )?;
        let mut rows = stmt.query(params![release_id])?;
        let mut snapshots = HashMap::new();

        while let Some(row) = rows.next()? {
            let raw_event_id: String = row.get(0)?;
            let event_id = Uuid::parse_str(&raw_event_id)
                .with_context(|| format!("invalid release snapshot event id: {raw_event_id}"))?;
            snapshots.insert(
                event_id,
                StoredTariaEventSnapshot {
                    normalized_title: row.get(1)?,
                    status: row.get(2)?,
                    time_json: row.get(3)?,
                },
            );
        }

        Ok(snapshots)
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

        let owns_transaction = self.conn.is_autocommit();
        if owns_transaction {
            self.conn
                .execute_batch("BEGIN IMMEDIATE")
                .context("failed to begin CalendarSet transaction")?;
        }

        let result = (|| -> anyhow::Result<TariaCalendarSetImportResult> {
            self.conn
                .execute(
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

            self.conn
                .execute(
                    r#"
                INSERT INTO taria_release_calendar_sets (
                    release_id, calendar_set_id, bundle_ref
                ) VALUES (?1, ?2, ?3)
                ON CONFLICT(release_id, calendar_set_id, bundle_ref) DO NOTHING
                "#,
                    params![set.release_id, set.calendar_set_id, set.bundle_ref],
                )
                .context("failed to associate CalendarSet with Taria release")?;

            self.conn
                .execute(
                    "DELETE FROM taria_calendar_memberships WHERE calendar_set_id = ?1",
                    params![set.calendar_set_id],
                )
                .context("failed to clear prior CalendarSet memberships")?;
            self.conn
                .execute(
                    "DELETE FROM taria_projected_calendars WHERE calendar_set_id = ?1",
                    params![set.calendar_set_id],
                )
                .context("failed to clear prior projected calendars")?;

            for calendar in calendars {
                self.conn
                    .execute(
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
                let event_id: Option<String> = self
                    .conn
                    .query_row(
                        r#"
                        SELECT event_id
                        FROM temporal_event_upstream_identities
                        WHERE identity_kind = 'reconciled' AND identity_value = ?1
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

                self.conn
                    .execute(
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
                if owns_transaction {
                    self.conn
                        .execute_batch("COMMIT")
                        .context("failed to commit CalendarSet transaction")?;
                }
                Ok(result)
            }
            Err(error) => {
                if owns_transaction {
                    let _ = self.conn.execute_batch("ROLLBACK");
                }
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
                    SELECT event_id
                    FROM temporal_event_upstream_identities
                    WHERE identity_kind = 'reconciled'
                      AND identity_value =
                        taria_calendar_memberships.reconciled_event_ref
                    LIMIT 1
                )
                WHERE event_id IS NULL
                  AND EXISTS (
                    SELECT 1
                    FROM temporal_event_upstream_identities
                    WHERE identity_kind = 'reconciled'
                      AND identity_value =
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

    pub fn taria_release_status(
        &self,
        release_id: Option<&str>,
    ) -> anyhow::Result<Option<TariaReleaseStatusRecord>> {
        let Some(release_id) = release_id else {
            return Ok(None);
        };

        self.conn
            .query_row(
                r#"
                SELECT release_id, channel, status, production_complete,
                       generated_at, adopted_at, coverage_json, manifest_json
                FROM taria_releases
                WHERE release_id = ?1
                "#,
                params![release_id],
                |row| {
                    Ok(TariaReleaseStatusRecord {
                        release_id: row.get(0)?,
                        channel: row.get(1)?,
                        status: row.get(2)?,
                        production_complete: row.get(3)?,
                        generated_at: row.get(4)?,
                        adopted_at: row.get(5)?,
                        coverage_json: row.get(6)?,
                        manifest_json: row.get(7)?,
                    })
                },
            )
            .optional()
            .context("failed to query adopted Taria release status")
    }

    pub fn taria_release_history(&self) -> anyhow::Result<Vec<TariaReleaseHistoryEntry>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                release.release_id,
                release.channel,
                release.status,
                release.production_complete,
                release.generated_at,
                release.adopted_at,
                (
                    SELECT COUNT(DISTINCT link.bundle_ref)
                    FROM taria_release_calendar_sets AS link
                    WHERE link.release_id = release.release_id
                ),
                (
                    SELECT COUNT(DISTINCT link.calendar_set_id)
                    FROM taria_release_calendar_sets AS link
                    WHERE link.release_id = release.release_id
                ),
                (
                    SELECT COUNT(DISTINCT calendar.calendar_id)
                    FROM taria_projected_calendars AS calendar
                    JOIN taria_release_calendar_sets AS link
                      ON link.calendar_set_id = calendar.calendar_set_id
                    WHERE link.release_id = release.release_id
                ),
                (
                    SELECT COUNT(DISTINCT source_link.source_id)
                    FROM taria_release_sources AS source_link
                    WHERE source_link.release_id = release.release_id
                ),
                (
                    SELECT COUNT(*)
                    FROM taria_release_event_snapshots AS snapshot
                    WHERE snapshot.release_id = release.release_id
                ),
                (
                    SELECT COUNT(DISTINCT membership.event_id)
                    FROM taria_calendar_memberships AS membership
                    JOIN taria_release_calendar_sets AS link
                      ON link.calendar_set_id = membership.calendar_set_id
                    WHERE link.release_id = release.release_id
                      AND membership.event_id IS NOT NULL
                )
            FROM taria_releases AS release
            ORDER BY release.adopted_at DESC, release.release_id DESC
            "#,
        )?;
        let mut rows = stmt.query([])?;
        let mut history = Vec::new();
        while let Some(row) = rows.next()? {
            history.push(TariaReleaseHistoryEntry {
                release_id: row.get(0)?,
                channel: row.get(1)?,
                status: row.get(2)?,
                production_complete: row.get(3)?,
                generated_at: row.get(4)?,
                adopted_at: row.get(5)?,
                bundle_count: i64_to_u64(row.get(6)?, "bundle count")?,
                calendar_set_count: i64_to_u64(row.get(7)?, "CalendarSet count")?,
                projected_calendar_count: i64_to_u64(row.get(8)?, "projected calendar count")?,
                source_count: i64_to_u64(row.get(9)?, "source count")?,
                snapshot_event_count: i64_to_u64(row.get(10)?, "snapshot event count")?,
                resolved_member_event_count: i64_to_u64(
                    row.get(11)?,
                    "resolved member event count",
                )?,
            });
        }
        Ok(history)
    }

    pub fn taria_previous_release_diff(
        &self,
        release_id: &str,
    ) -> anyhow::Result<Option<TariaReleaseDiff>> {
        let current: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT channel, adopted_at FROM taria_releases WHERE release_id = ?1",
                params![release_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .context("failed to query Taria release for history diff")?;
        let Some((channel, adopted_at)) = current else {
            return Ok(None);
        };

        let previous: Option<String> = self
            .conn
            .query_row(
                r#"
                SELECT release_id
                FROM taria_releases
                WHERE channel = ?1
                  AND (
                      adopted_at < ?2
                      OR (adopted_at = ?2 AND release_id < ?3)
                  )
                ORDER BY adopted_at DESC, release_id DESC
                LIMIT 1
                "#,
                params![channel, adopted_at, release_id],
                |row| row.get(0),
            )
            .optional()
            .context("failed to query previous Taria release")?;

        previous
            .as_deref()
            .map(|previous| self.taria_release_diff(previous, release_id))
            .transpose()
    }

    pub fn taria_release_diff(
        &self,
        from_release_id: &str,
        to_release_id: &str,
    ) -> anyhow::Result<TariaReleaseDiff> {
        let from_sources = self.taria_release_string_set(
            from_release_id,
            r#"
            SELECT DISTINCT projection_ref
            FROM taria_release_sources
            WHERE release_id = ?1
            "#,
        )?;
        let to_sources = self.taria_release_string_set(
            to_release_id,
            r#"
            SELECT DISTINCT projection_ref
            FROM taria_release_sources
            WHERE release_id = ?1
            "#,
        )?;

        let from_bundles = self.taria_release_string_set(
            from_release_id,
            r#"
            SELECT DISTINCT bundle_ref
            FROM taria_release_calendar_sets
            WHERE release_id = ?1
            "#,
        )?;
        let to_bundles = self.taria_release_string_set(
            to_release_id,
            r#"
            SELECT DISTINCT bundle_ref
            FROM taria_release_calendar_sets
            WHERE release_id = ?1
            "#,
        )?;

        let from_calendars = self.taria_release_string_set(
            from_release_id,
            r#"
            SELECT DISTINCT calendar.calendar_id
            FROM taria_projected_calendars AS calendar
            JOIN taria_release_calendar_sets AS link
              ON link.calendar_set_id = calendar.calendar_set_id
            WHERE link.release_id = ?1
            "#,
        )?;
        let to_calendars = self.taria_release_string_set(
            to_release_id,
            r#"
            SELECT DISTINCT calendar.calendar_id
            FROM taria_projected_calendars AS calendar
            JOIN taria_release_calendar_sets AS link
              ON link.calendar_set_id = calendar.calendar_set_id
            WHERE link.release_id = ?1
            "#,
        )?;

        let from_events = self.taria_release_string_set(
            from_release_id,
            r#"
            SELECT DISTINCT membership.event_id
            FROM taria_calendar_memberships AS membership
            JOIN taria_release_calendar_sets AS link
              ON link.calendar_set_id = membership.calendar_set_id
            WHERE link.release_id = ?1
              AND membership.event_id IS NOT NULL
            "#,
        )?;
        let to_events = self.taria_release_string_set(
            to_release_id,
            r#"
            SELECT DISTINCT membership.event_id
            FROM taria_calendar_memberships AS membership
            JOIN taria_release_calendar_sets AS link
              ON link.calendar_set_id = membership.calendar_set_id
            WHERE link.release_id = ?1
              AND membership.event_id IS NOT NULL
            "#,
        )?;

        let from_snapshots = self.taria_release_event_snapshots(from_release_id)?;
        let to_snapshots = self.taria_release_event_snapshots(to_release_id)?;
        let from_snapshot_ids = from_snapshots.keys().copied().collect::<BTreeSet<_>>();
        let to_snapshot_ids = to_snapshots.keys().copied().collect::<BTreeSet<_>>();

        let mut moved_event_ids = Vec::new();
        let mut status_changed_event_ids = Vec::new();
        let mut newly_cancelled_event_ids = Vec::new();
        let mut event_changes = Vec::new();

        for event_id in to_snapshot_ids.difference(&from_snapshot_ids) {
            let to = &to_snapshots[event_id];
            event_changes.push(TariaEventChangeDetail {
                event_id: *event_id,
                title: to.normalized_title.clone(),
                added: true,
                removed: false,
                title_changed: false,
                moved: false,
                status_changed: false,
                newly_cancelled: false,
                from_title: None,
                to_title: Some(to.normalized_title.clone()),
                from_status: None,
                to_status: Some(to.status.clone()),
                from_time_json: None,
                to_time_json: Some(to.time_json.clone()),
            });
        }

        for event_id in from_snapshot_ids.difference(&to_snapshot_ids) {
            let from = &from_snapshots[event_id];
            event_changes.push(TariaEventChangeDetail {
                event_id: *event_id,
                title: from.normalized_title.clone(),
                added: false,
                removed: true,
                title_changed: false,
                moved: false,
                status_changed: false,
                newly_cancelled: false,
                from_title: Some(from.normalized_title.clone()),
                to_title: None,
                from_status: Some(from.status.clone()),
                to_status: None,
                from_time_json: Some(from.time_json.clone()),
                to_time_json: None,
            });
        }

        for event_id in from_snapshot_ids.intersection(&to_snapshot_ids) {
            let from = &from_snapshots[event_id];
            let to = &to_snapshots[event_id];
            let title_changed = from.normalized_title != to.normalized_title;
            let moved = from.time_json != to.time_json;
            let status_changed = from.status != to.status;
            let newly_cancelled = status_changed && to.status == EventStatus::Cancelled.as_str();

            if moved {
                moved_event_ids.push(*event_id);
            }
            if status_changed {
                status_changed_event_ids.push(*event_id);
            }
            if newly_cancelled {
                newly_cancelled_event_ids.push(*event_id);
            }
            if title_changed || moved || status_changed {
                event_changes.push(TariaEventChangeDetail {
                    event_id: *event_id,
                    title: to.normalized_title.clone(),
                    added: false,
                    removed: false,
                    title_changed,
                    moved,
                    status_changed,
                    newly_cancelled,
                    from_title: Some(from.normalized_title.clone()),
                    to_title: Some(to.normalized_title.clone()),
                    from_status: Some(from.status.clone()),
                    to_status: Some(to.status.clone()),
                    from_time_json: Some(from.time_json.clone()),
                    to_time_json: Some(to.time_json.clone()),
                });
            }
        }

        event_changes.sort_by(|left, right| {
            left.title
                .to_ascii_lowercase()
                .cmp(&right.title.to_ascii_lowercase())
                .then_with(|| left.event_id.cmp(&right.event_id))
        });

        Ok(TariaReleaseDiff {
            from_release_id: from_release_id.to_string(),
            to_release_id: to_release_id.to_string(),
            added_source_projection_refs: set_added(&from_sources, &to_sources),
            removed_source_projection_refs: set_added(&to_sources, &from_sources),
            added_bundle_refs: set_added(&from_bundles, &to_bundles),
            removed_bundle_refs: set_added(&to_bundles, &from_bundles),
            added_calendar_ids: set_added(&from_calendars, &to_calendars),
            removed_calendar_ids: set_added(&to_calendars, &from_calendars),
            added_snapshot_event_ids: to_snapshot_ids
                .difference(&from_snapshot_ids)
                .copied()
                .collect(),
            removed_snapshot_event_ids: from_snapshot_ids
                .difference(&to_snapshot_ids)
                .copied()
                .collect(),
            moved_event_ids,
            status_changed_event_ids,
            newly_cancelled_event_ids,
            event_changes,
            added_member_event_ids: set_added(&from_events, &to_events)
                .into_iter()
                .map(|value| Uuid::parse_str(&value).context("invalid stored event UUID"))
                .collect::<anyhow::Result<Vec<_>>>()?,
            removed_member_event_ids: set_added(&to_events, &from_events)
                .into_iter()
                .map(|value| Uuid::parse_str(&value).context("invalid stored event UUID"))
                .collect::<anyhow::Result<Vec<_>>>()?,
        })
    }

    fn taria_release_string_set(
        &self,
        release_id: &str,
        sql: &str,
    ) -> anyhow::Result<std::collections::BTreeSet<String>> {
        let mut stmt = self.conn.prepare(sql)?;
        let mut rows = stmt.query(params![release_id])?;
        let mut values = std::collections::BTreeSet::new();
        while let Some(row) = rows.next()? {
            values.insert(row.get(0)?);
        }
        Ok(values)
    }

    pub fn taria_bundle_refs_for_release(
        &self,
        release_id: Option<&str>,
    ) -> anyhow::Result<Vec<String>> {
        let Some(release_id) = release_id else {
            return Ok(Vec::new());
        };

        let mut stmt = self.conn.prepare(
            r#"
            SELECT DISTINCT bundle_ref
            FROM taria_release_calendar_sets
            WHERE release_id = ?1
            ORDER BY bundle_ref
            "#,
        )?;
        let mut rows = stmt.query(params![release_id])?;
        let mut refs = Vec::new();
        while let Some(row) = rows.next()? {
            refs.push(row.get(0)?);
        }
        Ok(refs)
    }

    pub fn taria_projected_calendar_choices_for_release(
        &self,
        release_id: Option<&str>,
    ) -> anyhow::Result<Vec<TariaProjectedCalendarChoice>> {
        let Some(release_id) = release_id else {
            return Ok(Vec::new());
        };

        let mut stmt = self.conn.prepare(
            r#"
            SELECT release_set.bundle_ref,
                   calendar.calendar_set_id,
                   calendar.calendar_id,
                   calendar.name,
                   calendar.kind
            FROM taria_projected_calendars AS calendar
            JOIN taria_release_calendar_sets AS release_set
              ON release_set.calendar_set_id = calendar.calendar_set_id
            WHERE release_set.release_id = ?1
            ORDER BY release_set.bundle_ref, calendar.name COLLATE NOCASE, calendar.calendar_id
            "#,
        )?;
        let mut rows = stmt.query(params![release_id])?;
        let mut calendars = Vec::new();
        while let Some(row) = rows.next()? {
            calendars.push(TariaProjectedCalendarChoice {
                bundle_ref: row.get(0)?,
                calendar_set_id: row.get(1)?,
                calendar_id: row.get(2)?,
                name: row.get(3)?,
                kind: row.get(4)?,
            });
        }
        Ok(calendars)
    }

    pub fn taria_event_memberships_for_release(
        &self,
        release_id: Option<&str>,
    ) -> anyhow::Result<std::collections::HashMap<Uuid, EventMembership>> {
        let Some(release_id) = release_id else {
            return Ok(std::collections::HashMap::new());
        };

        let mut stmt = self.conn.prepare(
            r#"
            SELECT membership.event_id, release_set.bundle_ref, membership.calendar_id
            FROM taria_calendar_memberships AS membership
            JOIN taria_release_calendar_sets AS release_set
              ON release_set.calendar_set_id = membership.calendar_set_id
            WHERE release_set.release_id = ?1
              AND membership.event_id IS NOT NULL
            ORDER BY membership.event_id, release_set.bundle_ref, membership.calendar_id
            "#,
        )?;
        let mut rows = stmt.query(params![release_id])?;
        let mut memberships = std::collections::HashMap::<Uuid, EventMembership>::new();

        while let Some(row) = rows.next()? {
            let event_id_raw: String = row.get(0)?;
            let event_id = Uuid::parse_str(&event_id_raw)
                .with_context(|| format!("invalid Taria membership event UUID {event_id_raw}"))?;
            let bundle_ref: String = row.get(1)?;
            let calendar_id: String = row.get(2)?;
            let membership = memberships.entry(event_id).or_default();
            membership.bundle_refs.insert(bundle_ref);
            membership.calendar_refs.insert(calendar_id);
        }

        Ok(memberships)
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

    pub fn recurring_events(&self) -> anyhow::Result<Vec<TemporalEvent>> {
        let sql = event_select_sql(
            r#"
            WHERE recurrence_json IS NOT NULL
            ORDER BY normalized_title COLLATE NOCASE, id
            "#,
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query([])?;
        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            events.push(decode_event(row)?);
        }
        Ok(events)
    }

    pub fn merge_canonical_snapshot(
        &self,
        sources: &[TemporalSource],
        events: &[TemporalEvent],
    ) -> anyhow::Result<CanonicalSnapshotMergeResult> {
        self.merge_canonical_snapshot_records(CanonicalSnapshotMergeInput {
            sources,
            events,
            relations: &[],
            collections: &[],
            collection_members: &[],
            identity_assessments: &[],
            annotations: &[],
        })
    }

    pub fn merge_canonical_snapshot_with_topology(
        &self,
        sources: &[TemporalSource],
        events: &[TemporalEvent],
        relations: &[EventRelation],
        collections: &[EventCollection],
        collection_members: &[EventCollectionMember],
    ) -> anyhow::Result<CanonicalSnapshotMergeResult> {
        self.merge_canonical_snapshot_records(CanonicalSnapshotMergeInput {
            sources,
            events,
            relations,
            collections,
            collection_members,
            identity_assessments: &[],
            annotations: &[],
        })
    }

    pub fn merge_canonical_snapshot_with_identity(
        &self,
        sources: &[TemporalSource],
        events: &[TemporalEvent],
        relations: &[EventRelation],
        collections: &[EventCollection],
        collection_members: &[EventCollectionMember],
        identity_assessments: &[EventIdentityAssessment],
    ) -> anyhow::Result<CanonicalSnapshotMergeResult> {
        self.merge_canonical_snapshot_records(CanonicalSnapshotMergeInput {
            sources,
            events,
            relations,
            collections,
            collection_members,
            identity_assessments,
            annotations: &[],
        })
    }

    pub fn merge_canonical_snapshot_records(
        &self,
        input: CanonicalSnapshotMergeInput<'_>,
    ) -> anyhow::Result<CanonicalSnapshotMergeResult> {
        let CanonicalSnapshotMergeInput {
            sources,
            events,
            relations,
            collections,
            collection_members,
            identity_assessments,
            annotations,
        } = input;
        let tx = self
            .conn
            .unchecked_transaction()
            .context("failed to begin canonical snapshot merge transaction")?;

        let result = (|| -> anyhow::Result<CanonicalSnapshotMergeResult> {
            let mut sources_created = 0;
            let mut sources_updated = 0;
            let mut sources_unchanged = 0;
            for source in sources {
                match self.source_by_id(source.id)? {
                    None => {
                        self.upsert_source(source)?;
                        sources_created += 1;
                    }
                    Some(existing) if existing == *source => {
                        sources_unchanged += 1;
                    }
                    Some(_) => {
                        self.upsert_source(source)?;
                        sources_updated += 1;
                    }
                }
            }

            let mut events_created = 0;
            let mut events_updated = 0;
            let mut events_unchanged = 0;
            for event in events {
                event
                    .validate_recurrence()
                    .context("invalid event recurrence in canonical snapshot")?;
                match self.event_by_id(event.id)? {
                    None => {
                        self.upsert_event(event)?;
                        events_created += 1;
                    }
                    Some(existing) if existing == *event => {
                        events_unchanged += 1;
                    }
                    Some(_) => {
                        self.upsert_event(event)?;
                        events_updated += 1;
                    }
                }
            }

            let mut collections_created = 0;
            let mut collections_updated = 0;
            let mut collections_unchanged = 0;
            for collection in collections {
                match self.event_collection_by_id(collection.id)? {
                    None => {
                        self.upsert_event_collection(collection)?;
                        collections_created += 1;
                    }
                    Some(existing) if existing == *collection => {
                        collections_unchanged += 1;
                    }
                    Some(_) => {
                        self.upsert_event_collection(collection)?;
                        collections_updated += 1;
                    }
                }
            }

            let mut relations_created = 0;
            let mut relations_updated = 0;
            let mut relations_unchanged = 0;
            for relation in relations {
                match self.event_relation_by_id(relation.id)? {
                    None => {
                        self.upsert_event_relation(relation)?;
                        relations_created += 1;
                    }
                    Some(existing) if existing == *relation => {
                        relations_unchanged += 1;
                    }
                    Some(_) => {
                        self.upsert_event_relation(relation)?;
                        relations_updated += 1;
                    }
                }
            }

            let mut identity_assessments_created = 0;
            let mut identity_assessments_updated = 0;
            let mut identity_assessments_unchanged = 0;
            for assessment in identity_assessments {
                match self.event_identity_assessment_by_id(assessment.id)? {
                    None => {
                        self.upsert_event_identity_assessment(assessment)?;
                        identity_assessments_created += 1;
                    }
                    Some(existing) if existing == *assessment => {
                        identity_assessments_unchanged += 1;
                    }
                    Some(_) => {
                        self.upsert_event_identity_assessment(assessment)?;
                        identity_assessments_updated += 1;
                    }
                }
            }

            let mut annotations_created = 0;
            let mut annotations_updated = 0;
            let mut annotations_unchanged = 0;
            for annotation in annotations {
                match self.event_annotation_by_id(annotation.id)? {
                    None => {
                        self.upsert_event_annotation(annotation)?;
                        annotations_created += 1;
                    }
                    Some(existing) if existing == *annotation => {
                        annotations_unchanged += 1;
                    }
                    Some(_) => {
                        self.upsert_event_annotation(annotation)?;
                        annotations_updated += 1;
                    }
                }
            }

            let mut collection_memberships_replaced = 0;
            let mut collection_memberships_unchanged = 0;
            for collection in collections {
                let mut desired = collection_members
                    .iter()
                    .filter(|member| member.collection_id == collection.id)
                    .cloned()
                    .collect::<Vec<_>>();
                if collection.ordered {
                    desired.sort_by_key(|member| (member.position, member.event_id));
                } else {
                    desired.sort_by_key(|member| member.event_id);
                }

                let existing = self.event_collection_members(collection.id)?;
                if existing == desired {
                    collection_memberships_unchanged += 1;
                    continue;
                }

                let event_ids = desired
                    .iter()
                    .map(|member| member.event_id)
                    .collect::<Vec<_>>();
                self.replace_event_collection_members(collection.id, &event_ids)?;
                collection_memberships_replaced += 1;
            }

            Ok(CanonicalSnapshotMergeResult {
                sources_created,
                sources_updated,
                sources_unchanged,
                events_created,
                events_updated,
                events_unchanged,
                relations_created,
                relations_updated,
                relations_unchanged,
                collections_created,
                collections_updated,
                collections_unchanged,
                collection_memberships_replaced,
                collection_memberships_unchanged,
                identity_assessments_created,
                identity_assessments_updated,
                identity_assessments_unchanged,
                annotations_created,
                annotations_updated,
                annotations_unchanged,
            })
        })();

        match result {
            Ok(result) => {
                tx.commit()
                    .context("failed to commit canonical snapshot merge")?;
                Ok(result)
            }
            Err(error) => {
                let _ = tx.rollback();
                Err(error)
            }
        }
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

        let owns_transaction = self.conn.is_autocommit();
        if owns_transaction {
            self.conn
                .execute_batch("BEGIN IMMEDIATE")
                .context("failed to begin Taria import transaction")?;
        }

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

                for (identity_kind, identity_value) in [
                    ("event", event.upstream_event_ref.as_deref()),
                    ("reconciled", event.upstream_reconciled_key.as_deref()),
                ] {
                    let Some(identity_value) = identity_value else {
                        continue;
                    };
                    let existing_event_id: Option<String> = self
                        .conn
                        .query_row(
                            r#"
                            SELECT event_id
                            FROM temporal_event_upstream_identities
                            WHERE identity_kind = ?1 AND identity_value = ?2
                            "#,
                            params![identity_kind, identity_value],
                            |row| row.get(0),
                        )
                        .optional()
                        .context("failed to check Taria upstream identity alias")?;
                    if let Some(existing_event_id) = existing_event_id {
                        if existing_event_id != event.id.to_string() {
                            return Err(anyhow!(
                                "Taria upstream identity collision for {}:{}: {} != {}",
                                identity_kind,
                                identity_value,
                                existing_event_id,
                                event.id
                            ));
                        }
                    } else {
                        self.conn
                            .execute(
                                r#"
                            INSERT INTO temporal_event_upstream_identities (
                                identity_kind, identity_value, event_id
                            ) VALUES (?1, ?2, ?3)
                            "#,
                                params![identity_kind, identity_value, event.id.to_string()],
                            )
                            .context("failed to record Taria upstream identity alias")?;
                    }
                }

                self.conn
                    .execute(
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
                if owns_transaction {
                    self.conn
                        .execute_batch("COMMIT")
                        .context("failed to commit Taria import transaction")?;
                }
                Ok(result)
            }
            Err(error) => {
                if owns_transaction {
                    let _ = self.conn.execute_batch("ROLLBACK");
                }
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

fn i64_to_u64(value: i64, label: &str) -> anyhow::Result<u64> {
    u64::try_from(value).with_context(|| format!("{label} cannot be represented as u64"))
}

fn canonical_event_snapshot_json(event: &TemporalEvent) -> anyhow::Result<String> {
    let mut value =
        serde_json::to_value(event).context("failed to encode canonical event snapshot")?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| anyhow!("canonical event snapshot must encode as a JSON object"))?;

    for local_field in ["source_id", "source_record_key", "created_at", "updated_at"] {
        object.remove(local_field);
    }

    serde_json::to_string(&value).context("failed to serialize canonical event snapshot")
}

fn set_added(
    old: &std::collections::BTreeSet<String>,
    new: &std::collections::BTreeSet<String>,
) -> Vec<String> {
    new.difference(old).cloned().collect()
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
        add_event_recurrence_column(&tx)?;
        create_saved_views_schema_current(&tx)?;
        create_taria_release_schema_current(&tx)?;
        create_refresh_history_schema_current(&tx)?;
        create_event_relation_collection_schema_current(&tx)?;
        add_event_uncertainty_column(&tx)?;
        create_event_identity_schema_current(&tx)?;
        create_event_annotation_schema_current(&tx)?;
        create_event_provenance_schema_current(&tx)?;
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
        current = 9;
    }

    if current == 9 {
        migrate_v9_to_v10(conn)?;
        current = 10;
    }

    if current == 10 {
        migrate_v10_to_v11(conn)?;
        current = 11;
    }

    if current == 11 {
        migrate_v11_to_v12(conn)?;
        current = 12;
    }

    if current == 12 {
        migrate_v12_to_v13(conn)?;
        current = 13;
    }

    if current == 13 {
        migrate_v13_to_v14(conn)?;
        current = 14;
    }

    if current == 14 {
        migrate_v14_to_v15(conn)?;
        current = 15;
    }

    if current == 15 {
        migrate_v15_to_v16(conn)?;
        current = 16;
    }

    if current == 16 {
        migrate_v16_to_v17(conn)?;
        current = 17;
    }

    if current == 17 {
        migrate_v17_to_v18(conn)?;
        current = 18;
    }

    if current == 18 {
        migrate_v18_to_v19(conn)?;
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

        CREATE TABLE temporal_event_upstream_identities (
            identity_kind TEXT NOT NULL,
            identity_value TEXT NOT NULL,
            event_id TEXT NOT NULL REFERENCES temporal_events(id) ON DELETE CASCADE,
            PRIMARY KEY (identity_kind, identity_value)
        );

        CREATE INDEX temporal_event_upstream_identities_event
            ON temporal_event_upstream_identities(event_id);

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

        CREATE TABLE IF NOT EXISTS taria_release_sources (
            release_id TEXT NOT NULL REFERENCES taria_releases(release_id) ON DELETE CASCADE,
            source_id TEXT NOT NULL REFERENCES temporal_sources(id) ON DELETE CASCADE,
            projection_ref TEXT NOT NULL,
            PRIMARY KEY (release_id, source_id)
        );

        CREATE INDEX IF NOT EXISTS taria_release_sources_source
            ON taria_release_sources(source_id, release_id);

        CREATE TABLE IF NOT EXISTS taria_release_event_snapshots (
            release_id TEXT NOT NULL REFERENCES taria_releases(release_id) ON DELETE CASCADE,
            event_id TEXT NOT NULL,
            normalized_title TEXT NOT NULL,
            status TEXT NOT NULL,
            time_json TEXT NOT NULL,
            snapshot_json TEXT NOT NULL,
            PRIMARY KEY (release_id, event_id)
        );

        CREATE INDEX IF NOT EXISTS taria_release_event_snapshots_event
            ON taria_release_event_snapshots(event_id, release_id);

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

    conn.execute(
        r#"
        INSERT OR IGNORE INTO temporal_event_upstream_identities (
            identity_kind, identity_value, event_id
        )
        SELECT 'event', upstream_event_ref, id
        FROM temporal_events
        WHERE upstream_event_ref IS NOT NULL
        "#,
        [],
    )
    .context("failed to backfill Taria event identity aliases")?;

    conn.execute(
        r#"
        INSERT OR IGNORE INTO temporal_event_upstream_identities (
            identity_kind, identity_value, event_id
        )
        SELECT 'reconciled', upstream_reconciled_key, id
        FROM temporal_events
        WHERE upstream_reconciled_key IS NOT NULL
        "#,
        [],
    )
    .context("failed to backfill Taria reconciled identity aliases")?;

    Ok(())
}

fn create_refresh_history_schema_current(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE source_refresh_attempts (
            id TEXT PRIMARY KEY,
            refresh_kind TEXT NOT NULL,
            target TEXT NOT NULL,
            started_at TEXT NOT NULL,
            completed_at TEXT,
            success INTEGER,
            release_id TEXT,
            summary TEXT,
            error TEXT
        );

        CREATE INDEX source_refresh_attempts_started
            ON source_refresh_attempts(started_at DESC);
        "#,
    )
    .context("failed to create source refresh history schema")
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
            table_columns_json TEXT NOT NULL DEFAULT '["date","time","title","event_type","domain","jurisdiction","institution","status","importance","personal_relevance","source"]',
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

fn add_event_recurrence_column(conn: &Connection) -> anyhow::Result<()> {
    conn.execute(
        "ALTER TABLE temporal_events ADD COLUMN recurrence_json TEXT",
        [],
    )
    .context("failed to add event recurrence definition")?;
    Ok(())
}

fn add_event_uncertainty_column(conn: &Connection) -> anyhow::Result<()> {
    conn.execute(
        "ALTER TABLE temporal_events ADD COLUMN time_uncertainty_json TEXT",
        [],
    )
    .context("failed to add event temporal uncertainty")?;
    Ok(())
}

fn create_event_relation_collection_schema_current(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE event_relations (
            id TEXT PRIMARY KEY,
            from_event_id TEXT NOT NULL REFERENCES temporal_events(id) ON DELETE CASCADE,
            to_event_id TEXT NOT NULL REFERENCES temporal_events(id) ON DELETE CASCADE,
            relation_type TEXT NOT NULL,
            properties_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            CHECK (from_event_id <> to_event_id),
            UNIQUE (from_event_id, to_event_id, relation_type)
        );

        CREATE INDEX event_relations_from
            ON event_relations(from_event_id, relation_type);
        CREATE INDEX event_relations_to
            ON event_relations(to_event_id, relation_type);
        CREATE INDEX event_relations_type
            ON event_relations(relation_type);

        CREATE TABLE event_collections (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT,
            ordered INTEGER NOT NULL DEFAULT 0,
            properties_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE INDEX event_collections_name
            ON event_collections(name COLLATE NOCASE);

        CREATE TABLE event_collection_members (
            collection_id TEXT NOT NULL REFERENCES event_collections(id) ON DELETE CASCADE,
            event_id TEXT NOT NULL REFERENCES temporal_events(id) ON DELETE CASCADE,
            position INTEGER,
            PRIMARY KEY (collection_id, event_id)
        );

        CREATE INDEX event_collection_members_event
            ON event_collection_members(event_id, collection_id);
        "#,
    )
    .context("failed to create event relation/collection schema")
}

fn create_event_identity_schema_current(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE event_identity_assessments (
            id TEXT PRIMARY KEY,
            left_event_id TEXT NOT NULL REFERENCES temporal_events(id) ON DELETE CASCADE,
            right_event_id TEXT NOT NULL REFERENCES temporal_events(id) ON DELETE CASCADE,
            state TEXT NOT NULL
                CHECK (state IN ('candidate', 'same_event', 'distinct')),
            confidence REAL,
            rationale TEXT,
            properties_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            CHECK (left_event_id < right_event_id),
            CHECK (confidence IS NULL OR (confidence >= 0.0 AND confidence <= 1.0)),
            UNIQUE (left_event_id, right_event_id)
        );

        CREATE INDEX event_identity_assessments_left
            ON event_identity_assessments(left_event_id, state);
        CREATE INDEX event_identity_assessments_right
            ON event_identity_assessments(right_event_id, state);
        CREATE INDEX event_identity_assessments_state
            ON event_identity_assessments(state);
        "#,
    )
    .context("failed to create event identity assessment schema")
}

fn create_event_provenance_schema_current(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE event_provenance_records (
            id TEXT PRIMARY KEY,
            event_id TEXT NOT NULL REFERENCES temporal_events(id) ON DELETE CASCADE,
            role TEXT NOT NULL
                CHECK (role IN ('assertion', 'source', 'provenance')),
            reference TEXT NOT NULL,
            source_id TEXT REFERENCES temporal_sources(id) ON DELETE SET NULL,
            note TEXT,
            properties_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            CHECK (length(trim(reference)) > 0),
            UNIQUE (event_id, role, reference)
        );

        CREATE INDEX event_provenance_records_event
            ON event_provenance_records(event_id, role, reference);
        CREATE INDEX event_provenance_records_source
            ON event_provenance_records(source_id, event_id)
            WHERE source_id IS NOT NULL;
        "#,
    )
    .context("failed to create event provenance schema")
}

fn create_event_annotation_schema_current(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE event_annotations (
            id TEXT PRIMARY KEY,
            event_id TEXT NOT NULL REFERENCES temporal_events(id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            value_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            CHECK (length(trim(kind)) > 0)
        );

        CREATE INDEX event_annotations_event
            ON event_annotations(event_id, kind, updated_at);
        CREATE INDEX event_annotations_kind
            ON event_annotations(kind, updated_at);
        "#,
    )
    .context("failed to create event annotation schema")
}

fn migrate_v18_to_v19(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context("failed to start v18 to v19 migration")?;
    create_event_provenance_schema_current(&tx)?;
    tx.pragma_update(None, "user_version", 19)
        .context("failed to set schema version 19")?;
    tx.commit()
        .context("failed to commit v18 to v19 schema migration")
}

fn migrate_v17_to_v18(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context("failed to start v17 to v18 migration")?;
    create_event_annotation_schema_current(&tx)?;
    tx.pragma_update(None, "user_version", 18)
        .context("failed to set schema version 18")?;
    tx.commit()
        .context("failed to commit v17 to v18 schema migration")
}

fn migrate_v16_to_v17(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context("failed to start v16 to v17 migration")?;
    create_event_identity_schema_current(&tx)?;
    tx.pragma_update(None, "user_version", 17)
        .context("failed to set schema version 17")?;
    tx.commit()
        .context("failed to commit v16 to v17 schema migration")
}

fn migrate_v15_to_v16(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context("failed to start v15 to v16 migration")?;
    add_event_uncertainty_column(&tx)?;
    tx.pragma_update(None, "user_version", 16)
        .context("failed to set schema version 16")?;
    tx.commit()
        .context("failed to commit v15 to v16 schema migration")
}

fn migrate_v14_to_v15(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context("failed to start v14 to v15 migration")?;
    create_event_relation_collection_schema_current(&tx)?;
    tx.pragma_update(None, "user_version", 15)
        .context("failed to set schema version 15")?;
    tx.commit()
        .context("failed to commit v14 to v15 schema migration")
}

fn migrate_v13_to_v14(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context("failed to start v13 to v14 migration")?;
    add_event_recurrence_column(&tx)?;
    tx.pragma_update(None, "user_version", 14)
        .context("failed to set schema version 14")?;
    tx.commit()
        .context("failed to commit v13 to v14 schema migration")
}

fn migrate_v12_to_v13(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context("failed to start v12 to v13 migration")?;
    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS source_refresh_attempts (
            id TEXT PRIMARY KEY,
            refresh_kind TEXT NOT NULL,
            target TEXT NOT NULL,
            started_at TEXT NOT NULL,
            completed_at TEXT,
            success INTEGER,
            release_id TEXT,
            summary TEXT,
            error TEXT
        );

        CREATE INDEX IF NOT EXISTS source_refresh_attempts_started
            ON source_refresh_attempts(started_at DESC);
        "#,
    )
    .context("failed to add source refresh attempt history")?;
    tx.pragma_update(None, "user_version", 13)
        .context("failed to set schema version 13")?;
    tx.commit()
        .context("failed to commit v12 to v13 schema migration")
}

fn migrate_v11_to_v12(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context("failed to start v11 to v12 migration")?;
    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS taria_release_event_snapshots (
            release_id TEXT NOT NULL REFERENCES taria_releases(release_id) ON DELETE CASCADE,
            event_id TEXT NOT NULL,
            normalized_title TEXT NOT NULL,
            status TEXT NOT NULL,
            time_json TEXT NOT NULL,
            snapshot_json TEXT NOT NULL,
            PRIMARY KEY (release_id, event_id)
        );

        CREATE INDEX IF NOT EXISTS taria_release_event_snapshots_event
            ON taria_release_event_snapshots(event_id, release_id);
        "#,
    )
    .context("failed to add Taria release event snapshots")?;
    tx.pragma_update(None, "user_version", 12)
        .context("failed to set schema version 12")?;
    tx.commit()
        .context("failed to commit v11 to v12 schema migration")
}

fn migrate_v10_to_v11(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context("failed to start v10 to v11 migration")?;
    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS taria_release_sources (
            release_id TEXT NOT NULL REFERENCES taria_releases(release_id) ON DELETE CASCADE,
            source_id TEXT NOT NULL REFERENCES temporal_sources(id) ON DELETE CASCADE,
            projection_ref TEXT NOT NULL,
            PRIMARY KEY (release_id, source_id)
        );

        CREATE INDEX IF NOT EXISTS taria_release_sources_source
            ON taria_release_sources(source_id, release_id);
        "#,
    )
    .context("failed to add Taria release/source associations")?;
    tx.pragma_update(None, "user_version", 11)
        .context("failed to set schema version 11")?;
    tx.commit()
        .context("failed to commit v10 to v11 schema migration")
}

fn migrate_v9_to_v10(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context("failed to start v9 to v10 migration")?;
    tx.execute_batch(
        r#"ALTER TABLE saved_views ADD COLUMN table_columns_json TEXT NOT NULL
            DEFAULT '["date","time","title","event_type","domain","jurisdiction","institution","status","importance","personal_relevance","source"]';"#,
    )
    .context("failed to add saved-view table columns")?;
    tx.pragma_update(None, "user_version", 10)
        .context("failed to set schema version 10")?;
    tx.commit()
        .context("failed to commit v9 to v10 schema migration")
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
            time_uncertainty_json, recurrence_json,
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
    let table_columns_raw: String = row.get("table_columns_json")?;

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
        table_columns: serde_json::from_str(&table_columns_raw).map_err(to_sql_decode_error)?,
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

fn decode_event_provenance_record(row: &Row<'_>) -> rusqlite::Result<EventProvenanceRecord> {
    let id = Uuid::parse_str(&row.get::<_, String>("id")?).map_err(to_sql_decode_error)?;
    let event_id =
        Uuid::parse_str(&row.get::<_, String>("event_id")?).map_err(to_sql_decode_error)?;
    let source_id = row
        .get::<_, Option<String>>("source_id")?
        .map(|raw| Uuid::parse_str(&raw))
        .transpose()
        .map_err(to_sql_decode_error)?;
    let role_raw: String = row.get("role")?;
    let role = EventProvenanceRole::parse(&role_raw)
        .ok_or_else(|| to_sql_decode_error(anyhow!("invalid provenance role {role_raw:?}")))?;
    Ok(EventProvenanceRecord {
        id,
        event_id,
        role,
        reference: row.get("reference")?,
        source_id,
        note: row.get("note")?,
        properties: decode_json_value(row, "properties_json")?,
        created_at: parse_datetime(&row.get::<_, String>("created_at")?)
            .map_err(to_sql_decode_error)?,
        updated_at: parse_datetime(&row.get::<_, String>("updated_at")?)
            .map_err(to_sql_decode_error)?,
    })
}

fn decode_event_annotation(row: &Row<'_>) -> rusqlite::Result<EventAnnotation> {
    let id = Uuid::parse_str(&row.get::<_, String>("id")?).map_err(to_sql_decode_error)?;
    let event_id =
        Uuid::parse_str(&row.get::<_, String>("event_id")?).map_err(to_sql_decode_error)?;
    Ok(EventAnnotation {
        id,
        event_id,
        kind: row.get("kind")?,
        value: decode_json_value(row, "value_json")?,
        created_at: parse_datetime(&row.get::<_, String>("created_at")?)
            .map_err(to_sql_decode_error)?,
        updated_at: parse_datetime(&row.get::<_, String>("updated_at")?)
            .map_err(to_sql_decode_error)?,
    })
}

fn decode_event_identity_assessment(row: &Row<'_>) -> rusqlite::Result<EventIdentityAssessment> {
    let id = Uuid::parse_str(&row.get::<_, String>("id")?).map_err(to_sql_decode_error)?;
    let left_event_id =
        Uuid::parse_str(&row.get::<_, String>("left_event_id")?).map_err(to_sql_decode_error)?;
    let right_event_id =
        Uuid::parse_str(&row.get::<_, String>("right_event_id")?).map_err(to_sql_decode_error)?;
    let state_raw: String = row.get("state")?;
    let state = EventIdentityState::parse(&state_raw).ok_or_else(|| {
        to_sql_decode_error(anyhow!(
            "invalid event identity assessment state {state_raw:?}"
        ))
    })?;
    Ok(EventIdentityAssessment {
        id,
        left_event_id,
        right_event_id,
        state,
        confidence: row.get("confidence")?,
        rationale: row.get("rationale")?,
        properties: decode_json_value(row, "properties_json")?,
        created_at: parse_datetime(&row.get::<_, String>("created_at")?)
            .map_err(to_sql_decode_error)?,
        updated_at: parse_datetime(&row.get::<_, String>("updated_at")?)
            .map_err(to_sql_decode_error)?,
    })
}

fn decode_event_relation(row: &Row<'_>) -> rusqlite::Result<EventRelation> {
    let id = Uuid::parse_str(&row.get::<_, String>("id")?).map_err(to_sql_decode_error)?;
    let from_event_id =
        Uuid::parse_str(&row.get::<_, String>("from_event_id")?).map_err(to_sql_decode_error)?;
    let to_event_id =
        Uuid::parse_str(&row.get::<_, String>("to_event_id")?).map_err(to_sql_decode_error)?;
    Ok(EventRelation {
        id,
        from_event_id,
        to_event_id,
        relation_type: row.get("relation_type")?,
        properties: decode_json_value(row, "properties_json")?,
        created_at: parse_datetime(&row.get::<_, String>("created_at")?)
            .map_err(to_sql_decode_error)?,
        updated_at: parse_datetime(&row.get::<_, String>("updated_at")?)
            .map_err(to_sql_decode_error)?,
    })
}

fn decode_event_collection(row: &Row<'_>) -> rusqlite::Result<EventCollection> {
    let id = Uuid::parse_str(&row.get::<_, String>("id")?).map_err(to_sql_decode_error)?;
    Ok(EventCollection {
        id,
        name: row.get("name")?,
        description: row.get("description")?,
        ordered: row.get("ordered")?,
        properties: decode_json_value(row, "properties_json")?,
        created_at: parse_datetime(&row.get::<_, String>("created_at")?)
            .map_err(to_sql_decode_error)?,
        updated_at: parse_datetime(&row.get::<_, String>("updated_at")?)
            .map_err(to_sql_decode_error)?,
    })
}

fn decode_event_collection_member(row: &Row<'_>) -> rusqlite::Result<EventCollectionMember> {
    let collection_id =
        Uuid::parse_str(&row.get::<_, String>("collection_id")?).map_err(to_sql_decode_error)?;
    let event_id =
        Uuid::parse_str(&row.get::<_, String>("event_id")?).map_err(to_sql_decode_error)?;
    let raw_position: Option<i64> = row.get("position")?;
    let position = raw_position
        .map(u32::try_from)
        .transpose()
        .map_err(to_sql_decode_error)?;
    Ok(EventCollectionMember {
        collection_id,
        event_id,
        position,
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
        time_uncertainty: row
            .get::<_, Option<String>>("time_uncertainty_json")?
            .map(|raw| serde_json::from_str::<TimeUncertainty>(&raw))
            .transpose()
            .map_err(to_sql_decode_error)?,
        recurrence: row
            .get::<_, Option<String>>("recurrence_json")?
            .map(|raw| serde_json::from_str::<RecurrenceRule>(&raw))
            .transpose()
            .map_err(to_sql_decode_error)?,
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
        assert!(columns.contains(&"table_columns_json".to_string()));

        let event_columns = {
            let mut stmt = conn
                .prepare("PRAGMA table_info(temporal_events)")
                .expect("event table info");
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .expect("event columns");
            rows.collect::<Result<Vec<_>, _>>().expect("event columns")
        };
        assert!(event_columns.contains(&"recurrence_json".to_string()));

        let release_source_table: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'taria_release_sources'",
                [],
                |row| row.get(0),
            )
            .expect("release source table");
        assert_eq!(release_source_table, 1);

        let snapshot_table: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'taria_release_event_snapshots'",
                [],
                |row| row.get(0),
            )
            .expect("release event snapshot table");
        assert_eq!(snapshot_table, 1);

        let refresh_history_table: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'source_refresh_attempts'",
                [],
                |row| row.get(0),
            )
            .expect("refresh history table");
        assert_eq!(refresh_history_table, 1);
    }

    #[test]
    fn release_status_and_query_choices_are_readable() {
        let store = TemporalStore::open_in_memory().expect("store");

        store
            .upsert_taria_release(&TariaReleaseRecord {
                release_id: "release:choices".to_string(),
                channel: "bootstrap".to_string(),
                status: "bootstrap-partial".to_string(),
                production_complete: false,
                manifest_path: "manifest.json".to_string(),
                manifest_sha256: "abc".to_string(),
                generated_at: Some("2026-10-04T20:00:00Z".to_string()),
                coverage_json: r#"{"ready_events":12}"#.to_string(),
                manifest_json: r#"{"canonical_bundle_slots":[]}"#.to_string(),
            })
            .expect("release");

        store
            .replace_taria_calendar_set(
                &TariaCalendarSetRecord {
                    calendar_set_id: "calendar-set:choices".to_string(),
                    release_id: "release:choices".to_string(),
                    bundle_ref: "bundle:temporal/politics-government".to_string(),
                    projection_ref: "projection:choices".to_string(),
                    input_reconciled_event_set_ref: "reconciled-set:choices".to_string(),
                    source_path: "calendar-set.json".to_string(),
                    content_sha256: "def".to_string(),
                    raw_json: "{}".to_string(),
                },
                &[TariaProjectedCalendarRecord {
                    calendar_id: "projected-calendar:choices".to_string(),
                    name: "Politics".to_string(),
                    kind: "single".to_string(),
                    metadata_json: "{}".to_string(),
                }],
                &[],
            )
            .expect("calendar set");

        let release = store
            .taria_release_status(Some("release:choices"))
            .expect("status")
            .expect("release");
        assert_eq!(release.status, "bootstrap-partial");
        assert_eq!(
            release.generated_at.as_deref(),
            Some("2026-10-04T20:00:00Z")
        );

        assert_eq!(
            store
                .taria_bundle_refs_for_release(Some("release:choices"))
                .expect("bundles"),
            vec!["bundle:temporal/politics-government".to_string()]
        );

        let calendars = store
            .taria_projected_calendar_choices_for_release(Some("release:choices"))
            .expect("calendars");
        assert_eq!(calendars.len(), 1);
        assert_eq!(calendars[0].name, "Politics");
        assert_eq!(calendars[0].calendar_id, "projected-calendar:choices");
    }

    #[test]
    fn release_membership_index_keeps_bundle_and_calendar_context_external_to_event() {
        let store = TemporalStore::open_in_memory().expect("store");
        let source = TemporalSource::new("Taria", SourceKind::Taria, SourceAuthority::Derived);
        let mut event = TemporalEvent::new(
            "Membership event",
            TimeSpec::DateOnly {
                start: NaiveDate::from_ymd_opt(2026, 10, 4).expect("date"),
                end_exclusive: None,
            },
        );
        event.source_id = Some(source.id);
        event.source_record_key = Some("record:membership".to_string());
        event.upstream_event_ref = Some("event:membership".to_string());
        event.upstream_reconciled_key = Some("reconciled-event:membership".to_string());
        store
            .import_taria_batch(&source, std::slice::from_mut(&mut event))
            .expect("event import");

        store
            .upsert_taria_release(&TariaReleaseRecord {
                release_id: "release:test".to_string(),
                channel: "bootstrap".to_string(),
                status: "bootstrap-partial".to_string(),
                production_complete: false,
                manifest_path: "manifest.json".to_string(),
                manifest_sha256: "abc".to_string(),
                generated_at: None,
                coverage_json: "{}".to_string(),
                manifest_json: "{}".to_string(),
            })
            .expect("release");

        store
            .replace_taria_calendar_set(
                &TariaCalendarSetRecord {
                    calendar_set_id: "calendar-set:test".to_string(),
                    release_id: "release:test".to_string(),
                    bundle_ref: "bundle:temporal/politics-government".to_string(),
                    projection_ref: "projection:test".to_string(),
                    input_reconciled_event_set_ref: "reconciled-set:test".to_string(),
                    source_path: "calendar-set.json".to_string(),
                    content_sha256: "def".to_string(),
                    raw_json: "{}".to_string(),
                },
                &[TariaProjectedCalendarRecord {
                    calendar_id: "projected-calendar:test".to_string(),
                    name: "Test".to_string(),
                    kind: "single".to_string(),
                    metadata_json: "{}".to_string(),
                }],
                &[TariaCalendarMembershipRecord {
                    reconciled_event_ref: "reconciled-event:membership".to_string(),
                    calendar_ref: "projected-calendar:test".to_string(),
                }],
            )
            .expect("calendar set");

        let memberships = store
            .taria_event_memberships_for_release(Some("release:test"))
            .expect("membership index");
        let membership = memberships.get(&event.id).expect("event membership");
        assert!(
            membership
                .bundle_refs
                .contains("bundle:temporal/politics-government")
        );
        assert!(membership.calendar_refs.contains("projected-calendar:test"));
        assert!(event.domain.is_none());
    }

    #[test]
    fn release_history_diff_tracks_membership_changes() {
        let store = TemporalStore::open_in_memory().expect("store");
        let source = TemporalSource::new("Taria", SourceKind::Taria, SourceAuthority::Derived);
        let day = NaiveDate::from_ymd_opt(2026, 10, 4).expect("date");

        let mut old_event = TemporalEvent::new(
            "Old member",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        old_event.source_record_key = Some("record:old".to_string());
        old_event.upstream_event_ref = Some("event:old".to_string());
        old_event.upstream_reconciled_key = Some("reconciled-event:old".to_string());

        let mut new_event = TemporalEvent::new(
            "New member",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        new_event.source_record_key = Some("record:new".to_string());
        new_event.upstream_event_ref = Some("event:new".to_string());
        new_event.upstream_reconciled_key = Some("reconciled-event:new".to_string());

        store
            .import_taria_batch(&source, &mut [old_event.clone(), new_event.clone()])
            .expect("events");

        for release_id in ["release:old", "release:new"] {
            store
                .upsert_taria_release(&TariaReleaseRecord {
                    release_id: release_id.to_string(),
                    channel: "bootstrap".to_string(),
                    status: "bootstrap-partial".to_string(),
                    production_complete: false,
                    manifest_path: format!("{release_id}.json"),
                    manifest_sha256: format!("{release_id}:hash"),
                    generated_at: None,
                    coverage_json: "{}".to_string(),
                    manifest_json: "{}".to_string(),
                })
                .expect("release");
        }

        store
            .conn
            .execute(
                "UPDATE taria_releases SET adopted_at = ?1 WHERE release_id = ?2",
                params!["2026-10-04T20:00:00Z", "release:old"],
            )
            .expect("old timestamp");
        store
            .conn
            .execute(
                "UPDATE taria_releases SET adopted_at = ?1 WHERE release_id = ?2",
                params!["2026-10-04T21:00:00Z", "release:new"],
            )
            .expect("new timestamp");

        store
            .link_taria_release_source("release:old", source.id, "projection:old")
            .expect("old release source");
        store
            .link_taria_release_source("release:new", source.id, "projection:new")
            .expect("new release source");

        store
            .replace_taria_calendar_set(
                &TariaCalendarSetRecord {
                    calendar_set_id: "calendar-set:old".to_string(),
                    release_id: "release:old".to_string(),
                    bundle_ref: "bundle:temporal/politics-government".to_string(),
                    projection_ref: "projection:old".to_string(),
                    input_reconciled_event_set_ref: "reconciled-set:old".to_string(),
                    source_path: "old-calendar.json".to_string(),
                    content_sha256: "old-calendar-hash".to_string(),
                    raw_json: "{}".to_string(),
                },
                &[TariaProjectedCalendarRecord {
                    calendar_id: "projected-calendar:old".to_string(),
                    name: "Old calendar".to_string(),
                    kind: "single".to_string(),
                    metadata_json: "{}".to_string(),
                }],
                &[TariaCalendarMembershipRecord {
                    reconciled_event_ref: "reconciled-event:old".to_string(),
                    calendar_ref: "projected-calendar:old".to_string(),
                }],
            )
            .expect("old CalendarSet");

        store
            .replace_taria_calendar_set(
                &TariaCalendarSetRecord {
                    calendar_set_id: "calendar-set:new".to_string(),
                    release_id: "release:new".to_string(),
                    bundle_ref: "bundle:temporal/finance-markets".to_string(),
                    projection_ref: "projection:new".to_string(),
                    input_reconciled_event_set_ref: "reconciled-set:new".to_string(),
                    source_path: "new-calendar.json".to_string(),
                    content_sha256: "new-calendar-hash".to_string(),
                    raw_json: "{}".to_string(),
                },
                &[TariaProjectedCalendarRecord {
                    calendar_id: "projected-calendar:new".to_string(),
                    name: "New calendar".to_string(),
                    kind: "single".to_string(),
                    metadata_json: "{}".to_string(),
                }],
                &[TariaCalendarMembershipRecord {
                    reconciled_event_ref: "reconciled-event:new".to_string(),
                    calendar_ref: "projected-calendar:new".to_string(),
                }],
            )
            .expect("new CalendarSet");

        let history = store.taria_release_history().expect("history");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].release_id, "release:new");
        assert_eq!(history[0].source_count, 1);
        assert_eq!(history[0].resolved_member_event_count, 1);

        let diff = store
            .taria_previous_release_diff("release:new")
            .expect("previous diff")
            .expect("previous release");
        assert_eq!(diff.from_release_id, "release:old");
        assert_eq!(diff.to_release_id, "release:new");
        assert_eq!(
            diff.added_source_projection_refs,
            vec!["projection:new".to_string()]
        );
        assert_eq!(
            diff.removed_source_projection_refs,
            vec!["projection:old".to_string()]
        );
        assert_eq!(
            diff.added_bundle_refs,
            vec!["bundle:temporal/finance-markets".to_string()]
        );
        assert_eq!(
            diff.removed_bundle_refs,
            vec!["bundle:temporal/politics-government".to_string()]
        );
        assert_eq!(
            diff.added_calendar_ids,
            vec!["projected-calendar:new".to_string()]
        );
        assert_eq!(
            diff.removed_calendar_ids,
            vec!["projected-calendar:old".to_string()]
        );
        assert_eq!(diff.added_member_event_ids, vec![new_event.id]);
        assert_eq!(diff.removed_member_event_ids, vec![old_event.id]);
    }

    #[test]
    fn release_event_snapshot_diff_tracks_added_moved_and_cancelled_events() {
        let store = TemporalStore::open_in_memory().expect("store");

        for release_id in ["release:snapshot-old", "release:snapshot-new"] {
            store
                .upsert_taria_release(&TariaReleaseRecord {
                    release_id: release_id.to_string(),
                    channel: "bootstrap".to_string(),
                    status: "bootstrap-partial".to_string(),
                    production_complete: false,
                    manifest_path: format!("{release_id}.json"),
                    manifest_sha256: format!("{release_id}:hash"),
                    generated_at: None,
                    coverage_json: "{}".to_string(),
                    manifest_json: "{}".to_string(),
                })
                .expect("release");
        }

        let old_day = NaiveDate::from_ymd_opt(2026, 11, 1).expect("old day");
        let new_day = NaiveDate::from_ymd_opt(2026, 11, 2).expect("new day");

        let mut changing = TemporalEvent::new(
            "Changing event",
            TimeSpec::DateOnly {
                start: old_day,
                end_exclusive: None,
            },
        );
        changing.status = EventStatus::Confirmed;
        store.upsert_event(&changing).expect("changing event");

        let removed = TemporalEvent::new(
            "Removed event",
            TimeSpec::DateOnly {
                start: old_day,
                end_exclusive: None,
            },
        );
        store.upsert_event(&removed).expect("removed event");

        store
            .capture_taria_release_event_snapshots(
                "release:snapshot-old",
                &BTreeSet::from([changing.id, removed.id]),
            )
            .expect("old snapshots");

        changing.normalized_title = "Changed event".to_string();
        changing.time = TimeSpec::DateOnly {
            start: new_day,
            end_exclusive: None,
        };
        changing.status = EventStatus::Cancelled;
        changing.updated_at = Utc::now();
        store.upsert_event(&changing).expect("changed event");

        let added = TemporalEvent::new(
            "Added event",
            TimeSpec::DateOnly {
                start: new_day,
                end_exclusive: None,
            },
        );
        store.upsert_event(&added).expect("added event");

        store
            .capture_taria_release_event_snapshots(
                "release:snapshot-new",
                &BTreeSet::from([changing.id, added.id]),
            )
            .expect("new snapshots");

        assert_eq!(
            store
                .capture_taria_release_event_snapshots(
                    "release:snapshot-old",
                    &BTreeSet::from([changing.id]),
                )
                .expect("existing release snapshots remain immutable"),
            0
        );

        assert_eq!(
            store
                .taria_release_event_snapshot_count("release:snapshot-old")
                .expect("old snapshot count"),
            2
        );
        assert_eq!(
            store
                .taria_release_event_snapshot_count("release:snapshot-new")
                .expect("new snapshot count"),
            2
        );

        let diff = store
            .taria_release_diff("release:snapshot-old", "release:snapshot-new")
            .expect("snapshot diff");
        assert_eq!(diff.added_snapshot_event_ids, vec![added.id]);
        assert_eq!(diff.removed_snapshot_event_ids, vec![removed.id]);
        assert_eq!(diff.moved_event_ids, vec![changing.id]);
        assert_eq!(diff.status_changed_event_ids, vec![changing.id]);
        assert_eq!(diff.newly_cancelled_event_ids, vec![changing.id]);
        assert_eq!(diff.event_changes.len(), 3);

        let added_change = diff
            .event_changes
            .iter()
            .find(|change| change.event_id == added.id)
            .expect("added detail");
        assert!(added_change.added);
        assert!(!added_change.title_changed);
        assert_eq!(added_change.title, "Added event");
        assert_eq!(added_change.to_title.as_deref(), Some("Added event"));

        let removed_change = diff
            .event_changes
            .iter()
            .find(|change| change.event_id == removed.id)
            .expect("removed detail");
        assert!(removed_change.removed);
        assert!(!removed_change.title_changed);
        assert_eq!(removed_change.title, "Removed event");
        assert_eq!(removed_change.from_title.as_deref(), Some("Removed event"));

        let changed = diff
            .event_changes
            .iter()
            .find(|change| change.event_id == changing.id)
            .expect("changed detail");
        assert!(changed.title_changed);
        assert!(changed.moved);
        assert!(changed.status_changed);
        assert!(changed.newly_cancelled);
        assert_eq!(changed.from_title.as_deref(), Some("Changing event"));
        assert_eq!(changed.to_title.as_deref(), Some("Changed event"));
        assert_eq!(changed.from_status.as_deref(), Some("confirmed"));
        assert_eq!(changed.to_status.as_deref(), Some("cancelled"));
        assert_ne!(changed.from_time_json, changed.to_time_json);
    }

    #[test]
    fn schema_bootstraps_at_current_version() {
        let store = TemporalStore::open_in_memory().expect("store");
        assert_eq!(store.schema_version().expect("version"), SCHEMA_VERSION);
    }

    #[test]
    fn temporal_uncertainty_roundtrips_and_invalid_windows_do_not_persist() {
        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 7).expect("day");
        let mut event = TemporalEvent::new(
            "Uncertain placement",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.time_uncertainty = Some(TimeUncertainty::DateWindow {
            earliest: day - chrono::Duration::days(2),
            latest: day + chrono::Duration::days(3),
        });

        store.upsert_event(&event).expect("persist uncertainty");
        let loaded = store.event_by_id(event.id).expect("query").expect("event");
        assert_eq!(loaded.time_uncertainty, event.time_uncertainty);

        let mut invalid = TemporalEvent::new(
            "Invalid uncertainty",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        invalid.time_uncertainty = Some(TimeUncertainty::DateWindow {
            earliest: day + chrono::Duration::days(1),
            latest: day + chrono::Duration::days(2),
        });
        let error = store
            .upsert_event(&invalid)
            .expect_err("invalid uncertainty must not persist");
        assert!(
            error
                .to_string()
                .contains("invalid event temporal uncertainty")
        );
        assert!(
            store
                .event_by_id(invalid.id)
                .expect("query invalid event")
                .is_none()
        );
    }

    #[test]
    fn v15_migration_adds_nullable_temporal_uncertainty_without_rewriting_events() {
        let mut conn = Connection::open_in_memory().expect("connection");
        configure_connection(&conn).expect("configure");

        let tx = conn.transaction().expect("schema transaction");
        create_schema_v2(&tx).expect("base schema");
        add_event_recurrence_column(&tx).expect("recurrence column");
        create_saved_views_schema_current(&tx).expect("saved views");
        create_taria_release_schema_current(&tx).expect("taria schema");
        create_refresh_history_schema_current(&tx).expect("refresh schema");
        create_event_relation_collection_schema_current(&tx).expect("topology schema");
        tx.pragma_update(None, "user_version", 15).expect("set v15");
        tx.commit().expect("commit v15 schema");

        let event_id = Uuid::new_v4();
        conn.execute(
            r#"
            INSERT INTO temporal_events (
                id,
                assertion_refs_json, source_refs_json, provenance_refs_json,
                normalized_title, status,
                time_kind, start_date,
                tags_json, properties_json,
                created_at, updated_at
            ) VALUES (?1, '[]', '[]', '[]', ?2, 'scheduled', 'date_only', ?3, '[]', '{}', ?4, ?4)
            "#,
            params![
                event_id.to_string(),
                "Pre-v16 event",
                "2026-10-07",
                "2026-10-07T00:00:00Z",
            ],
        )
        .expect("insert v15 event");

        migrate(&mut conn).expect("migrate to current");
        let store = TemporalStore { conn, path: None };
        assert_eq!(store.schema_version().expect("version"), SCHEMA_VERSION);

        let loaded = store.event_by_id(event_id).expect("query").expect("event");
        assert_eq!(loaded.normalized_title, "Pre-v16 event");
        assert!(loaded.time_uncertainty.is_none());
    }

    #[test]
    fn event_title_search_is_bounded_case_insensitive_and_excludable() {
        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 7).expect("day");
        let alpha = TemporalEvent::new(
            "Alpha Launch",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let mut beta = TemporalEvent::new(
            "Beta Review",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        beta.upstream_event_ref = Some("alpha-reference".to_string());
        store.upsert_event(&alpha).expect("alpha");
        store.upsert_event(&beta).expect("beta");

        let matches = store
            .search_event_titles("ALPHA", None, 10)
            .expect("search");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0], (alpha.id, "Alpha Launch".to_string()));

        let excluded = store
            .search_event_titles("alpha", Some(alpha.id), 1)
            .expect("excluded search");
        assert_eq!(excluded, vec![(beta.id, "Beta Review".to_string())]);

        assert!(
            store
                .search_event_titles("", None, 10)
                .expect("empty search")
                .is_empty()
        );
    }

    #[test]
    fn event_provenance_records_roundtrip_enforce_semantic_uniqueness_and_fk_behavior() {
        let store = TemporalStore::open_in_memory().expect("store");
        let source = TemporalSource::new("Archive", SourceKind::Json, SourceAuthority::Official);
        store.upsert_source(&source).expect("source");
        let day = NaiveDate::from_ymd_opt(2026, 10, 7).expect("day");
        let event = TemporalEvent::new(
            "Provenance target",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        store.upsert_event(&event).expect("event");

        let mut record = EventProvenanceRecord::new(
            event.id,
            EventProvenanceRole::Source,
            "https://example.org/archive/item-42",
        );
        record.source_id = Some(source.id);
        record.note = Some("Primary archival source".to_string());
        record.properties = serde_json::json!({"page": 17});
        store
            .upsert_event_provenance_record(&record)
            .expect("provenance");

        assert_eq!(
            store
                .event_provenance_record_by_id(record.id)
                .expect("query")
                .expect("record"),
            record
        );
        assert_eq!(
            store
                .event_provenance_records_for_event(event.id)
                .expect("event provenance"),
            vec![record.clone()]
        );
        assert_eq!(
            store
                .event_provenance_records_for_source(source.id)
                .expect("source provenance"),
            vec![record.clone()]
        );
        assert_eq!(
            store
                .list_event_provenance_records()
                .expect("all provenance"),
            vec![record.clone()]
        );

        let duplicate = EventProvenanceRecord::new(
            event.id,
            EventProvenanceRole::Source,
            "https://example.org/archive/item-42",
        );
        assert!(
            store
                .upsert_event_provenance_record(&duplicate)
                .expect_err("duplicate semantic provenance must fail")
                .to_string()
                .contains("failed to upsert event provenance record")
        );

        let invalid = EventProvenanceRecord::new(event.id, EventProvenanceRole::Assertion, "   ");
        assert!(
            store
                .upsert_event_provenance_record(&invalid)
                .expect_err("empty provenance reference must fail")
                .to_string()
                .contains("invalid event provenance record")
        );

        store
            .conn
            .execute(
                "DELETE FROM temporal_sources WHERE id = ?1",
                params![source.id.to_string()],
            )
            .expect("delete source");
        let after_source_delete = store
            .event_provenance_record_by_id(record.id)
            .expect("query after source delete")
            .expect("record after source delete");
        assert!(after_source_delete.source_id.is_none());

        store
            .conn
            .execute(
                "DELETE FROM temporal_events WHERE id = ?1",
                params![event.id.to_string()],
            )
            .expect("delete event");
        assert!(
            store
                .event_provenance_record_by_id(record.id)
                .expect("query after event cascade")
                .is_none()
        );
    }

    #[test]
    fn v18_migration_adds_structured_provenance_without_rewriting_events() {
        let mut conn = Connection::open_in_memory().expect("connection");
        configure_connection(&conn).expect("configure");

        let tx = conn.transaction().expect("schema transaction");
        create_schema_v2(&tx).expect("base schema");
        add_event_recurrence_column(&tx).expect("recurrence column");
        create_saved_views_schema_current(&tx).expect("saved views");
        create_taria_release_schema_current(&tx).expect("taria schema");
        create_refresh_history_schema_current(&tx).expect("refresh schema");
        create_event_relation_collection_schema_current(&tx).expect("topology schema");
        add_event_uncertainty_column(&tx).expect("uncertainty column");
        create_event_identity_schema_current(&tx).expect("identity schema");
        create_event_annotation_schema_current(&tx).expect("annotation schema");
        tx.pragma_update(None, "user_version", 18).expect("set v18");
        tx.commit().expect("commit v18 schema");

        let event_id = Uuid::new_v4();
        conn.execute(
            r#"
            INSERT INTO temporal_events (
                id,
                assertion_refs_json, source_refs_json, provenance_refs_json,
                normalized_title, status,
                time_kind, start_date,
                tags_json, properties_json,
                created_at, updated_at
            ) VALUES (?1, '[]', '[]', '[]', ?2, 'scheduled', 'date_only', ?3, '[]', '{}', ?4, ?4)
            "#,
            params![
                event_id.to_string(),
                "Pre-v19 event",
                "2026-10-07",
                "2026-10-07T00:00:00Z",
            ],
        )
        .expect("insert v18 event");

        migrate(&mut conn).expect("migrate to current");
        let store = TemporalStore { conn, path: None };
        assert_eq!(store.schema_version().expect("version"), SCHEMA_VERSION);

        let loaded = store.event_by_id(event_id).expect("query").expect("event");
        assert_eq!(loaded.normalized_title, "Pre-v19 event");
        assert!(
            store
                .event_provenance_records_for_event(event_id)
                .expect("provenance")
                .is_empty()
        );
    }

    #[test]
    fn event_annotations_roundtrip_survive_source_refresh_and_cascade_on_event_delete() {
        let store = TemporalStore::open_in_memory().expect("store");
        let source = TemporalSource::new("Feed", SourceKind::Ics, SourceAuthority::Official);
        let day = NaiveDate::from_ymd_opt(2026, 10, 7).expect("day");

        let mut event = TemporalEvent::new(
            "Original title",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.source_record_key = Some("uid-annotation".to_string());
        let first = store
            .import_batch(&source, std::slice::from_mut(&mut event))
            .expect("initial import");
        assert_eq!(first.created, 1);

        let canonical = store
            .event_by_source_record(source.id, "uid-annotation")
            .expect("event query")
            .expect("event");
        let mut annotation = EventAnnotation::new(
            canonical.id,
            "note",
            serde_json::json!({"text": "Keep this local note"}),
        );
        annotation.updated_at = Utc::now();
        store
            .upsert_event_annotation(&annotation)
            .expect("annotation");

        assert_eq!(
            store
                .event_annotation_by_id(annotation.id)
                .expect("annotation query")
                .expect("annotation"),
            annotation
        );
        assert_eq!(
            store
                .event_annotations_for_event(canonical.id)
                .expect("event annotations"),
            vec![annotation.clone()]
        );

        let mut refreshed = TemporalEvent::new(
            "Updated title from source",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        refreshed.source_record_key = Some("uid-annotation".to_string());
        let result = store
            .import_batch(&source, std::slice::from_mut(&mut refreshed))
            .expect("refresh import");
        assert_eq!(result.updated, 1);

        let refreshed_event = store
            .event_by_source_record(source.id, "uid-annotation")
            .expect("event query")
            .expect("event");
        assert_eq!(refreshed_event.id, canonical.id);
        assert_eq!(
            refreshed_event.normalized_title,
            "Updated title from source"
        );
        assert_eq!(
            store
                .event_annotations_for_event(canonical.id)
                .expect("annotations after refresh"),
            vec![annotation.clone()]
        );

        assert!(
            store
                .delete_event_annotation(annotation.id)
                .expect("delete annotation")
        );
        assert!(
            store
                .event_annotations_for_event(canonical.id)
                .expect("annotations after delete")
                .is_empty()
        );

        let cascade = EventAnnotation::new(canonical.id, "watched", serde_json::json!(true));
        store
            .upsert_event_annotation(&cascade)
            .expect("cascade annotation");
        store
            .conn
            .execute(
                "DELETE FROM temporal_events WHERE id = ?1",
                params![canonical.id.to_string()],
            )
            .expect("delete event");
        assert!(
            store
                .event_annotation_by_id(cascade.id)
                .expect("annotation after cascade")
                .is_none()
        );
    }

    #[test]
    fn v17_migration_adds_annotations_without_rewriting_events() {
        let mut conn = Connection::open_in_memory().expect("connection");
        configure_connection(&conn).expect("configure");

        let tx = conn.transaction().expect("schema transaction");
        create_schema_v2(&tx).expect("base schema");
        add_event_recurrence_column(&tx).expect("recurrence column");
        create_saved_views_schema_current(&tx).expect("saved views");
        create_taria_release_schema_current(&tx).expect("taria schema");
        create_refresh_history_schema_current(&tx).expect("refresh schema");
        create_event_relation_collection_schema_current(&tx).expect("topology schema");
        add_event_uncertainty_column(&tx).expect("uncertainty column");
        create_event_identity_schema_current(&tx).expect("identity schema");
        tx.pragma_update(None, "user_version", 17).expect("set v17");
        tx.commit().expect("commit v17 schema");

        let event_id = Uuid::new_v4();
        conn.execute(
            r#"
            INSERT INTO temporal_events (
                id,
                assertion_refs_json, source_refs_json, provenance_refs_json,
                normalized_title, status,
                time_kind, start_date,
                tags_json, properties_json,
                created_at, updated_at
            ) VALUES (?1, '[]', '[]', '[]', ?2, 'scheduled', 'date_only', ?3, '[]', '{}', ?4, ?4)
            "#,
            params![
                event_id.to_string(),
                "Pre-v18 event",
                "2026-10-07",
                "2026-10-07T00:00:00Z",
            ],
        )
        .expect("insert v17 event");

        migrate(&mut conn).expect("migrate to current");
        let store = TemporalStore { conn, path: None };
        assert_eq!(store.schema_version().expect("version"), SCHEMA_VERSION);

        let loaded = store.event_by_id(event_id).expect("query").expect("event");
        assert_eq!(loaded.normalized_title, "Pre-v18 event");
        assert!(
            store
                .event_annotations_for_event(event_id)
                .expect("annotations")
                .is_empty()
        );
    }

    #[test]
    fn event_identity_assessments_roundtrip_enforce_pair_uniqueness_and_cascade() {
        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 7).expect("day");
        let first = TemporalEvent::new(
            "Possible duplicate A",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let second = TemporalEvent::new(
            "Possible duplicate B",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        store.upsert_event(&first).expect("first");
        store.upsert_event(&second).expect("second");

        let mut assessment = EventIdentityAssessment::new(second.id, first.id);
        assessment.state = EventIdentityState::Candidate;
        assessment.confidence = Some(0.84);
        assessment.rationale = Some("same title/date from independent feeds".to_string());
        assessment.properties = serde_json::json!({"method": "manual_review"});
        store
            .upsert_event_identity_assessment(&assessment)
            .expect("assessment");

        assert_eq!(
            store
                .event_identity_assessment_by_id(assessment.id)
                .expect("query by id")
                .expect("assessment"),
            assessment
        );
        assert_eq!(
            store
                .event_identity_assessment_between(first.id, second.id)
                .expect("query pair")
                .expect("pair"),
            assessment
        );
        assert_eq!(
            store
                .event_identity_assessment_between(second.id, first.id)
                .expect("query reversed pair")
                .expect("pair"),
            assessment
        );
        assert_eq!(
            store
                .event_identity_assessments_for_event(first.id)
                .expect("first assessments"),
            vec![assessment.clone()]
        );
        assert_eq!(
            store
                .list_event_identity_assessments()
                .expect("all assessments"),
            vec![assessment.clone()]
        );

        let duplicate = EventIdentityAssessment::new(first.id, second.id);
        assert!(
            store
                .upsert_event_identity_assessment(&duplicate)
                .expect_err("duplicate pair must fail")
                .to_string()
                .contains("failed to upsert event identity assessment")
        );

        let mut resolved = assessment.clone();
        resolved.state = EventIdentityState::SameEvent;
        resolved.confidence = Some(1.0);
        resolved.updated_at = Utc::now();
        store
            .upsert_event_identity_assessment(&resolved)
            .expect("resolve same event");
        assert_eq!(
            store
                .event_identity_assessment_between(first.id, second.id)
                .expect("resolved pair")
                .expect("assessment")
                .state,
            EventIdentityState::SameEvent
        );

        store
            .conn
            .execute(
                "DELETE FROM temporal_events WHERE id = ?1",
                params![first.id.to_string()],
            )
            .expect("delete event");
        assert!(
            store
                .event_identity_assessment_by_id(assessment.id)
                .expect("query after cascade")
                .is_none()
        );
    }

    #[test]
    fn v16_migration_adds_identity_assessments_without_rewriting_events() {
        let mut conn = Connection::open_in_memory().expect("connection");
        configure_connection(&conn).expect("configure");

        let tx = conn.transaction().expect("schema transaction");
        create_schema_v2(&tx).expect("base schema");
        add_event_recurrence_column(&tx).expect("recurrence column");
        create_saved_views_schema_current(&tx).expect("saved views");
        create_taria_release_schema_current(&tx).expect("taria schema");
        create_refresh_history_schema_current(&tx).expect("refresh schema");
        create_event_relation_collection_schema_current(&tx).expect("topology schema");
        add_event_uncertainty_column(&tx).expect("uncertainty column");
        tx.pragma_update(None, "user_version", 16).expect("set v16");
        tx.commit().expect("commit v16 schema");

        let event_id = Uuid::new_v4();
        conn.execute(
            r#"
            INSERT INTO temporal_events (
                id,
                assertion_refs_json, source_refs_json, provenance_refs_json,
                normalized_title, status,
                time_kind, start_date,
                tags_json, properties_json,
                created_at, updated_at
            ) VALUES (?1, '[]', '[]', '[]', ?2, 'scheduled', 'date_only', ?3, '[]', '{}', ?4, ?4)
            "#,
            params![
                event_id.to_string(),
                "Pre-v17 event",
                "2026-10-07",
                "2026-10-07T00:00:00Z",
            ],
        )
        .expect("insert v16 event");

        migrate(&mut conn).expect("migrate to current");
        let store = TemporalStore { conn, path: None };
        assert_eq!(store.schema_version().expect("version"), SCHEMA_VERSION);
        assert_eq!(
            store
                .event_by_id(event_id)
                .expect("query")
                .expect("event")
                .normalized_title,
            "Pre-v17 event"
        );
        assert!(
            store
                .list_event_identity_assessments()
                .expect("identity assessments")
                .is_empty()
        );
    }

    #[test]
    fn event_relations_roundtrip_query_and_enforce_identity_rules() {
        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 7).expect("day");
        let first = TemporalEvent::new(
            "Cause",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let second = TemporalEvent::new(
            "Effect",
            TimeSpec::DateOnly {
                start: day.succ_opt().expect("next day"),
                end_exclusive: None,
            },
        );
        store.upsert_event(&first).expect("first event");
        store.upsert_event(&second).expect("second event");

        let mut relation = EventRelation::new(first.id, second.id, "causes");
        relation.properties = serde_json::json!({"confidence": 0.8});
        store
            .upsert_event_relation(&relation)
            .expect("persist relation");

        assert_eq!(
            store
                .event_relation_by_id(relation.id)
                .expect("relation query")
                .expect("relation"),
            relation
        );
        assert_eq!(
            store.list_event_relations().expect("relations"),
            vec![relation.clone()]
        );
        assert_eq!(
            store
                .event_relations_for_event(first.id)
                .expect("first relations"),
            vec![relation.clone()]
        );
        assert_eq!(
            store
                .event_relations_for_event(second.id)
                .expect("second relations"),
            vec![relation.clone()]
        );

        let duplicate = EventRelation::new(first.id, second.id, "causes");
        assert!(
            store
                .upsert_event_relation(&duplicate)
                .expect_err("duplicate semantic relation must fail")
                .to_string()
                .contains("failed to upsert event relation")
        );

        let self_relation = EventRelation::new(first.id, first.id, "references");
        assert!(
            store
                .upsert_event_relation(&self_relation)
                .expect_err("self relation must fail")
                .to_string()
                .contains("invalid event relation")
        );

        store
            .conn
            .execute(
                "DELETE FROM temporal_events WHERE id = ?1",
                params![first.id.to_string()],
            )
            .expect("delete event");
        assert!(
            store
                .event_relation_by_id(relation.id)
                .expect("relation query after cascade")
                .is_none()
        );
    }

    #[test]
    fn ordered_event_collection_preserves_sequence_and_reverse_lookup() {
        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 7).expect("day");
        let alpha = TemporalEvent::new(
            "Alpha",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let beta = TemporalEvent::new(
            "Beta",
            TimeSpec::DateOnly {
                start: day.succ_opt().expect("next day"),
                end_exclusive: None,
            },
        );
        let gamma = TemporalEvent::new(
            "Gamma",
            TimeSpec::DateOnly {
                start: day
                    .succ_opt()
                    .and_then(|date| date.succ_opt())
                    .expect("third day"),
                end_exclusive: None,
            },
        );
        for event in [&alpha, &beta, &gamma] {
            store.upsert_event(event).expect("event");
        }

        let mut collection = EventCollection::new("Sequence", true);
        collection.description = Some("Intentional narrative order".to_string());
        collection.properties = serde_json::json!({"kind": "sequence"});
        store
            .upsert_event_collection(&collection)
            .expect("collection");
        store
            .replace_event_collection_members(collection.id, &[gamma.id, alpha.id, beta.id])
            .expect("members");

        assert_eq!(
            store
                .event_collection_by_id(collection.id)
                .expect("collection query")
                .expect("collection"),
            collection
        );
        let members = store
            .event_collection_members(collection.id)
            .expect("members");
        assert_eq!(
            members,
            vec![
                EventCollectionMember {
                    collection_id: collection.id,
                    event_id: gamma.id,
                    position: Some(0),
                },
                EventCollectionMember {
                    collection_id: collection.id,
                    event_id: alpha.id,
                    position: Some(1),
                },
                EventCollectionMember {
                    collection_id: collection.id,
                    event_id: beta.id,
                    position: Some(2),
                },
            ]
        );

        assert_eq!(
            store
                .list_event_collection_members()
                .expect("all collection members"),
            members
        );

        let events = store
            .events_for_collection(collection.id)
            .expect("collection events");
        assert_eq!(
            events
                .iter()
                .map(|event| event.normalized_title.as_str())
                .collect::<Vec<_>>(),
            vec!["Gamma", "Alpha", "Beta"]
        );
        assert_eq!(
            store
                .collections_for_event(alpha.id)
                .expect("reverse lookup"),
            vec![collection.clone()]
        );

        let before = members;
        let missing = Uuid::new_v4();
        assert!(
            store
                .replace_event_collection_members(collection.id, &[alpha.id, missing])
                .is_err()
        );
        assert_eq!(
            store
                .event_collection_members(collection.id)
                .expect("members after rollback"),
            before
        );

        assert!(
            store
                .replace_event_collection_members(collection.id, &[alpha.id, alpha.id])
                .is_err()
        );
        assert_eq!(
            store
                .event_collection_members(collection.id)
                .expect("members after duplicate rejection"),
            before
        );

        assert!(
            store
                .delete_event_collection(collection.id)
                .expect("delete collection")
        );
        assert!(
            store
                .event_collection_by_id(collection.id)
                .expect("collection query")
                .is_none()
        );
        assert!(
            store
                .event_collection_members(collection.id)
                .expect("members after collection delete")
                .is_empty()
        );
    }

    #[test]
    fn relation_collection_membership_index_preserves_direction_and_collection_ids() {
        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 7).expect("day");
        let first = TemporalEvent::new(
            "First",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let second = TemporalEvent::new(
            "Second",
            TimeSpec::DateOnly {
                start: day.succ_opt().expect("next day"),
                end_exclusive: None,
            },
        );
        store.upsert_event(&first).expect("first");
        store.upsert_event(&second).expect("second");

        let relation = EventRelation::new(first.id, second.id, "precedes");
        store.upsert_event_relation(&relation).expect("relation");

        let collection = EventCollection::new("Pair", false);
        store
            .upsert_event_collection(&collection)
            .expect("collection");
        store
            .replace_event_collection_members(collection.id, &[first.id, second.id])
            .expect("members");

        let index = store
            .event_relation_collection_memberships()
            .expect("membership index");
        let first_membership = index.get(&first.id).expect("first membership");
        assert!(first_membership.collection_ids.contains(&collection.id));
        assert!(
            first_membership
                .outgoing_relation_types
                .contains("precedes")
        );
        assert!(first_membership.incoming_relation_types.is_empty());

        let second_membership = index.get(&second.id).expect("second membership");
        assert!(second_membership.collection_ids.contains(&collection.id));
        assert!(
            second_membership
                .incoming_relation_types
                .contains("precedes")
        );
        assert!(second_membership.outgoing_relation_types.is_empty());
    }

    #[test]
    fn unordered_event_collection_omits_positions_and_uses_stable_event_order() {
        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 7).expect("day");
        let beta = TemporalEvent::new(
            "Beta",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let alpha = TemporalEvent::new(
            "Alpha",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        store.upsert_event(&beta).expect("beta");
        store.upsert_event(&alpha).expect("alpha");

        let collection = EventCollection::new("Set", false);
        store
            .upsert_event_collection(&collection)
            .expect("collection");
        store
            .replace_event_collection_members(collection.id, &[beta.id, alpha.id])
            .expect("members");

        let members = store
            .event_collection_members(collection.id)
            .expect("members");
        assert!(members.iter().all(|member| member.position.is_none()));

        let events = store
            .events_for_collection(collection.id)
            .expect("collection events");
        assert_eq!(
            events
                .iter()
                .map(|event| event.normalized_title.as_str())
                .collect::<Vec<_>>(),
            vec!["Alpha", "Beta"]
        );
    }

    #[test]
    fn store_rejects_invalid_recurrence_definition() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Broken recurrence",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Daily,
            interval: 0,
            count: None,
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let error = store
            .upsert_event(&event)
            .expect_err("invalid recurrence must not persist");
        assert!(
            error
                .to_string()
                .contains("invalid event recurrence definition")
        );
        assert_eq!(store.event_count().expect("event count"), 0);
    }

    #[test]
    fn store_rejects_recurrence_on_unsupported_time_precision() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let mut event = TemporalEvent::new("Too coarse", TimeSpec::Year { year: 2026 });
        event.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Yearly));

        let error = store
            .upsert_event(&event)
            .expect_err("unsupported recurrence must not persist");
        assert!(
            error
                .to_string()
                .contains("invalid event recurrence definition")
        );
        assert_eq!(store.event_count().expect("event count"), 0);
    }

    #[test]
    fn weekly_bymonth_roundtrips_through_event_storage() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule, RecurrenceWeekday};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 1, 26).expect("day");
        let mut event = TemporalEvent::new(
            "Selected-month weekly",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Weekly,
            interval: 2,
            count: Some(5),
            until: None,
            week_start: RecurrenceWeekday::Sunday,
            by_weekday: vec![RecurrenceWeekday::Monday, RecurrenceWeekday::Wednesday],
            by_month: vec![1, 3],
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: vec![-1],
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        store.upsert_event(&event).expect("persist recurrence");
        let loaded = store.event_by_id(event.id).expect("query").expect("event");

        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn yearly_by_month_roundtrips_through_event_storage() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 4, 15).expect("day");
        let mut event = TemporalEvent::new(
            "Yearly selectors",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(8),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: vec![1, 4, 10],
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        store.upsert_event(&event).expect("save");
        let loaded = store
            .event_by_id(event.id)
            .expect("load")
            .expect("stored event");
        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn store_rejects_monthly_ordinal_byday_on_non_monthly_recurrence() {
        use crate::domain::{
            RecurrenceFrequency, RecurrenceOrdinalWeekday, RecurrenceRule, RecurrenceWeekday,
        };

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Invalid ordinal BYDAY",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Weekly,
            interval: 1,
            count: None,
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: vec![RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday)],
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let error = store
            .upsert_event(&event)
            .expect_err("non-monthly ordinal BYDAY must not persist");
        assert!(
            error
                .to_string()
                .contains("invalid event recurrence definition")
        );
        assert_eq!(store.event_count().expect("event count"), 0);
    }

    #[test]
    fn monthly_ordinal_byday_roundtrips_through_event_storage() {
        use crate::domain::{
            RecurrenceFrequency, RecurrenceOrdinalWeekday, RecurrenceRule, RecurrenceWeekday,
        };

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Ordinal weekday selectors",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: Some(8),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: vec![
                RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday),
                RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday),
            ],
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        store.upsert_event(&event).expect("save");
        let loaded = store
            .event_by_id(event.id)
            .expect("load")
            .expect("stored event");
        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn combined_monthly_selectors_roundtrip_through_event_storage() {
        use crate::domain::{
            RecurrenceFrequency, RecurrenceOrdinalWeekday, RecurrenceRule, RecurrenceWeekday,
        };

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Combined monthly selectors",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: Some(4),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![1, 15, -1],
            by_month_weekday: vec![
                RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday),
                RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday),
            ],
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        store.upsert_event(&event).expect("save");
        let loaded = store
            .event_by_id(event.id)
            .expect("load")
            .expect("stored event");
        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn monthly_plain_byday_roundtrips_through_event_storage() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule, RecurrenceWeekday};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 1, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Monthly weekdays",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: Some(6),
            until: None,
            week_start: Default::default(),
            by_weekday: vec![
                RecurrenceWeekday::Monday,
                RecurrenceWeekday::Wednesday,
                RecurrenceWeekday::Friday,
            ],
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: vec![-1],
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        store.upsert_event(&event).expect("save");
        let loaded = store
            .event_by_id(event.id)
            .expect("load")
            .expect("stored event");
        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn store_rejects_by_week_no_on_non_yearly_recurrence() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Invalid BYWEEKNO",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: None,
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: vec![1],
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let error = store
            .upsert_event(&event)
            .expect_err("non-yearly BYWEEKNO must not persist");
        assert!(
            error
                .to_string()
                .contains("invalid event recurrence definition")
        );
        assert_eq!(store.event_count().expect("event count"), 0);
    }

    #[test]
    fn yearly_by_week_no_roundtrips_through_event_storage() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule, RecurrenceWeekday};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Week-number selectors",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(4),
            until: None,
            week_start: RecurrenceWeekday::Sunday,
            by_weekday: vec![RecurrenceWeekday::Sunday, RecurrenceWeekday::Wednesday],
            by_month: Vec::new(),
            by_week_no: vec![1, -1],
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: vec![-1],
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        store.upsert_event(&event).expect("save");
        let loaded = store
            .event_by_id(event.id)
            .expect("load")
            .expect("stored event");
        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn store_rejects_by_set_pos_without_other_selector() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Invalid BYSETPOS",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: None,
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: vec![-1],
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let error = store
            .upsert_event(&event)
            .expect_err("selector-free BYSETPOS must not persist");
        assert!(
            error
                .to_string()
                .contains("invalid event recurrence definition")
        );
        assert_eq!(store.event_count().expect("event count"), 0);
    }

    #[test]
    fn by_set_pos_roundtrips_through_event_storage() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Set-position selectors",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: Some(6),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![1, 15, -1],
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: vec![1, -1],
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        store.upsert_event(&event).expect("save");
        let loaded = store
            .event_by_id(event.id)
            .expect("load")
            .expect("stored event");
        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn store_rejects_by_year_day_on_non_yearly_recurrence() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Invalid BYYEARDAY",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: None,
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: vec![1],
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let error = store
            .upsert_event(&event)
            .expect_err("non-yearly BYYEARDAY must not persist");
        assert!(
            error
                .to_string()
                .contains("invalid event recurrence definition")
        );
        assert_eq!(store.event_count().expect("event count"), 0);
    }

    #[test]
    fn yearly_by_year_day_roundtrips_through_event_storage() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Year-day selectors",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(8),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: vec![1, 7, 12],
            by_week_no: Vec::new(),
            by_year_day: vec![1, 100, 212, -1],
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        store.upsert_event(&event).expect("save");
        let loaded = store
            .event_by_id(event.id)
            .expect("load")
            .expect("stored event");
        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn store_rejects_out_of_range_whole_year_ordinal_byday() {
        use crate::domain::{
            RecurrenceFrequency, RecurrenceOrdinalWeekday, RecurrenceRule, RecurrenceWeekday,
        };

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Invalid yearly ordinal BYDAY",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: None,
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: vec![RecurrenceOrdinalWeekday::new(54, RecurrenceWeekday::Monday)],
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let error = store
            .upsert_event(&event)
            .expect_err("whole-year ordinal BYDAY beyond 53 must not persist");
        assert!(
            error
                .to_string()
                .contains("invalid event recurrence definition")
        );
        assert_eq!(store.event_count().expect("event count"), 0);
    }

    #[test]
    fn yearly_by_month_and_ordinal_byday_roundtrips_through_event_storage() {
        use crate::domain::{
            RecurrenceFrequency, RecurrenceOrdinalWeekday, RecurrenceRule, RecurrenceWeekday,
        };

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 4, 15).expect("day");
        let mut event = TemporalEvent::new(
            "Yearly ordinal selectors",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(8),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: vec![3, 11],
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![1, 30, -1],
            by_month_weekday: vec![
                RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday),
                RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday),
            ],
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        store.upsert_event(&event).expect("save");
        let loaded = store
            .event_by_id(event.id)
            .expect("load")
            .expect("stored event");
        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn store_rejects_out_of_range_yearly_bymonthday() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Invalid yearly BYMONTHDAY",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: None,
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![32],
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let error = store
            .upsert_event(&event)
            .expect_err("yearly BYMONTHDAY beyond 31 must not persist");
        assert!(
            error
                .to_string()
                .contains("invalid event recurrence definition")
        );
        assert_eq!(store.event_count().expect("event count"), 0);
    }

    #[test]
    fn yearly_by_month_and_month_day_roundtrips_through_event_storage() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 6, 15).expect("day");
        let mut event = TemporalEvent::new(
            "Yearly month-day selectors",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(8),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: vec![1, 7, 12],
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![1, -1],
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        store.upsert_event(&event).expect("save");
        let loaded = store
            .event_by_id(event.id)
            .expect("load")
            .expect("stored event");
        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn store_rejects_by_month_day_without_supported_month_context() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Invalid BYMONTHDAY",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Weekly,
            interval: 1,
            count: None,
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![15],
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let error = store
            .upsert_event(&event)
            .expect_err("non-monthly BYMONTHDAY must not persist");
        assert!(
            error
                .to_string()
                .contains("invalid event recurrence definition")
        );
        assert_eq!(store.event_count().expect("event count"), 0);
    }

    #[test]
    fn monthly_by_month_day_roundtrips_through_event_storage() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Monthly selectors",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: Some(8),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![-1, 1, 15, 31],
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        store.upsert_event(&event).expect("save");
        let loaded = store
            .event_by_id(event.id)
            .expect("load")
            .expect("stored event");
        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn store_rejects_custom_week_start_without_weekly_byday() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule, RecurrenceWeekday};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Invalid WKST",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Weekly,
            interval: 1,
            count: None,
            until: None,
            week_start: RecurrenceWeekday::Sunday,
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let error = store
            .upsert_event(&event)
            .expect_err("custom WKST without BYDAY must not persist");
        assert!(
            error
                .to_string()
                .contains("invalid event recurrence definition")
        );
        assert_eq!(store.event_count().expect("event count"), 0);
    }

    #[test]
    fn store_rejects_duplicate_daily_byday_weekdays() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule, RecurrenceWeekday};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Invalid duplicate BYDAY",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Daily,
            interval: 1,
            count: None,
            until: None,
            week_start: Default::default(),
            by_weekday: vec![RecurrenceWeekday::Monday, RecurrenceWeekday::Monday],
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let error = store
            .upsert_event(&event)
            .expect_err("duplicate daily BYDAY weekday must not persist");
        assert!(
            error
                .to_string()
                .contains("invalid event recurrence definition")
        );
        assert_eq!(store.event_count().expect("event count"), 0);
    }

    #[test]
    fn store_rejects_conflicting_recurrence_exception() {
        use crate::domain::{RecurrenceFrequency, RecurrenceOverride, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let excluded = TimeSpec::DateOnly {
            start: day + chrono::Duration::days(1),
            end_exclusive: None,
        };
        let mut event = TemporalEvent::new(
            "Conflicting exception",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Daily,
            interval: 1,
            count: None,
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: vec![excluded.clone()],
            overrides: vec![RecurrenceOverride {
                original: excluded,
                replacement: None,
                cancelled: true,
            }],
        });

        let error = store
            .upsert_event(&event)
            .expect_err("conflicting exception must not persist");
        assert!(
            error
                .to_string()
                .contains("invalid event recurrence definition")
        );
        assert_eq!(store.event_count().expect("event count"), 0);
    }

    #[test]
    fn store_rejects_override_for_non_occurrence() {
        use crate::domain::{RecurrenceFrequency, RecurrenceOverride, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Invalid detached override",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Weekly,
            interval: 1,
            count: Some(2),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: vec![RecurrenceOverride {
                original: TimeSpec::DateOnly {
                    start: day + chrono::Duration::days(2),
                    end_exclusive: None,
                },
                replacement: None,
                cancelled: true,
            }],
        });

        let error = store
            .upsert_event(&event)
            .expect_err("phantom override must not persist");
        assert!(
            error
                .to_string()
                .contains("invalid event recurrence definition")
        );
        assert_eq!(store.event_count().expect("event count"), 0);
    }

    #[test]
    fn recurring_events_are_retrievable_outside_base_window() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let start = NaiveDate::from_ymd_opt(2020, 1, 1).expect("start");
        let mut recurring = TemporalEvent::new(
            "Long-lived recurrence",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        recurring.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Weekly,
            interval: 1,
            count: None,
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });
        store.upsert_event(&recurring).expect("save recurring");

        let ordinary = TemporalEvent::new(
            "Ordinary",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        store.upsert_event(&ordinary).expect("save ordinary");

        let loaded = store.recurring_events().expect("recurring events");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, recurring.id);
    }

    #[test]
    fn recurrence_definition_roundtrips_through_event_storage() {
        use crate::domain::{RecurrenceFrequency, RecurrenceOverride, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("date");
        let mut event = TemporalEvent::new(
            "Recurring",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Weekly,
            interval: 2,
            count: Some(5),
            until: Some(NaiveDate::from_ymd_opt(2027, 1, 1).expect("until")),
            week_start: crate::domain::RecurrenceWeekday::Sunday,
            by_weekday: vec![
                crate::domain::RecurrenceWeekday::Monday,
                crate::domain::RecurrenceWeekday::Wednesday,
                crate::domain::RecurrenceWeekday::Friday,
            ],
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: vec![TimeSpec::DateOnly {
                start: day + chrono::Duration::days(1),
                end_exclusive: None,
            }],
            exdates: vec![TimeSpec::DateOnly {
                start: day + chrono::Duration::days(14),
                end_exclusive: None,
            }],
            overrides: vec![RecurrenceOverride {
                original: TimeSpec::DateOnly {
                    start: day + chrono::Duration::days(16),
                    end_exclusive: None,
                },
                replacement: Some(TimeSpec::DateOnly {
                    start: day + chrono::Duration::days(17),
                    end_exclusive: None,
                }),
                cancelled: false,
            }],
        });

        store.upsert_event(&event).expect("save");
        let loaded = store
            .event_by_id(event.id)
            .expect("load")
            .expect("stored event");

        assert_eq!(loaded.recurrence, event.recurrence);
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
    fn refresh_attempt_history_records_success_failure_and_incomplete_attempts() {
        let store = TemporalStore::open_in_memory().expect("store");

        let success = store
            .begin_refresh_attempt("taria_workspace", "bootstrap")
            .expect("success attempt");
        store
            .finish_refresh_attempt(
                success,
                true,
                Some("release:success"),
                Some("imported"),
                None,
            )
            .expect("finish success");

        let failure = store
            .begin_refresh_attempt("taria_workspace", "production")
            .expect("failure attempt");
        store
            .finish_refresh_attempt(failure, false, None, None, Some("integrity mismatch"))
            .expect("finish failure");

        let incomplete = store
            .begin_refresh_attempt("ics", "calendar.ics")
            .expect("incomplete attempt");

        let attempts = store.source_refresh_attempts(10).expect("attempt history");
        assert_eq!(attempts.len(), 3);

        let success = attempts
            .iter()
            .find(|attempt| attempt.id == success)
            .expect("success record");
        assert_eq!(success.success, Some(true));
        assert_eq!(success.release_id.as_deref(), Some("release:success"));
        assert!(success.completed_at.is_some());

        let failure = attempts
            .iter()
            .find(|attempt| attempt.id == failure)
            .expect("failure record");
        assert_eq!(failure.success, Some(false));
        assert_eq!(failure.error.as_deref(), Some("integrity mismatch"));

        let incomplete = attempts
            .iter()
            .find(|attempt| attempt.id == incomplete)
            .expect("incomplete record");
        assert_eq!(incomplete.success, None);
        assert!(incomplete.completed_at.is_none());
    }

    #[test]
    fn source_event_counts_cover_all_canonical_events() {
        let store = TemporalStore::open_in_memory().expect("store");
        let first = TemporalSource::new("First source", SourceKind::Ics, SourceAuthority::Official);
        let second =
            TemporalSource::new("Second source", SourceKind::Taria, SourceAuthority::Derived);
        store.upsert_source(&first).expect("first source");
        store.upsert_source(&second).expect("second source");

        let day = NaiveDate::from_ymd_opt(2026, 10, 4).expect("date");
        for (index, source_id) in [first.id, first.id, second.id].into_iter().enumerate() {
            let mut event = TemporalEvent::new(
                format!("Event {index}"),
                TimeSpec::DateOnly {
                    start: day,
                    end_exclusive: None,
                },
            );
            event.source_id = Some(source_id);
            store.upsert_event(&event).expect("event");
        }

        let counts = store.source_event_counts().expect("source event counts");
        assert_eq!(counts.get(&first.id), Some(&2));
        assert_eq!(counts.get(&second.id), Some(&1));
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
    fn taria_release_id_rejects_changed_manifest_content() {
        let store = TemporalStore::open_in_memory().expect("store");
        let release = TariaReleaseRecord {
            release_id: "temporal-bundle-release:test".to_string(),
            channel: "bootstrap".to_string(),
            status: "bootstrap-partial".to_string(),
            production_complete: false,
            manifest_path: "/tmp/release.json".to_string(),
            manifest_sha256: "hash-one".to_string(),
            generated_at: Some("2026-10-04T00:00:00Z".to_string()),
            coverage_json: "{}".to_string(),
            manifest_json: "{}".to_string(),
        };
        store.upsert_taria_release(&release).expect("first release");

        let mut changed = release.clone();
        changed.manifest_sha256 = "hash-two".to_string();
        changed.manifest_json = r#"{"changed":true}"#.to_string();

        let error = store
            .upsert_taria_release(&changed)
            .expect_err("immutable release must reject changed content");
        assert!(error.to_string().contains("changed content"));
        assert_eq!(store.taria_release_count().expect("count"), 1);
    }

    #[test]
    fn calendar_set_id_rejects_changed_content_but_can_join_multiple_releases() {
        let store = TemporalStore::open_in_memory().expect("store");

        for release_id in ["release:one", "release:two"] {
            store
                .upsert_taria_release(&TariaReleaseRecord {
                    release_id: release_id.to_string(),
                    channel: "bootstrap".to_string(),
                    status: "bootstrap-partial".to_string(),
                    production_complete: false,
                    manifest_path: format!("/{release_id}.json"),
                    manifest_sha256: format!("{release_id}:hash"),
                    generated_at: None,
                    coverage_json: "{}".to_string(),
                    manifest_json: format!(r#"{{"release_id":"{release_id}"}}"#),
                })
                .expect("release");
        }

        let base = TariaCalendarSetRecord {
            calendar_set_id: "calendar-set:immutable".to_string(),
            release_id: "release:one".to_string(),
            bundle_ref: "bundle:temporal/politics-government".to_string(),
            projection_ref: "projection:politics".to_string(),
            input_reconciled_event_set_ref: "reconciled-set:one".to_string(),
            source_path: "/tmp/calendar-set.json".to_string(),
            content_sha256: "calendar-hash".to_string(),
            raw_json: r#"{"calendar_set_id":"calendar-set:immutable"}"#.to_string(),
        };

        store
            .replace_taria_calendar_set(&base, &[], &[])
            .expect("first association");

        let mut reused = base.clone();
        reused.release_id = "release:two".to_string();
        store
            .replace_taria_calendar_set(&reused, &[], &[])
            .expect("second release may reuse immutable CalendarSet");

        let mut changed = reused;
        changed.content_sha256 = "different-hash".to_string();
        let error = store
            .replace_taria_calendar_set(&changed, &[], &[])
            .expect_err("changed CalendarSet must be rejected");
        assert!(error.to_string().contains("changed content"));
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
            SortDirection, SortField, SortRule, TableColumn,
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
            calendar_layout: CalendarLayout::Timeline,
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
                saved_view_id: Some(Uuid::new_v4()),
                query: EventQuery {
                    status: Some(EventStatus::Cancelled),
                    ..EventQuery::default()
                },
            }],
            table_columns: vec![TableColumn::Title, TableColumn::Date, TableColumn::Status],
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
        assert_eq!(loaded[0].calendar_layout, CalendarLayout::Timeline);
        assert_eq!(loaded[0].group_by, GroupBy::Jurisdiction);
        assert_eq!(loaded[0].color_by, ColorBy::EventType);
        assert_eq!(loaded[0].color_rules.len(), 1);
        assert_eq!(loaded[0].composition_layers.len(), 1);
        assert_eq!(loaded[0].overlays.len(), 1);
        assert_eq!(
            loaded[0].table_columns,
            vec![TableColumn::Title, TableColumn::Date, TableColumn::Status,]
        );
        assert_eq!(loaded[0].sort_rules.len(), 1);
        store.delete_saved_view(view.id).expect("delete");
        assert!(store.list_saved_views().expect("list").is_empty());
    }

    #[test]
    fn saved_view_store_rejects_reference_cycle() {
        use crate::calendar::CalendarView;
        use crate::query::{
            ColorBy, CompositionLayer, CompositionOperator, EventQuery, GroupBy, SavedView,
            SortRule, default_table_columns,
        };

        let store = TemporalStore::open_in_memory().expect("store");
        let a_id = Uuid::new_v4();
        let b_id = Uuid::new_v4();

        let make_view = |id, name: &str, referenced_id| SavedView {
            id,
            name: name.to_string(),
            query: EventQuery::default(),
            hidden_source_ids: std::collections::BTreeSet::new(),
            calendar_view: CalendarView::Month,
            calendar_layout: CalendarLayout::Grid,
            group_by: GroupBy::Date,
            sort_rules: vec![SortRule::default()],
            color_by: ColorBy::Status,
            color_rules: Vec::new(),
            composition_layers: vec![CompositionLayer {
                id: Uuid::new_v4(),
                name: "Reference".to_string(),
                enabled: true,
                operator: CompositionOperator::Union,
                saved_view_id: Some(referenced_id),
                query: EventQuery::default(),
            }],
            overlays: Vec::new(),
            table_columns: default_table_columns(),
            display_timezone: "America/Mexico_City".to_string(),
            week_start_monday: false,
        };

        store
            .upsert_saved_view(&make_view(a_id, "A", b_id))
            .expect("dangling forward reference is allowed");

        let error = store
            .upsert_saved_view(&make_view(b_id, "B", a_id))
            .expect_err("cycle must be rejected");
        assert!(error.to_string().contains("composition cycle rejected"));
        assert_eq!(store.list_saved_views().expect("views").len(), 1);
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

    #[test]
    fn yearly_plain_byday_roundtrips_through_event_storage() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule, RecurrenceWeekday};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "March Mondays",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(5),
            until: None,
            week_start: Default::default(),
            by_weekday: vec![RecurrenceWeekday::Monday],
            by_month: vec![3],
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: vec![-1],
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        store.upsert_event(&event).expect("persist recurrence");
        let loaded = store.event_by_id(event.id).expect("query").expect("event");

        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn whole_year_ordinal_byday_roundtrips_through_event_storage() {
        use crate::domain::{
            RecurrenceFrequency, RecurrenceOrdinalWeekday, RecurrenceRule, RecurrenceWeekday,
        };

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Fifty-third Friday",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(2),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: vec![RecurrenceOrdinalWeekday::new(53, RecurrenceWeekday::Friday)],
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        store.upsert_event(&event).expect("persist recurrence");
        let loaded = store.event_by_id(event.id).expect("query").expect("event");

        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn yearly_bymonthday_without_bymonth_roundtrips_through_event_storage() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 1, 20).expect("day");
        let mut event = TemporalEvent::new(
            "First and last day",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(6),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![1, -1],
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: vec![-1],
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        store.upsert_event(&event).expect("persist recurrence");
        let loaded = store.event_by_id(event.id).expect("query").expect("event");

        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn daily_byday_roundtrips_through_event_storage() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule, RecurrenceWeekday};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Weekday recurrence",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Daily,
            interval: 2,
            count: Some(6),
            until: None,
            week_start: Default::default(),
            by_weekday: vec![
                RecurrenceWeekday::Monday,
                RecurrenceWeekday::Wednesday,
                RecurrenceWeekday::Friday,
            ],
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        store.upsert_event(&event).expect("persist recurrence");
        let loaded = store.event_by_id(event.id).expect("query").expect("event");

        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn daily_bymonth_roundtrips_through_event_storage() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule, RecurrenceWeekday};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 1, 30).expect("day");
        let mut event = TemporalEvent::new(
            "Selected-month weekdays",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Daily,
            interval: 1,
            count: Some(5),
            until: None,
            week_start: Default::default(),
            by_weekday: vec![RecurrenceWeekday::Monday, RecurrenceWeekday::Friday],
            by_month: vec![1, 3],
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        store.upsert_event(&event).expect("persist recurrence");
        let loaded = store.event_by_id(event.id).expect("query").expect("event");

        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn monthly_bymonth_roundtrips_through_event_storage() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule, RecurrenceWeekday};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "March Mondays",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: Some(4),
            until: None,
            week_start: Default::default(),
            by_weekday: vec![RecurrenceWeekday::Monday],
            by_month: vec![3, 9],
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: vec![-1],
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        store.upsert_event(&event).expect("persist recurrence");
        let loaded = store.event_by_id(event.id).expect("query").expect("event");

        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn daily_bymonthday_roundtrips_through_event_storage() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule, RecurrenceWeekday};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 1, 30).expect("day");
        let mut event = TemporalEvent::new(
            "Selected daily month-days",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Daily,
            interval: 1,
            count: Some(6),
            until: None,
            week_start: Default::default(),
            by_weekday: vec![RecurrenceWeekday::Monday, RecurrenceWeekday::Friday],
            by_month: vec![1, 3],
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![1, -1],
            by_month_weekday: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        store.upsert_event(&event).expect("persist recurrence");
        let loaded = store.event_by_id(event.id).expect("query").expect("event");

        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn byhour_roundtrips_through_event_storage() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let start = NaiveDateTime::parse_from_str("2026-10-05T08:15:00", "%Y-%m-%dT%H:%M:%S")
            .expect("start");
        let mut event = TemporalEvent::new(
            "Timed recurrence",
            TimeSpec::Floating {
                start,
                end: None,
                source_timezone: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.count = Some(4);
        rule.by_hour = vec![9, 17];
        rule.by_set_pos = vec![2];
        event.recurrence = Some(rule);

        store.upsert_event(&event).expect("persist BYHOUR");
        let loaded = store.event_by_id(event.id).expect("query").expect("event");

        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn store_rejects_byhour_on_date_only_recurrence() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Invalid date-only BYHOUR",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.by_hour = vec![9];
        event.recurrence = Some(rule);

        let error = store
            .upsert_event(&event)
            .expect_err("DATE-valued recurrence must reject BYHOUR");
        assert!(
            error
                .to_string()
                .contains("invalid event recurrence definition")
        );
        assert_eq!(store.event_count().expect("event count"), 0);
    }

    #[test]
    fn byminute_roundtrips_through_event_storage() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let start = NaiveDateTime::parse_from_str("2026-10-05T08:10:30", "%Y-%m-%dT%H:%M:%S")
            .expect("start");
        let mut event = TemporalEvent::new(
            "Minute recurrence",
            TimeSpec::Floating {
                start,
                end: None,
                source_timezone: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.count = Some(4);
        rule.by_hour = vec![9, 17];
        rule.by_minute = vec![15, 45];
        rule.by_set_pos = vec![-1];
        event.recurrence = Some(rule);

        store.upsert_event(&event).expect("persist BYMINUTE");
        let loaded = store.event_by_id(event.id).expect("query").expect("event");

        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn store_rejects_byminute_on_date_only_recurrence() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Invalid date-only BYMINUTE",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.by_minute = vec![15];
        event.recurrence = Some(rule);

        let error = store
            .upsert_event(&event)
            .expect_err("DATE-valued recurrence must reject BYMINUTE");
        assert!(
            error
                .to_string()
                .contains("invalid event recurrence definition")
        );
        assert_eq!(store.event_count().expect("event count"), 0);
    }

    #[test]
    fn bysecond_roundtrips_through_event_storage() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let start = NaiveDateTime::parse_from_str("2026-10-05T08:10:20", "%Y-%m-%dT%H:%M:%S")
            .expect("start");
        let mut event = TemporalEvent::new(
            "Second recurrence",
            TimeSpec::Floating {
                start,
                end: None,
                source_timezone: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.count = Some(4);
        rule.by_hour = vec![9, 17];
        rule.by_minute = vec![15, 45];
        rule.by_second = vec![5, 35];
        rule.by_set_pos = vec![-1];
        event.recurrence = Some(rule);

        store.upsert_event(&event).expect("persist BYSECOND");
        let loaded = store.event_by_id(event.id).expect("query").expect("event");

        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn store_rejects_bysecond_on_date_only_recurrence() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Invalid date-only BYSECOND",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.by_second = vec![15];
        event.recurrence = Some(rule);

        let error = store
            .upsert_event(&event)
            .expect_err("DATE-valued recurrence must reject BYSECOND");
        assert!(
            error
                .to_string()
                .contains("invalid event recurrence definition")
        );
        assert_eq!(store.event_count().expect("event count"), 0);
    }

    #[test]
    fn hourly_recurrence_roundtrips_through_event_storage() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule, RecurrenceWeekday};

        let store = TemporalStore::open_in_memory().expect("store");
        let start = NaiveDateTime::parse_from_str("2026-10-05T08:30:20", "%Y-%m-%dT%H:%M:%S")
            .expect("start");
        let mut event = TemporalEvent::new(
            "Hourly persisted recurrence",
            TimeSpec::Floating {
                start,
                end: None,
                source_timezone: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Hourly);
        rule.interval = 2;
        rule.count = Some(6);
        rule.by_month = vec![10];
        rule.by_weekday = vec![RecurrenceWeekday::Monday];
        rule.by_month_day = vec![5];
        rule.by_hour = vec![8, 10, 12];
        rule.by_minute = vec![15, 45];
        rule.by_second = vec![5, 35];
        rule.by_set_pos = vec![-1];
        event.recurrence = Some(rule);

        store
            .upsert_event(&event)
            .expect("persist hourly recurrence");
        let loaded = store.event_by_id(event.id).expect("query").expect("event");

        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn store_rejects_hourly_recurrence_on_date_only_base() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Invalid date-only hourly",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Hourly));

        let error = store
            .upsert_event(&event)
            .expect_err("DATE-valued recurrence must reject HOURLY");
        assert!(
            error
                .to_string()
                .contains("invalid event recurrence definition")
        );
        assert_eq!(store.event_count().expect("event count"), 0);
    }

    #[test]
    fn minutely_recurrence_roundtrips_through_event_storage() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule, RecurrenceWeekday};

        let store = TemporalStore::open_in_memory().expect("store");
        let start = NaiveDateTime::parse_from_str("2026-10-05T09:14:30", "%Y-%m-%dT%H:%M:%S")
            .expect("start");
        let mut event = TemporalEvent::new(
            "Minutely persisted recurrence",
            TimeSpec::Floating {
                start,
                end: None,
                source_timezone: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Minutely);
        rule.interval = 5;
        rule.count = Some(6);
        rule.by_month = vec![10];
        rule.by_weekday = vec![RecurrenceWeekday::Monday];
        rule.by_month_day = vec![5];
        rule.by_hour = vec![9, 10];
        rule.by_minute = vec![15, 45];
        rule.by_second = vec![5, 35];
        rule.by_set_pos = vec![-1];
        event.recurrence = Some(rule);

        store
            .upsert_event(&event)
            .expect("persist minutely recurrence");
        let loaded = store.event_by_id(event.id).expect("query").expect("event");

        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn store_rejects_minutely_recurrence_on_date_only_base() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Invalid date-only minutely",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Minutely));

        let error = store
            .upsert_event(&event)
            .expect_err("DATE-valued recurrence must reject MINUTELY");
        assert!(
            error
                .to_string()
                .contains("invalid event recurrence definition")
        );
        assert_eq!(store.event_count().expect("event count"), 0);
    }

    #[test]
    fn secondly_recurrence_roundtrips_through_event_storage() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule, RecurrenceWeekday};

        let store = TemporalStore::open_in_memory().expect("store");
        let start = NaiveDateTime::parse_from_str("2026-10-05T09:14:58", "%Y-%m-%dT%H:%M:%S")
            .expect("start");
        let mut event = TemporalEvent::new(
            "Secondly persisted recurrence",
            TimeSpec::Floating {
                start,
                end: None,
                source_timezone: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Secondly);
        rule.interval = 5;
        rule.count = Some(6);
        rule.by_month = vec![10];
        rule.by_weekday = vec![RecurrenceWeekday::Monday];
        rule.by_month_day = vec![5];
        rule.by_hour = vec![9];
        rule.by_minute = vec![15, 45];
        rule.by_second = vec![5, 35];
        rule.by_set_pos = vec![-1];
        event.recurrence = Some(rule);

        store
            .upsert_event(&event)
            .expect("persist secondly recurrence");
        let loaded = store.event_by_id(event.id).expect("query").expect("event");

        assert_eq!(loaded.recurrence, event.recurrence);
    }

    #[test]
    fn store_rejects_secondly_recurrence_on_date_only_base() {
        use crate::domain::{RecurrenceFrequency, RecurrenceRule};

        let store = TemporalStore::open_in_memory().expect("store");
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Invalid date-only secondly",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Secondly));

        let error = store
            .upsert_event(&event)
            .expect_err("DATE-valued recurrence must reject SECONDLY");
        assert!(
            error
                .to_string()
                .contains("invalid event recurrence definition")
        );
        assert_eq!(store.event_count().expect("event count"), 0);
    }
}
