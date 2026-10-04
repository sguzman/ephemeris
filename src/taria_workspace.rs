use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, anyhow};
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::store::{
    TariaCalendarMembershipRecord, TariaCalendarSetRecord, TariaProjectedCalendarRecord,
    TariaReleaseRecord, TemporalStore,
};
use crate::taria::{
    TariaImportReport, import_compact_reconciled_event_index_file, import_reconciled_event_set_file,
};

const RELEASE_REGISTRY_RELATIVE: &str = "registry/temporal-bundle-releases.yml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TariaWorkspaceUpdateReport {
    pub resourcearium_root: PathBuf,
    pub channel: String,
    pub release_id: String,
    pub release_status: String,
    pub manifest_path: PathBuf,
    pub imported_artifacts: usize,
    pub skipped_artifacts: usize,
    pub imported_events: usize,
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub retained_missing: usize,
    pub imprecise: usize,
    pub blocked_or_undated: usize,
    pub calendar_sets_imported: usize,
    pub calendar_memberships: usize,
    pub resolved_calendar_memberships: usize,
    pub skipped: Vec<String>,
}

impl TariaWorkspaceUpdateReport {
    pub fn summary(&self) -> String {
        let mut summary = format!(
            "{} via {}: {} payloads imported, {} skipped; {} events ({} created, {} updated, {} unchanged); {} CalendarSets / {} memberships",
            self.release_id,
            self.channel,
            self.imported_artifacts,
            self.skipped_artifacts,
            self.imported_events,
            self.created,
            self.updated,
            self.unchanged,
            self.calendar_sets_imported,
            self.calendar_memberships
        );
        if self.retained_missing > 0 {
            summary.push_str(&format!(", {} retained missing", self.retained_missing));
        }
        if self.calendar_memberships > self.resolved_calendar_memberships {
            summary.push_str(&format!(
                ", {} memberships awaiting event payload",
                self.calendar_memberships - self.resolved_calendar_memberships
            ));
        }
        summary
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IntegrityMode {
    FileSha256,
    ContentFingerprint,
}

#[derive(Debug, Clone, Copy)]
struct CalendarSetArtifactSpec<'a> {
    release_id: &'a str,
    bundle_ref: &'a str,
    container: &'a Map<String, Value>,
    path_field: &'a str,
    hash_fields: &'a [&'a str],
    integrity_mode: IntegrityMode,
}

#[derive(Debug, Deserialize)]
struct ReleaseRegistry {
    channels: BTreeMap<String, ReleaseChannel>,
}

#[derive(Debug, Deserialize)]
struct ReleaseChannel {
    current_release_ref: Option<String>,
    manifest_path: Option<String>,
}

pub fn normalize_resourcearium_root(path: impl AsRef<Path>) -> anyhow::Result<PathBuf> {
    let path = path.as_ref();

    if path.is_file()
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "temporal-bundle-releases.yml")
    {
        let registry = path
            .parent()
            .ok_or_else(|| anyhow!("Taria release registry has no parent directory"))?;
        let root = registry
            .parent()
            .ok_or_else(|| anyhow!("Taria registry directory has no Resourcearium parent"))?;
        return validate_resourcearium_root(root);
    }

    if path.join(RELEASE_REGISTRY_RELATIVE).is_file() {
        return validate_resourcearium_root(path);
    }

    let nested = path.join("incubator").join("resourcearium");
    if nested.join(RELEASE_REGISTRY_RELATIVE).is_file() {
        return validate_resourcearium_root(&nested);
    }

    Err(anyhow!(
        "{} is neither a Resourcearium root nor a Taria repository root",
        path.display()
    ))
}

pub fn detect_resourcearium_root() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("TARIA_RESOURCEARIUM_ROOT")
        && let Ok(root) = normalize_resourcearium_root(PathBuf::from(value))
    {
        return Some(root);
    }

    let cwd = std::env::current_dir().ok()?;
    let mut candidates = vec![
        cwd.clone(),
        cwd.join("..").join("taria"),
        cwd.join("taria"),
        cwd.join(".."),
    ];

    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join("Code").join("Text").join("taria"));
    }

    candidates
        .into_iter()
        .find_map(|candidate| normalize_resourcearium_root(candidate).ok())
}

