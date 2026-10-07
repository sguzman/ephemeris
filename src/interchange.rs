use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};

use crate::domain::{
    EventCollection, EventCollectionMember, EventRelation, TemporalEvent, TemporalSource,
};
use crate::store::{CanonicalSnapshotMergeResult, TemporalStore};

pub const CANONICAL_SNAPSHOT_FORMAT: &str = "ephemeris.canonical_snapshot";
pub const CANONICAL_SNAPSHOT_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalJsonSnapshot {
    pub format: String,
    pub version: u32,
    pub sources: Vec<TemporalSource>,
    pub events: Vec<TemporalEvent>,
    #[serde(default)]
    pub relations: Vec<EventRelation>,
    #[serde(default)]
    pub collections: Vec<EventCollection>,
    #[serde(default)]
    pub collection_members: Vec<EventCollectionMember>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalJsonExportReport {
    pub source_count: usize,
    pub event_count: usize,
    pub relation_count: usize,
    pub collection_count: usize,
    pub collection_member_count: usize,
    pub output_path: PathBuf,
}

impl CanonicalJsonSnapshot {
    pub fn from_store(store: &TemporalStore) -> anyhow::Result<Self> {
        let mut sources = store.list_sources()?;
        let mut events = store.list_events()?;
        let mut relations = store.list_event_relations()?;
        let mut collections = store.list_event_collections()?;
        let mut collection_members = store.list_event_collection_members()?;
        sources.sort_by_key(|source| source.id);
        events.sort_by_key(|event| event.id);
        relations.sort_by_key(|relation| relation.id);
        collections.sort_by_key(|collection| collection.id);
        collection_members
            .sort_by_key(|member| (member.collection_id, member.position, member.event_id));

        let snapshot = Self {
            format: CANONICAL_SNAPSHOT_FORMAT.to_string(),
            version: CANONICAL_SNAPSHOT_VERSION,
            sources,
            events,
            relations,
            collections,
            collection_members,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.format != CANONICAL_SNAPSHOT_FORMAT {
            return Err(anyhow!(
                "unsupported Ephemeris JSON format {:?}",
                self.format
            ));
        }
        if !(1..=CANONICAL_SNAPSHOT_VERSION).contains(&self.version) {
            return Err(anyhow!(
                "unsupported Ephemeris canonical snapshot version {}; supported versions are 1..={}",
                self.version,
                CANONICAL_SNAPSHOT_VERSION
            ));
        }
        if self.version == 1
            && (!self.relations.is_empty()
                || !self.collections.is_empty()
                || !self.collection_members.is_empty())
        {
            return Err(anyhow!(
                "canonical snapshot version 1 cannot contain topology records"
            ));
        }

        let mut source_ids = HashSet::new();
        let mut external_refs = HashSet::new();
        for source in &self.sources {
            if !source_ids.insert(source.id) {
                return Err(anyhow!("duplicate temporal source UUID {}", source.id));
            }
            if let Some(external_ref) = source.external_ref.as_deref()
                && !external_refs.insert(external_ref)
            {
                return Err(anyhow!(
                    "duplicate temporal source external_ref {external_ref:?}"
                ));
            }
        }

        let mut event_ids = HashSet::new();
        let mut source_records = BTreeSet::new();
        for event in &self.events {
            if !event_ids.insert(event.id) {
                return Err(anyhow!("duplicate temporal event UUID {}", event.id));
            }
            if let Some(source_id) = event.source_id
                && !source_ids.contains(&source_id)
            {
                return Err(anyhow!(
                    "event {} references source {} that is not present in the snapshot",
                    event.id,
                    source_id
                ));
            }
            if let (Some(source_id), Some(record_key)) =
                (event.source_id, event.source_record_key.as_deref())
                && !source_records.insert((source_id, record_key))
            {
                return Err(anyhow!(
                    "duplicate source-record identity {} / {:?}",
                    source_id,
                    record_key
                ));
            }
            event
                .validate_recurrence()
                .with_context(|| format!("event {} has invalid recurrence", event.id))?;
        }

        if self.version >= 2 {
            let mut collection_ids = HashSet::new();
            for collection in &self.collections {
                collection
                    .validate()
                    .with_context(|| format!("collection {} is invalid", collection.id))?;
                if !collection_ids.insert(collection.id) {
                    return Err(anyhow!("duplicate event collection UUID {}", collection.id));
                }
            }

            let mut relation_ids = HashSet::new();
            let mut relation_semantics = BTreeSet::new();
            for relation in &self.relations {
                relation
                    .validate()
                    .with_context(|| format!("relation {} is invalid", relation.id))?;
                if !relation_ids.insert(relation.id) {
                    return Err(anyhow!("duplicate event relation UUID {}", relation.id));
                }
                if !event_ids.contains(&relation.from_event_id)
                    || !event_ids.contains(&relation.to_event_id)
                {
                    return Err(anyhow!(
                        "relation {} references an event that is not present in the snapshot",
                        relation.id
                    ));
                }
                if !relation_semantics.insert((
                    relation.from_event_id,
                    relation.to_event_id,
                    relation.relation_type.as_str(),
                )) {
                    return Err(anyhow!(
                        "duplicate semantic event relation {} -> {} / {:?}",
                        relation.from_event_id,
                        relation.to_event_id,
                        relation.relation_type
                    ));
                }
            }

            let mut member_pairs = HashSet::new();
            let mut members_by_collection =
                std::collections::BTreeMap::<uuid::Uuid, Vec<&EventCollectionMember>>::new();
            for member in &self.collection_members {
                if !collection_ids.contains(&member.collection_id) {
                    return Err(anyhow!(
                        "collection member references collection {} that is not present in the snapshot",
                        member.collection_id
                    ));
                }
                if !event_ids.contains(&member.event_id) {
                    return Err(anyhow!(
                        "collection member references event {} that is not present in the snapshot",
                        member.event_id
                    ));
                }
                if !member_pairs.insert((member.collection_id, member.event_id)) {
                    return Err(anyhow!(
                        "duplicate collection membership {} / {}",
                        member.collection_id,
                        member.event_id
                    ));
                }
                members_by_collection
                    .entry(member.collection_id)
                    .or_default()
                    .push(member);
            }

            for collection in &self.collections {
                let members = members_by_collection
                    .get(&collection.id)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                if collection.ordered {
                    let mut positions = members
                        .iter()
                        .map(|member| {
                            member.position.ok_or_else(|| {
                                anyhow!(
                                    "ordered collection {} has a member without a position",
                                    collection.id
                                )
                            })
                        })
                        .collect::<anyhow::Result<Vec<_>>>()?;
                    positions.sort_unstable();
                    let expected = (0..positions.len())
                        .map(|index| {
                            u32::try_from(index)
                                .context("ordered collection position cannot fit in u32")
                        })
                        .collect::<anyhow::Result<Vec<_>>>()?;
                    if positions != expected {
                        return Err(anyhow!(
                            "ordered collection {} positions must be contiguous from zero",
                            collection.id
                        ));
                    }
                } else if members.iter().any(|member| member.position.is_some()) {
                    return Err(anyhow!(
                        "unordered collection {} cannot contain member positions",
                        collection.id
                    ));
                }
            }
        }

        Ok(())
    }
}

pub fn parse_canonical_json_snapshot(raw: &str) -> anyhow::Result<CanonicalJsonSnapshot> {
    let snapshot: CanonicalJsonSnapshot =
        serde_json::from_str(raw).context("failed to parse Ephemeris canonical JSON snapshot")?;
    snapshot.validate()?;
    Ok(snapshot)
}

pub fn format_canonical_json_snapshot(snapshot: &CanonicalJsonSnapshot) -> anyhow::Result<String> {
    snapshot.validate()?;
    serde_json::to_string_pretty(snapshot)
        .context("failed to serialize Ephemeris canonical JSON snapshot")
}

pub fn import_canonical_json_snapshot(
    store: &TemporalStore,
    snapshot: &CanonicalJsonSnapshot,
) -> anyhow::Result<CanonicalSnapshotMergeResult> {
    snapshot.validate()?;
    if snapshot.version == 1 {
        store.merge_canonical_snapshot(&snapshot.sources, &snapshot.events)
    } else {
        store.merge_canonical_snapshot_with_topology(
            &snapshot.sources,
            &snapshot.events,
            &snapshot.relations,
            &snapshot.collections,
            &snapshot.collection_members,
        )
    }
}

pub fn import_canonical_json_file(
    store: &TemporalStore,
    path: impl AsRef<Path>,
) -> anyhow::Result<CanonicalSnapshotMergeResult> {
    let path = path.as_ref();
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read canonical JSON snapshot {}", path.display()))?;
    let snapshot = parse_canonical_json_snapshot(&raw)?;
    import_canonical_json_snapshot(store, &snapshot)
}

pub fn export_canonical_json_file(
    store: &TemporalStore,
    path: impl AsRef<Path>,
) -> anyhow::Result<CanonicalJsonExportReport> {
    let snapshot = CanonicalJsonSnapshot::from_store(store)?;
    let encoded = format_canonical_json_snapshot(&snapshot)?;
    let output_path = path.as_ref().to_path_buf();
    write_atomically(&output_path, &encoded)?;

    Ok(CanonicalJsonExportReport {
        source_count: snapshot.sources.len(),
        event_count: snapshot.events.len(),
        relation_count: snapshot.relations.len(),
        collection_count: snapshot.collections.len(),
        collection_member_count: snapshot.collection_members.len(),
        output_path,
    })
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
        .unwrap_or("ephemeris.json");
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
    use chrono::{Duration, TimeZone, Utc};
    use serde_json::json;
    use uuid::Uuid;

    use super::*;
    use crate::domain::{
        RecurrenceFrequency, RecurrenceRule, SourceAuthority, SourceKind, TimeSpec,
    };

    fn fixture_source() -> TemporalSource {
        let mut source = TemporalSource::new("Fixture", SourceKind::Json, SourceAuthority::Manual);
        source.external_ref = Some("json:test:fixture".to_string());
        source.read_only = false;
        source.properties = json!({"fixture": true});
        source
    }

    fn fixture_event(source: &TemporalSource) -> TemporalEvent {
        let start = Utc
            .with_ymd_and_hms(2026, 10, 7, 9, 0, 0)
            .single()
            .expect("start");
        let mut event = TemporalEvent::new(
            "Snapshot event",
            TimeSpec::Instant {
                start_utc: start,
                end_utc: Some(start + Duration::hours(1)),
                source_timezone: Some("America/Mexico_City".to_string()),
            },
        );
        event.source_id = Some(source.id);
        event.source_record_key = Some("snapshot-event".to_string());
        let mut recurrence = RecurrenceRule::new(RecurrenceFrequency::Weekly);
        recurrence.count = Some(3);
        event.recurrence = Some(recurrence);
        event.properties = json!({"nested": {"value": 42}});
        event
    }

    #[test]
    fn canonical_snapshot_roundtrips_losslessly() {
        let source = fixture_source();
        let event = fixture_event(&source);
        let snapshot = CanonicalJsonSnapshot {
            format: CANONICAL_SNAPSHOT_FORMAT.to_string(),
            version: CANONICAL_SNAPSHOT_VERSION,
            sources: vec![source],
            events: vec![event],
            relations: Vec::new(),
            collections: Vec::new(),
            collection_members: Vec::new(),
        };

        let encoded = format_canonical_json_snapshot(&snapshot).expect("encode");
        let decoded = parse_canonical_json_snapshot(&encoded).expect("decode");
        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn canonical_snapshot_merges_exact_ids_and_is_repeatable() {
        let source = fixture_source();
        let event = fixture_event(&source);
        let snapshot = CanonicalJsonSnapshot {
            format: CANONICAL_SNAPSHOT_FORMAT.to_string(),
            version: CANONICAL_SNAPSHOT_VERSION,
            sources: vec![source.clone()],
            events: vec![event.clone()],
            relations: Vec::new(),
            collections: Vec::new(),
            collection_members: Vec::new(),
        };
        let store = TemporalStore::open_in_memory().expect("store");

        let first = import_canonical_json_snapshot(&store, &snapshot).expect("first merge");
        assert_eq!(first.sources_created, 1);
        assert_eq!(first.events_created, 1);
        assert_eq!(
            store
                .source_by_id(source.id)
                .expect("source query")
                .expect("source"),
            source
        );
        assert_eq!(
            store
                .event_by_id(event.id)
                .expect("event query")
                .expect("event"),
            event
        );

        let second = import_canonical_json_snapshot(&store, &snapshot).expect("repeat merge");
        assert_eq!(second.sources_unchanged, 1);
        assert_eq!(second.events_unchanged, 1);
        assert_eq!(second.sources_created, 0);
        assert_eq!(second.events_created, 0);
    }

    #[test]
    fn canonical_snapshot_merge_updates_existing_records_without_changing_ids() {
        let source = fixture_source();
        let event = fixture_event(&source);
        let store = TemporalStore::open_in_memory().expect("store");
        let initial = CanonicalJsonSnapshot {
            format: CANONICAL_SNAPSHOT_FORMAT.to_string(),
            version: CANONICAL_SNAPSHOT_VERSION,
            sources: vec![source.clone()],
            events: vec![event.clone()],
            relations: Vec::new(),
            collections: Vec::new(),
            collection_members: Vec::new(),
        };
        import_canonical_json_snapshot(&store, &initial).expect("initial merge");

        let mut changed_source = source.clone();
        changed_source.name = "Fixture renamed".to_string();
        let mut changed_event = event.clone();
        changed_event.normalized_title = "Snapshot event updated".to_string();
        let changed = CanonicalJsonSnapshot {
            format: CANONICAL_SNAPSHOT_FORMAT.to_string(),
            version: CANONICAL_SNAPSHOT_VERSION,
            sources: vec![changed_source.clone()],
            events: vec![changed_event.clone()],
            relations: Vec::new(),
            collections: Vec::new(),
            collection_members: Vec::new(),
        };

        let report = import_canonical_json_snapshot(&store, &changed).expect("changed merge");
        assert_eq!(report.sources_updated, 1);
        assert_eq!(report.events_updated, 1);
        assert_eq!(
            store
                .source_by_id(source.id)
                .expect("source query")
                .expect("source"),
            changed_source
        );
        assert_eq!(
            store
                .event_by_id(event.id)
                .expect("event query")
                .expect("event"),
            changed_event
        );
    }

    #[test]
    fn canonical_snapshot_rejects_dangling_source_and_duplicate_record_identity() {
        let source = fixture_source();
        let mut dangling = fixture_event(&source);
        dangling.source_id = Some(Uuid::new_v4());
        let dangling_snapshot = CanonicalJsonSnapshot {
            format: CANONICAL_SNAPSHOT_FORMAT.to_string(),
            version: CANONICAL_SNAPSHOT_VERSION,
            sources: vec![source.clone()],
            events: vec![dangling],
            relations: Vec::new(),
            collections: Vec::new(),
            collection_members: Vec::new(),
        };
        assert!(dangling_snapshot.validate().is_err());

        let event = fixture_event(&source);
        let mut duplicate = event.clone();
        duplicate.id = Uuid::new_v4();
        let duplicate_snapshot = CanonicalJsonSnapshot {
            format: CANONICAL_SNAPSHOT_FORMAT.to_string(),
            version: CANONICAL_SNAPSHOT_VERSION,
            sources: vec![source],
            events: vec![event, duplicate],
            relations: Vec::new(),
            collections: Vec::new(),
            collection_members: Vec::new(),
        };
        assert!(duplicate_snapshot.validate().is_err());
    }

    #[test]
    fn canonical_snapshot_rejects_unknown_format_version() {
        let snapshot = CanonicalJsonSnapshot {
            format: CANONICAL_SNAPSHOT_FORMAT.to_string(),
            version: CANONICAL_SNAPSHOT_VERSION + 1,
            sources: Vec::new(),
            events: Vec::new(),
            relations: Vec::new(),
            collections: Vec::new(),
            collection_members: Vec::new(),
        };
        assert!(snapshot.validate().is_err());
    }

    #[test]
    fn canonical_snapshot_v1_remains_readable_without_topology() {
        let source = fixture_source();
        let event = fixture_event(&source);
        let raw = serde_json::json!({
            "format": CANONICAL_SNAPSHOT_FORMAT,
            "version": 1,
            "sources": [source],
            "events": [event],
        })
        .to_string();

        let parsed = parse_canonical_json_snapshot(&raw).expect("parse v1");
        assert_eq!(parsed.version, 1);
        assert!(parsed.relations.is_empty());
        assert!(parsed.collections.is_empty());
        assert!(parsed.collection_members.is_empty());
    }

    #[test]
    fn canonical_snapshot_v2_roundtrips_topology() {
        let source = fixture_source();
        let first = fixture_event(&source);
        let mut second = fixture_event(&source);
        second.id = Uuid::new_v4();
        second.source_record_key = Some("snapshot-event-2".to_string());
        second.normalized_title = "Second snapshot event".to_string();

        let relation = EventRelation::new(first.id, second.id, "precedes");
        let collection = EventCollection::new("Sequence", true);
        let members = vec![
            EventCollectionMember {
                collection_id: collection.id,
                event_id: second.id,
                position: Some(0),
            },
            EventCollectionMember {
                collection_id: collection.id,
                event_id: first.id,
                position: Some(1),
            },
        ];
        let snapshot = CanonicalJsonSnapshot {
            format: CANONICAL_SNAPSHOT_FORMAT.to_string(),
            version: CANONICAL_SNAPSHOT_VERSION,
            sources: vec![source],
            events: vec![first.clone(), second.clone()],
            relations: vec![relation.clone()],
            collections: vec![collection.clone()],
            collection_members: members.clone(),
        };

        let store = TemporalStore::open_in_memory().expect("store");
        let report = import_canonical_json_snapshot(&store, &snapshot).expect("merge topology");
        assert_eq!(report.relations_created, 1);
        assert_eq!(report.collections_created, 1);
        assert_eq!(report.collection_memberships_replaced, 1);
        assert_eq!(
            store.list_event_relations().expect("relations"),
            vec![relation]
        );
        assert_eq!(
            store.list_event_collections().expect("collections"),
            vec![collection.clone()]
        );
        assert_eq!(
            store
                .event_collection_members(collection.id)
                .expect("members"),
            members
        );

        let exported = CanonicalJsonSnapshot::from_store(&store).expect("snapshot");
        assert_eq!(exported.relations.len(), 1);
        assert_eq!(exported.collections.len(), 1);
        assert_eq!(exported.collection_members.len(), 2);
    }

    #[test]
    fn canonical_snapshot_file_export_is_atomic_and_reparseable() {
        let source = fixture_source();
        let event = fixture_event(&source);
        let store = TemporalStore::open_in_memory().expect("store");
        store.upsert_source(&source).expect("source");
        store.upsert_event(&event).expect("event");

        let directory = tempfile::tempdir().expect("tempdir");
        let output = directory.path().join("snapshot.ephemeris.json");
        let report = export_canonical_json_file(&store, &output).expect("export");
        assert_eq!(report.source_count, 1);
        assert_eq!(report.event_count, 1);
        assert_eq!(report.relation_count, 0);
        assert_eq!(report.collection_count, 0);
        assert_eq!(report.collection_member_count, 0);

        let raw = std::fs::read_to_string(&output).expect("read");
        let parsed = parse_canonical_json_snapshot(&raw).expect("parse");
        assert_eq!(parsed.sources, vec![source]);
        assert_eq!(parsed.events, vec![event]);
    }
}
