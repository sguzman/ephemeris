use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, anyhow};
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::store::TemporalStore;
use crate::taria::{
    TariaImportReport, import_compact_reconciled_event_index_file,
    import_reconciled_event_set_file,
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
    pub skipped: Vec<String>,
}

impl TariaWorkspaceUpdateReport {
    pub fn summary(&self) -> String {
        let mut summary = format!(
            "{} via {}: {} artifacts imported, {} skipped; {} events ({} created, {} updated, {} unchanged)",
            self.release_id,
            self.channel,
            self.imported_artifacts,
            self.skipped_artifacts,
            self.imported_events,
            self.created,
            self.updated,
            self.unchanged
        );
        if self.retained_missing > 0 {
            summary.push_str(&format!(
                ", {} retained missing",
                self.retained_missing
            ));
        }
        summary
    }
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

    if path
        .join(RELEASE_REGISTRY_RELATIVE)
        .is_file()
    {
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
    let candidates = [
        cwd.clone(),
        cwd.join("..").join("taria"),
        cwd.join("taria"),
        cwd.join(".."),
    ];

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
        if artifacts.is_empty() {
            return Ok(report);
        }

        // Production bundles overlap by design. Until Ephemeris has release-level
        // global identity reconciliation plus CalendarSet membership persistence,
        // importing each projection independently would duplicate canonical events.
        report.skipped_artifacts += artifacts.len();
        report.skipped.push(format!(
            "{} production bundle_artifacts present; release-level cross-bundle deduplication is not materialized yet",
            artifacts.len()
        ));
    }

    if report.imported_artifacts == 0 && report.skipped_artifacts == 0 {
        return Err(anyhow!(
            "Taria release {} contains neither shards[] nor bundle_artifacts[]",
            report.release_id
        ));
    }

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

    if let Some(relative) = optional_string(shard, "event_index_path") {
        let path = resolve_relative_artifact(root, &relative)?;
        validate_declared_hash(
            &path,
            optional_string(shard, "event_index_content_sha256").as_deref(),
        )?;
        let imported =
            import_compact_reconciled_event_index_file(store, &path, Some(&bundle_ref))?;
        merge_import_report(report, &imported);
        report.imported_artifacts += 1;
        return Ok(());
    }

    if let Some(relative) = optional_string(shard, "reconciled_event_set_path") {
        let path = resolve_relative_artifact(root, &relative)?;
        validate_declared_hash(
            &path,
            optional_string(shard, "reconciled_event_set_sha256")
                .or_else(|| optional_string(shard, "reconciled_event_set_content_sha256"))
                .as_deref(),
        )?;
        let imported = import_reconciled_event_set_file(store, &path)?;
        merge_import_report(report, &imported);
        report.imported_artifacts += 1;
        return Ok(());
    }

    report.skipped_artifacts += 1;
    report.skipped.push(format!(
        "{shard_id}: populated shard has no accepted reconciled event payload path"
    ));
    Ok(())
}

fn merge_import_report(
    aggregate: &mut TariaWorkspaceUpdateReport,
    imported: &TariaImportReport,
) {
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

fn validate_declared_hash(path: &Path, expected: Option<&str>) -> anyhow::Result<()> {
    let Some(expected) = expected else {
        return Err(anyhow!(
            "Taria release does not declare an integrity hash for {}",
            path.display()
        ));
    };

    let bytes =
        std::fs::read(path).with_context(|| format!("failed to hash {}", path.display()))?;
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected.to_ascii_lowercase() {
        return Err(anyhow!(
            "Taria artifact hash mismatch for {}: expected {}, got {}",
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
    fn local_update_imports_compact_payload_and_reports_unsupported_shard() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("resourcearium");
        let compact_path = root.join("derived/politics/event-index.json");
        write(
            &compact_path,
            r#"{
              "schema_version": 1,
              "kind": "CompactReconciledEventIndex",
              "projection_ref": "projection:test-politics",
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
        let compact_hash = hash(&compact_path);

        write(
            &root.join("examples/bundle-releases/current.json"),
            &format!(
                r#"{{
                  "release_id": "temporal-bundle-release:test",
                  "schema_version": 1,
                  "status": "bootstrap-partial",
                  "shards": [
                    {{
                      "shard_id": "shard:politics",
                      "bundle_ref": "bundle:temporal/politics-government",
                      "event_index_path": "derived/politics/event-index.json",
                      "event_index_content_sha256": "{compact_hash}"
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
        assert_eq!(store.event_count().expect("count"), 1);
    }

    #[test]
    fn rejects_hash_mismatch() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("resourcearium");
        write(
            &root.join("derived/politics/event-index.json"),
            r#"{"schema_version":1,"kind":"CompactReconciledEventIndex","projection_ref":"projection:test","events":[]}"#,
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
        assert!(error.to_string().contains("hash mismatch"));
        assert_eq!(store.event_count().expect("count"), 0);
    }
}