pub fn update_taria_sources(
    store: &TemporalStore,
    configured_root: impl AsRef<Path>,
    channel: &str,
) -> anyhow::Result<TariaWorkspaceUpdateReport> {
    let root = normalize_resourcearium_root(configured_root)?;
    let registry_path = root.join(RELEASE_REGISTRY_RELATIVE);
    let registry_raw = std::fs::read_to_string(&registry_path)
        .with_context(|| format!("failed to read {}", registry_path.display()))?;
    let registry: ReleaseRegistry =
        serde_yaml::from_str(&registry_raw).context("failed to decode Taria release registry")?;

    let channel_entry = registry
        .channels
        .get(channel)
        .ok_or_else(|| anyhow!("Taria release registry has no channel {channel:?}"))?;
    let release_ref = channel_entry
        .current_release_ref
        .as_deref()
        .ok_or_else(|| anyhow!("Taria channel {channel:?} has no current release"))?;
    let manifest_relative = channel_entry
        .manifest_path
        .as_deref()
        .ok_or_else(|| anyhow!("Taria channel {channel:?} has no manifest path"))?;
    let manifest_path = resolve_relative_artifact(&root, manifest_relative)?;

    let manifest_raw = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let manifest: Value =
        serde_json::from_str(&manifest_raw).context("failed to decode Taria release manifest")?;
    let object = manifest
        .as_object()
        .ok_or_else(|| anyhow!("Taria release manifest must be a JSON object"))?;

    let schema_version = object
        .get("schema_version")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    if schema_version != 1 {
        return Err(anyhow!(
            "unsupported Taria release schema version {schema_version}"
        ));
    }

    let release_id = required_string(object, "release_id")?;
    if release_id != release_ref {
        return Err(anyhow!(
            "Taria channel {channel:?} points to {release_ref}, but manifest declares {release_id}"
        ));
    }
    let release_status = required_string(object, "status")?;
    let manifest_sha256 = format!("{:x}", Sha256::digest(manifest_raw.as_bytes()));
    let coverage_json = object
        .get("coverage")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()))
        .to_string();

    store.upsert_taria_release(&TariaReleaseRecord {
        release_id: release_id.clone(),
        channel: channel.to_string(),
        status: release_status.clone(),
        production_complete: object
            .get("production_complete")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        manifest_path: manifest_path.display().to_string(),
        manifest_sha256,
        generated_at: optional_string(object, "generated_at"),
        coverage_json,
        manifest_json: manifest_raw.clone(),
    })?;

    let mut report = TariaWorkspaceUpdateReport {
        resourcearium_root: root.clone(),
        channel: channel.to_string(),
        release_id,
        release_status,
        manifest_path,
        imported_artifacts: 0,
        skipped_artifacts: 0,
        imported_events: 0,
        created: 0,
        updated: 0,
        unchanged: 0,
        retained_missing: 0,
        imprecise: 0,
        blocked_or_undated: 0,
        calendar_sets_imported: 0,
        calendar_memberships: 0,
        resolved_calendar_memberships: 0,
        skipped: Vec::new(),
    };

    if let Some(shards) = object.get("shards").and_then(Value::as_array) {
        for shard in shards {
            let shard = shard
                .as_object()
                .ok_or_else(|| anyhow!("Taria release shards[] contains a non-object value"))?;
            import_bootstrap_shard(store, &root, shard, &mut report)?;
        }
    }

    if let Some(artifacts) = object.get("bundle_artifacts").and_then(Value::as_array) {
        for artifact in artifacts {
            let artifact = artifact.as_object().ok_or_else(|| {
                anyhow!("Taria release bundle_artifacts[] contains a non-object value")
            })?;
            import_production_artifact(store, &root, artifact, &mut report)?;
        }
    }

    if report.imported_artifacts == 0
        && report.skipped_artifacts == 0
        && report.calendar_sets_imported == 0
    {
        return Err(anyhow!(
            "Taria release {} contains neither consumable shards[] nor bundle_artifacts[]",
            report.release_id
        ));
    }

    report.resolved_calendar_memberships += store.resolve_taria_calendar_memberships()?;
    report.resolved_calendar_memberships = report
        .resolved_calendar_memberships
        .min(report.calendar_memberships);

    Ok(report)
}

