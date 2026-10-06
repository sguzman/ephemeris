use std::ffi::OsStr;
use std::path::PathBuf;

use anyhow::{Context, anyhow};
use ephemeris::domain::{SourceKind, TemporalSource};
use ephemeris::ics::{export_ics_source_by_id, ics_file_external_ref};
use ephemeris::store::TemporalStore;
use uuid::Uuid;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let Some(selector) = args.next() else {
        return Err(anyhow!(usage()));
    };

    if selector == OsStr::new("--help") || selector == OsStr::new("-h") {
        println!("{}", usage());
        return Ok(());
    }

    let store = TemporalStore::open_default().context("failed to open Ephemeris database")?;

    if selector == OsStr::new("--list") {
        list_ics_sources(&store)?;
        return Ok(());
    }

    let output_path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!(usage()))?;
    if args.next().is_some() {
        return Err(anyhow!(usage()));
    }

    let selector_text = selector.to_string_lossy();
    let source = resolve_ics_source(&store, &selector_text)?;
    let report = export_ics_source_by_id(&store, source.id, &output_path)?;

    println!("Exported iCalendar source: {}", source.name);
    println!("Source ID: {}", report.source_id);
    println!("Events: {}", report.total_events);
    println!("Output: {}", report.output_path.display());

    Ok(())
}

fn usage() -> &'static str {
    "usage:
  ephemeris-export --list
  ephemeris-export <source-id|external-ref|source-file|exact-name> <output.ics>"
}

fn list_ics_sources(store: &TemporalStore) -> anyhow::Result<()> {
    let counts = store.source_event_counts()?;
    let sources = store.list_sources()?;
    let mut found = false;

    for source in sources
        .into_iter()
        .filter(|source| source.kind == SourceKind::Ics)
    {
        found = true;
        let count = counts.get(&source.id).copied().unwrap_or(0);
        println!(
            "{}\t{}\t{}\t{}",
            source.id,
            count,
            source.name,
            source
                .locator
                .as_deref()
                .or(source.external_ref.as_deref())
                .unwrap_or("")
        );
    }

    if !found {
        println!("No imported ICS sources.");
    }

    Ok(())
}

fn resolve_ics_source(store: &TemporalStore, selector: &str) -> anyhow::Result<TemporalSource> {
    if let Ok(id) = Uuid::parse_str(selector) {
        let source = store
            .source_by_id(id)?
            .ok_or_else(|| anyhow!("temporal source {id} does not exist"))?;
        return require_ics_source(source, selector);
    }

    if let Some(source) = store.source_by_external_ref(selector)? {
        return require_ics_source(source, selector);
    }

    let candidate_path = PathBuf::from(selector);
    if candidate_path.exists() {
        let external_ref = ics_file_external_ref(&candidate_path)?;
        if let Some(source) = store.source_by_external_ref(&external_ref)? {
            return require_ics_source(source, selector);
        }
    }

    let matches = store
        .list_sources()?
        .into_iter()
        .filter(|source| source.kind == SourceKind::Ics && source.name == selector)
        .collect::<Vec<_>>();

    match matches.as_slice() {
        [source] => Ok(source.clone()),
        [] => Err(anyhow!(
            "no imported ICS source matches {selector:?}; use --list to inspect sources"
        )),
        _ => Err(anyhow!(
            "multiple imported ICS sources are named {selector:?}; use --list and select by source ID"
        )),
    }
}

fn require_ics_source(source: TemporalSource, selector: &str) -> anyhow::Result<TemporalSource> {
    if source.kind == SourceKind::Ics {
        Ok(source)
    } else {
        Err(anyhow!(
            "source {selector:?} is {}, not an ICS source",
            source.kind.as_str()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ephemeris::ics::import_ics_text;

    const FIXTURE: &str = concat!(
        "BEGIN:VCALENDAR\r\n",
        "PRODID:-//Ephemeris Export CLI Test//EN\r\n",
        "VERSION:2.0\r\n",
        "X-WR-CALNAME:CLI Fixture\r\n",
        "BEGIN:VEVENT\r\n",
        "UID:cli@example.com\r\n",
        "DTSTAMP:20261006T120000Z\r\n",
        "DTSTART:20261007T090000Z\r\n",
        "SUMMARY:CLI event\r\n",
        "END:VEVENT\r\n",
        "END:VCALENDAR\r\n"
    );

    #[test]
    fn source_resolution_accepts_id_external_ref_and_exact_name() {
        let store = TemporalStore::open_in_memory().expect("store");
        let imported =
            import_ics_text(&store, FIXTURE, "ics:test:cli", None).expect("import fixture");

        let by_id =
            resolve_ics_source(&store, &imported.source_id.to_string()).expect("source by id");
        let by_ref = resolve_ics_source(&store, "ics:test:cli").expect("source by ref");
        let by_name = resolve_ics_source(&store, "CLI Fixture").expect("source by name");

        assert_eq!(by_id.id, imported.source_id);
        assert_eq!(by_ref.id, imported.source_id);
        assert_eq!(by_name.id, imported.source_id);
    }

    #[test]
    fn source_resolution_rejects_non_ics_source() {
        let store = TemporalStore::open_in_memory().expect("store");
        let mut source = TemporalSource::new(
            "Manual source",
            SourceKind::Manual,
            ephemeris::domain::SourceAuthority::Manual,
        );
        source.external_ref = Some("manual:test".to_string());
        store.upsert_source(&source).expect("save source");

        let error =
            resolve_ics_source(&store, "manual:test").expect_err("non-ICS source must fail");
        assert!(error.to_string().contains("not an ICS source"));
    }

    #[test]
    fn source_resolution_rejects_ambiguous_exact_names() {
        let store = TemporalStore::open_in_memory().expect("store");
        import_ics_text(&store, FIXTURE, "ics:test:first", None).expect("first fixture");
        import_ics_text(&store, FIXTURE, "ics:test:second", None).expect("second fixture");

        let error =
            resolve_ics_source(&store, "CLI Fixture").expect_err("duplicate name must fail");
        assert!(error.to_string().contains("multiple imported ICS sources"));
    }
}
