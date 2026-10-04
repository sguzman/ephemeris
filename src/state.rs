use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::calendar::CalendarView;
use crate::domain::EventStatus;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedUiState {
    pub version: u32,
    pub calendar_view: CalendarView,
    pub focus_date: String,
    pub display_timezone: String,
    pub week_start_monday: bool,
    pub show_sources: bool,
    pub show_inspector: bool,
    #[serde(default)]
    pub hidden_source_ids: BTreeSet<Uuid>,
    #[serde(default)]
    pub search_query: String,
    #[serde(default)]
    pub domain_filter: Option<String>,
    #[serde(default)]
    pub jurisdiction_filter: Option<String>,
    #[serde(default)]
    pub status_filter: Option<EventStatus>,
    pub selected_event_id: Option<Uuid>,
}

impl Default for PersistedUiState {
    fn default() -> Self {
        Self {
            version: 1,
            calendar_view: CalendarView::Month,
            focus_date: Local::now().date_naive().to_string(),
            display_timezone: "America/Mexico_City".to_string(),
            week_start_monday: false,
            show_sources: true,
            show_inspector: true,
            hidden_source_ids: BTreeSet::new(),
            search_query: String::new(),
            domain_filter: None,
            jurisdiction_filter: None,
            status_filter: None,
            selected_event_id: None,
        }
    }
}

impl PersistedUiState {
    pub fn load() -> anyhow::Result<Self> {
        Self::load_from_path(&default_state_path()?)
    }

    pub fn load_or_default() -> Self {
        Self::load().unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to_path(&default_state_path()?)
    }

    pub fn load_from_path(path: &Path) -> anyhow::Result<Self> {
        if !path.is_file() {
            return Ok(Self::default());
        }

        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let state = serde_json::from_str(&raw)
            .with_context(|| format!("failed to decode {}", path.display()))?;
        Ok(state)
    }

    pub fn save_to_path(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let raw = serde_json::to_string_pretty(self).context("failed to encode UI state")?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, raw)
            .with_context(|| format!("failed to write {}", temporary.display()))?;
        std::fs::rename(&temporary, path)
            .with_context(|| format!("failed to replace {}", path.display()))?;
        Ok(())
    }

    pub fn focus_date(&self) -> NaiveDate {
        NaiveDate::parse_from_str(&self.focus_date, "%Y-%m-%d")
            .unwrap_or_else(|_| Local::now().date_naive())
    }

    pub fn set_focus_date(&mut self, value: NaiveDate) {
        self.focus_date = value.format("%Y-%m-%d").to_string();
    }
}

fn default_state_path() -> anyhow::Result<PathBuf> {
    if let Some(path) = dirs::data_local_dir() {
        return Ok(path.join("ephemeris").join("ui-state.json"));
    }

    Ok(std::env::current_dir()
        .context("failed to resolve current directory")?
        .join(".ephemeris")
        .join("ui-state.json"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use tempfile::tempdir;

    #[test]
    fn state_roundtrips() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("ui-state.json");
        let state = PersistedUiState {
            calendar_view: CalendarView::Week,
            display_timezone: "UTC".to_string(),
            ..PersistedUiState::default()
        };

        state.save_to_path(&path).expect("save");
        let loaded = PersistedUiState::load_from_path(&path).expect("load");

        assert_eq!(loaded.calendar_view, CalendarView::Week);
        assert_eq!(loaded.display_timezone, "UTC");
    }
}