fn import_bootstrap_shard(
    store: &TemporalStore,
    root: &Path,
    shard: &Map<String, Value>,
    report: &mut TariaWorkspaceUpdateReport,
) -> anyhow::Result<()> {
    let shard_id = required_string(shard, "shard_id")?;
    let bundle_ref = required_string(shard, "bundle_ref")?;

    let imported_payload = if let Some(relative) = optional_string(shard, "event_index_path") {
        let path = resolve_relative_artifact(root, &relative)?;
        validate_declared_integrity(
            &path,
            optional_string(shard, "event_index_content_sha256").as_deref(),
            IntegrityMode::ContentFingerprint,
        )?;
        let imported = import_compact_reconciled_event_index_file(store, &path, Some(&bundle_ref))?;
        merge_import_report(report, &imported);
        report.imported_artifacts += 1;
        true
    } else if let Some(relative) = optional_string(shard, "reconciled_event_set_path") {
        let path = resolve_relative_artifact(root, &relative)?;
        validate_declared_integrity(
            &path,
            optional_string(shard, "reconciled_event_set_sha256")
                .or_else(|| optional_string(shard, "reconciled_event_set_content_sha256"))
                .as_deref(),
            IntegrityMode::ContentFingerprint,
        )?;
        let imported = import_reconciled_event_set_file(store, &path)?;
        merge_import_report(report, &imported);
        report.imported_artifacts += 1;
        true
    } else {
        false
    };

    let release_id = report.release_id.clone();
    import_calendar_set_from_container(
        store,
        root,
        CalendarSetArtifactSpec {
            release_id: &release_id,
            bundle_ref: &bundle_ref,
            container: shard,
            path_field: "calendar_set_path",
            hash_fields: &["calendar_set_content_sha256", "calendar_set_sha256"],
            integrity_mode: IntegrityMode::ContentFingerprint,
        },
        report,
    )?;

    if !imported_payload {
        report.skipped_artifacts += 1;
        report.skipped.push(format!(
            "{shard_id}: populated shard has no accepted reconciled event payload path"
        ));
    }

    Ok(())
}

fn import_production_artifact(
    store: &TemporalStore,
    root: &Path,
    artifact: &Map<String, Value>,
    report: &mut TariaWorkspaceUpdateReport,
) -> anyhow::Result<()> {
    let artifact_id = required_string(artifact, "artifact_id")?;
    let bundle_ref = required_string(artifact, "bundle_ref")?;
    let relative = required_string(artifact, "reconciled_event_set_path")?;
    let path = resolve_relative_artifact(root, &relative)?;
    validate_declared_integrity(
        &path,
        optional_string(artifact, "reconciled_event_set_sha256").as_deref(),
        IntegrityMode::FileSha256,
    )?;

    let imported = import_reconciled_event_set_file(store, &path)
        .with_context(|| format!("failed to import production artifact {artifact_id}"))?;
    merge_import_report(report, &imported);
    report.imported_artifacts += 1;

    let release_id = report.release_id.clone();
    import_calendar_set_from_container(
        store,
        root,
        CalendarSetArtifactSpec {
            release_id: &release_id,
            bundle_ref: &bundle_ref,
            container: artifact,
            path_field: "calendar_set_path",
            hash_fields: &["calendar_set_sha256", "calendar_set_content_sha256"],
            integrity_mode: IntegrityMode::FileSha256,
        },
        report,
    )?;

    Ok(())
}

