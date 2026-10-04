use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::calendar::{CalendarLayout, CalendarView};
use crate::domain::EventStatus;
use crate::query::{ColorBy, EventQuery, GroupBy, SavedView, SortRule};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedUiState {
    pub version: u32,
    pub calendar_view: CalendarView,
    #[serde(default)]
    pub calendar_layout: CalendarLayout,
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
    #[serde(default)]
    pub group_by: GroupBy,
    #[serde(default)]
    pub sort_rules: Vec<SortRule>,
    #[serde(default)]
    pub color_by: ColorBy,
    #[serde(default, rename = "saved_views", skip_serializing_if = "Vec::is_empty")]
    pub legacy_saved_views: Vec<SavedView>,
    #[serde(default)]
    pub active_saved_view_id: Option<Uuid>,
    pub selected_event_id: Option<Uuid>,
}

impl Default for PersistedUiState {
    fn default() -> Self {
        Self {
            version: 4,
            calendar_view: CalendarView::Month,
            calendar_layout: CalendarLayout::Grid,
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
            group_by: GroupBy::Date,
            sort_rules: vec![SortRule::default()],
            color_by: ColorBy::Status,
            legacy_saved_views: Vec::new(),
            active_saved_view_id: None,
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
        let mut state: Self = serde_json::from_str(&raw)
            .with_context(|| format!("failed to decode {}", path.display()))?;
        if state.version < 4 {
            state.version = 4;
        }
        if state.sort_rules.is_empty() {
            state.sort_rules.push(SortRule::default());
        }
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

    pub fn event_query(&self) -> EventQuery {
        EventQuery {
            text: self.search_query.clone(),
            domain: self.domain_filter.clone(),
            jurisdiction: self.jurisdiction_filter.clone(),
            status: self.status_filter,
        }
    }

    pub fn set_event_query(&mut self, query: &EventQuery) {
        self.search_query.clone_from(&query.text);
        self.domain_filter.clone_from(&query.domain);
        self.jurisdiction_filter.clone_from(&query.jurisdiction);
        self.status_filter = query.status;
    }

    pub fn clear_query(&mut self) {
        self.search_query.clear();
        self.domain_filter = None;
        self.jurisdiction_filter = None;
        self.status_filter = None;
        self.active_saved_view_id = None;
    }

    pub fn capture_saved_view(&self, name: impl Into<String>) -> SavedView {
        SavedView {
            id: Uuid::new_v4(),
            name: name.into(),
            query: self.event_query(),
            hidden_source_ids: self.hidden_source_ids.clone(),
            calendar_view: self.calendar_view,
            calendar_layout: self.calendar_layout,
            group_by: self.group_by,
            sort_rules: self.sort_rules.clone(),
            color_by: self.color_by,
            display_timezone: self.display_timezone.clone(),
            week_start_monday: self.week_start_monday,
        }
    }

    pub fn apply_saved_view(&mut self, view: &SavedView) {
        self.set_event_query(&view.query);
        self.hidden_source_ids.clone_from(&view.hidden_source_ids);
        self.calendar_view = view.calendar_view;
        self.calendar_layout = view.calendar_layout;
        self.group_by = view.group_by;
        self.sort_rules.clone_from(&view.sort_rules);
        if self.sort_rules.is_empty() {
            self.sort_rules.push(SortRule::default());
        }
        self.color_by = view.color_by;
        self.display_timezone.clone_from(&view.display_timezone);
        self.week_start_monday = view.week_start_monday;
        self.active_saved_view_id = Some(view.id);
        self.selected_event_id = None;
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
            calendar_layout: CalendarLayout::Agenda,
            display_timezone: "UTC".to_string(),
            domain_filter: Some("elections".to_string()),
            group_by: GroupBy::Jurisdiction,
            color_by: ColorBy::EventType,
            ..PersistedUiState::default()
        };

        state.save_to_path(&path).expect("save");
        let loaded = PersistedUiState::load_from_path(&path).expect("load");

        assert_eq!(loaded.calendar_view, CalendarView::Week);
        assert_eq!(loaded.calendar_layout, CalendarLayout::Agenda);
        assert_eq!(loaded.display_timezone, "UTC");
        assert_eq!(loaded.domain_filter.as_deref(), Some("elections"));
        assert_eq!(loaded.group_by, GroupBy::Jurisdiction);
        assert_eq!(loaded.color_by, ColorBy::EventType);
        assert!(loaded.legacy_saved_views.is_empty());
    }

    #[test]
    fn applying_saved_view_restores_query_and_presentation() {
        let mut state = PersistedUiState {
            domain_filter: Some("elections".to_string()),
            calendar_view: CalendarView::Year,
            calendar_layout: CalendarLayout::Agenda,
            group_by: GroupBy::Domain,
            color_by: ColorBy::Jurisdiction,
            ..PersistedUiState::default()
        };
        let view = state.capture_saved_view("Elections");

        state.clear_query();
        state.calendar_view = CalendarView::Day;
        state.apply_saved_view(&view);

        assert_eq!(state.domain_filter.as_deref(), Some("elections"));
        assert_eq!(state.calendar_view, CalendarView::Year);
        assert_eq!(state.calendar_layout, CalendarLayout::Agenda);
        assert_eq!(state.group_by, GroupBy::Domain);
        assert_eq!(state.color_by, ColorBy::Jurisdiction);
        assert_eq!(state.active_saved_view_id, Some(view.id));
    }
}