fn import_calendar_set_from_container(
    store: &TemporalStore,
    root: &Path,
    spec: CalendarSetArtifactSpec<'_>,
    report: &mut TariaWorkspaceUpdateReport,
) -> anyhow::Result<()> {
    let Some(relative) = optional_string(spec.container, spec.path_field) else {
        return Ok(());
    };
    let expected_hash = spec
        .hash_fields
        .iter()
        .find_map(|field| optional_string(spec.container, field))
        .ok_or_else(|| anyhow!("CalendarSet {relative} has no declared content hash"))?;
    let path = resolve_relative_artifact(root, &relative)?;
    validate_declared_integrity(&path, Some(&expected_hash), spec.integrity_mode)?;

    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read CalendarSet {}", path.display()))?;
    let value: Value =
        serde_json::from_str(&raw).context("failed to decode Taria CalendarSet JSON")?;
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("Taria CalendarSet must be a JSON object"))?;

    let schema_version = object
        .get("schema_version")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    if schema_version != 1 {
        return Err(anyhow!(
            "unsupported Taria CalendarSet schema version {schema_version}"
        ));
    }

    let calendar_set_id = required_string(object, "calendar_set_id")?;
    let projection_ref = required_string(object, "projection_ref")?;
    let input_reconciled_event_set_ref = required_string(object, "input_reconciled_event_set_ref")?;

    let calendars = object
        .get("calendars")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Taria CalendarSet is missing calendars[]"))?
        .iter()
        .map(|calendar| {
            let calendar = calendar
                .as_object()
                .ok_or_else(|| anyhow!("CalendarSet calendars[] contains a non-object value"))?;
            let mut metadata = calendar.clone();
            metadata.remove("event_refs");
            Ok(TariaProjectedCalendarRecord {
                calendar_id: required_string(calendar, "calendar_id")?,
                name: required_string(calendar, "name")?,
                kind: required_string(calendar, "kind")?,
                metadata_json: Value::Object(metadata).to_string(),
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let mut memberships = Vec::new();
    for membership in object
        .get("event_membership")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Taria CalendarSet is missing event_membership[]"))?
    {
        let membership = membership
            .as_object()
            .ok_or_else(|| anyhow!("CalendarSet event_membership[] contains a non-object value"))?;
        let reconciled_event_ref = required_string(membership, "reconciled_event_ref")?;
        let calendar_refs = membership
            .get("calendar_refs")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("CalendarSet membership is missing calendar_refs[]"))?;
        for calendar_ref in calendar_refs {
            let calendar_ref = calendar_ref
                .as_str()
                .ok_or_else(|| anyhow!("CalendarSet calendar_refs[] contains a non-string"))?;
            memberships.push(TariaCalendarMembershipRecord {
                reconciled_event_ref: reconciled_event_ref.clone(),
                calendar_ref: calendar_ref.to_string(),
            });
        }
    }

    let result = store.replace_taria_calendar_set(
        &TariaCalendarSetRecord {
            calendar_set_id,
            release_id: spec.release_id.to_string(),
            bundle_ref: spec.bundle_ref.to_string(),
            projection_ref,
            input_reconciled_event_set_ref,
            source_path: path.display().to_string(),
            content_sha256: expected_hash,
            raw_json: raw,
        },
        &calendars,
        &memberships,
    )?;

    report.calendar_sets_imported += 1;
    report.calendar_memberships += result.memberships;
    report.resolved_calendar_memberships += result.resolved_memberships;

    Ok(())
}

fn merge_import_report(aggregate: &mut TariaWorkspaceUpdateReport, imported: &TariaImportReport) {
    aggregate.imported_events += imported.total_events;
    aggregate.created += imported.created;
    aggregate.updated += imported.updated;
    aggregate.unchanged += imported.unchanged;
    aggregate.retained_missing += imported.retained_missing;
    aggregate.imprecise += imported.imprecise;
    aggregate.blocked_or_undated += imported.blocked_or_undated;
}

fn validate_resourcearium_root(root: &Path) -> anyhow::Result<PathBuf> {
    let registry = root.join(RELEASE_REGISTRY_RELATIVE);
    if !registry.is_file() {
        return Err(anyhow!(
            "{} does not contain {}",
            root.display(),
            RELEASE_REGISTRY_RELATIVE
        ));
    }
    root.canonicalize()
        .with_context(|| format!("failed to canonicalize {}", root.display()))
}

fn resolve_relative_artifact(root: &Path, relative: &str) -> anyhow::Result<PathBuf> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(anyhow!(
            "Taria artifact path must remain under Resourcearium root: {relative}"
        ));
    }

    let path = root.join(relative_path);
    if !path.is_file() {
        return Err(anyhow!("Taria artifact does not exist: {}", path.display()));
    }

    let canonical = path
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", path.display()))?;
    if !canonical.starts_with(root) {
        return Err(anyhow!(
            "Taria artifact escapes Resourcearium root: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

fn validate_declared_integrity(
    path: &Path,
    expected: Option<&str>,
    mode: IntegrityMode,
) -> anyhow::Result<()> {
    let Some(expected) = expected else {
        return Err(anyhow!(
            "Taria release does not declare integrity metadata for {}",
            path.display()
        ));
    };

    let actual = match mode {
        IntegrityMode::FileSha256 => {
            let bytes = std::fs::read(path)
                .with_context(|| format!("failed to hash {}", path.display()))?;
            format!("{:x}", Sha256::digest(bytes))
        }
        IntegrityMode::ContentFingerprint => {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let value: Value = serde_json::from_str(&raw)
                .with_context(|| format!("failed to decode {}", path.display()))?;
            value
                .get("content_fingerprint")
                .and_then(Value::as_object)
                .and_then(|fingerprint| fingerprint.get("value"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .ok_or_else(|| {
                    anyhow!(
                        "Taria artifact {} has no content_fingerprint.value",
                        path.display()
                    )
                })?
        }
    };

    if actual != expected.to_ascii_lowercase() && actual != expected {
        return Err(anyhow!(
            "Taria artifact integrity mismatch for {}: expected {}, got {}",
            path.display(),
            expected,
            actual
        ));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use tempfile::tempdir;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, content).expect("write");
    }

    fn hash(path: &Path) -> String {
        format!("{:x}", Sha256::digest(std::fs::read(path).expect("read")))
    }

    fn write_calendar_set(
        root: &Path,
        relative: &str,
        calendar_set_id: &str,
        projection_ref: &str,
        reconciled_ref: &str,
        calendar_id: &str,
    ) -> (String, String) {
        let path = root.join(relative);
        let content_fingerprint = format!("{calendar_set_id}:fingerprint");
        write(
            &path,
            &format!(
                r#"{{
                  "schema_version": 1,
                  "calendar_set_id": "{calendar_set_id}",
                  "projection_ref": "{projection_ref}",
                  "input_reconciled_event_set_ref": "reconciled-set:test",
                  "calendars": [{{
                    "calendar_id": "{calendar_id}",
                    "name": "Test Calendar",
                    "kind": "single",
                    "event_refs": ["{reconciled_ref}"]
                  }}],
                  "event_membership": [{{
                    "reconciled_event_ref": "{reconciled_ref}",
                    "calendar_refs": ["{calendar_id}"]
                  }}],
                  "content_fingerprint": {{"algorithm":"sha256","value":"{content_fingerprint}"}}
                }}"#
            ),
        );
        (hash(&path), content_fingerprint)
    }

    #[test]
    fn accepts_taria_repo_or_resourcearium_root() {
        let dir = tempdir().expect("tempdir");
        let resourcearium = dir.path().join("incubator/resourcearium");
        write(
            &resourcearium.join(RELEASE_REGISTRY_RELATIVE),
            "version: 1\nchannels: {}\n",
        );

        assert_eq!(
            normalize_resourcearium_root(dir.path()).expect("repo root"),
            resourcearium.canonicalize().expect("canonical")
        );
        assert_eq!(
            normalize_resourcearium_root(&resourcearium).expect("resourcearium root"),
            resourcearium.canonicalize().expect("canonical")
        );
    }

    #[test]
    fn local_update_imports_compact_payload_and_calendar_membership() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("resourcearium");
        let compact_path = root.join("derived/politics/event-index.json");
        write(
            &compact_path,
            r#"{
              "schema_version": 1,
              "kind": "CompactReconciledEventIndex",
              "projection_ref": "projection:test-politics",
              "content_fingerprint": {"algorithm":"sha256","value":"compact:test:fingerprint"},
              "events": [{
                "reconciled_event_ref": "reconciled-event:test",
                "assertion_ref": "assertion:test",
                "event_ref": "event:test",
                "title": "Test election",
                "temporal_value": {"kind":"date","start":"2026-11-03","end":"2026-11-04"},
                "event_class": "election",
                "schedule_status": "confirmed",
                "categories": ["politics-government"],
                "provenance_ref": "trace:test"
              }]
            }"#,
        );
        let compact_hash = "compact:test:fingerprint";
        let (_, calendar_hash) = write_calendar_set(
            &root,
            "derived/politics/calendar-set.json",
            "calendar-set:test-politics",
            "projection:test-politics",
            "reconciled-event:test",
            "projected-calendar:test-politics",
        );

        write(
            &root.join("examples/bundle-releases/current.json"),
            &format!(
                r#"{{
                  "release_id": "temporal-bundle-release:test",
                  "schema_version": 1,
                  "status": "bootstrap-partial",
                  "production_complete": false,
                  "coverage": {{"ready_events":1}},
                  "shards": [
                    {{
                      "shard_id": "shard:politics",
                      "bundle_ref": "bundle:temporal/politics-government",
                      "event_index_path": "derived/politics/event-index.json",
                      "event_index_content_sha256": "{compact_hash}",
                      "calendar_set_path": "derived/politics/calendar-set.json",
                      "calendar_set_content_sha256": "{calendar_hash}"
                    }},
                    {{
                      "shard_id": "shard:sports",
                      "bundle_ref": "bundle:temporal/sports-competition"
                    }}
                  ]
                }}"#
            ),
        );
        write(
            &root.join(RELEASE_REGISTRY_RELATIVE),
            r#"version: 1
channels:
  bootstrap:
    current_release_ref: temporal-bundle-release:test
    manifest_path: examples/bundle-releases/current.json
"#,
        );

        let store = TemporalStore::open_in_memory().expect("store");
        let report = update_taria_sources(&store, &root, "bootstrap").expect("update");

        assert_eq!(report.release_id, "temporal-bundle-release:test");
        assert_eq!(report.imported_artifacts, 1);
        assert_eq!(report.skipped_artifacts, 1);
        assert_eq!(report.imported_events, 1);
        assert_eq!(report.created, 1);
        assert_eq!(report.calendar_sets_imported, 1);
        assert_eq!(report.calendar_memberships, 1);
        assert_eq!(report.resolved_calendar_memberships, 1);
        assert_eq!(store.event_count().expect("count"), 1);
        assert_eq!(store.taria_release_count().expect("release count"), 1);
        assert_eq!(
            store
                .taria_calendar_membership_count()
                .expect("membership count"),
            1
        );
    }

    #[test]
    fn production_overlapping_bundles_share_one_canonical_event() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("resourcearium");

        let payload = |projection: &str| {
            format!(
                r#"{{
                  "schema_version": 1,
                  "reconciled_projection_event_set_id": "reconciled-set:{projection}",
                  "projection_ref": "{projection}",
                  "events": [{{
                    "reconciled_event_key": "reconciled-event:shared",
                    "event_ref": "event:shared",
                    "assertion_refs": ["assertion:shared"],
                    "retained_provenance_refs": ["trace:shared"],
                    "renderability": "ready",
                    "display_fields": {{
                      "title": "Shared event",
                      "temporal_value": {{"kind":"date","start":"2026-12-01"}},
                      "schedule_status": "confirmed",
                      "event_class": "meeting"
                    }}
                  }}]
                }}"#
            )
        };

        let politics_payload = root.join("derived/politics/reconciled.json");
        let finance_payload = root.join("derived/finance/reconciled.json");
        write(&politics_payload, &payload("projection:politics"));
        write(&finance_payload, &payload("projection:finance"));
        let politics_hash = hash(&politics_payload);
        let finance_hash = hash(&finance_payload);

        let (politics_calendar_hash, _) = write_calendar_set(
            &root,
            "derived/politics/calendar-set.json",
            "calendar-set:politics",
            "projection:politics",
            "reconciled-event:shared",
            "projected-calendar:politics",
        );
        let (finance_calendar_hash, _) = write_calendar_set(
            &root,
            "derived/finance/calendar-set.json",
            "calendar-set:finance",
            "projection:finance",
            "reconciled-event:shared",
            "projected-calendar:finance",
        );

        write(
            &root.join("examples/bundle-releases/current.json"),
            &format!(
                r#"{{
                  "release_id": "temporal-bundle-release:production:test",
                  "schema_version": 1,
                  "status": "production-partial",
                  "production_complete": false,
                  "bundle_artifacts": [
                    {{
                      "artifact_id": "bundle-artifact:politics",
                      "bundle_ref": "bundle:temporal/politics-government",
                      "reconciled_event_set_path": "derived/politics/reconciled.json",
                      "reconciled_event_set_sha256": "{politics_hash}",
                      "calendar_set_path": "derived/politics/calendar-set.json",
                      "calendar_set_sha256": "{politics_calendar_hash}"
                    }},
                    {{
                      "artifact_id": "bundle-artifact:finance",
                      "bundle_ref": "bundle:temporal/finance-markets",
                      "reconciled_event_set_path": "derived/finance/reconciled.json",
                      "reconciled_event_set_sha256": "{finance_hash}",
                      "calendar_set_path": "derived/finance/calendar-set.json",
                      "calendar_set_sha256": "{finance_calendar_hash}"
                    }}
                  ]
                }}"#
            ),
        );
        write(
            &root.join(RELEASE_REGISTRY_RELATIVE),
            r#"version: 1
channels:
  production:
    current_release_ref: temporal-bundle-release:production:test
    manifest_path: examples/bundle-releases/current.json
"#,
        );

        let store = TemporalStore::open_in_memory().expect("store");
        let report = update_taria_sources(&store, &root, "production").expect("update");

        assert_eq!(report.imported_artifacts, 2);
        assert_eq!(report.skipped_artifacts, 0);
        assert_eq!(report.created, 1);
        assert_eq!(report.unchanged, 1);
        assert_eq!(report.calendar_sets_imported, 2);
        assert_eq!(report.calendar_memberships, 2);
        assert_eq!(report.resolved_calendar_memberships, 2);
        assert_eq!(store.event_count().expect("canonical event count"), 1);
        assert_eq!(
            store
                .taria_calendar_membership_count()
                .expect("membership count"),
            2
        );
    }

    #[test]
    fn rejects_hash_mismatch() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("resourcearium");
        write(
            &root.join("derived/politics/event-index.json"),
            r#"{"schema_version":1,"kind":"CompactReconciledEventIndex","projection_ref":"projection:test","content_fingerprint":{"algorithm":"sha256","value":"actual"},"events":[]}"#,
        );
        write(
            &root.join("examples/bundle-releases/current.json"),
            r#"{
              "release_id":"temporal-bundle-release:test",
              "schema_version":1,
              "status":"bootstrap-partial",
              "shards":[{
                "shard_id":"shard:test",
                "bundle_ref":"bundle:temporal/politics-government",
                "event_index_path":"derived/politics/event-index.json",
                "event_index_content_sha256":"deadbeef"
              }]
            }"#,
        );
        write(
            &root.join(RELEASE_REGISTRY_RELATIVE),
            r#"version: 1
channels:
  bootstrap:
    current_release_ref: temporal-bundle-release:test
    manifest_path: examples/bundle-releases/current.json
"#,
        );

        let store = TemporalStore::open_in_memory().expect("store");
        let error = update_taria_sources(&store, &root, "bootstrap").expect_err("hash failure");
        assert!(error.to_string().contains("integrity mismatch"));
        assert_eq!(store.event_count().expect("count"), 0);
    }
}
