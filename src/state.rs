use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::calendar::{CalendarLayout, CalendarView};
use crate::domain::EventStatus;
use crate::query::{
    ColorBy, ColorRule, CompositionLayer, EventQuery, GroupBy, Overlay, QueryExpr, SavedView,
    SortRule, TableColumn, default_table_columns,
};

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
    pub event_type_filter: Option<String>,
    #[serde(default)]
    pub institution_filter: Option<String>,
    #[serde(default)]
    pub renderability_filter: Option<String>,
    #[serde(default)]
    pub tag_filter: Option<String>,
    #[serde(default)]
    pub status_filter: Option<EventStatus>,
    #[serde(default)]
    pub query_expression: Option<QueryExpr>,
    #[serde(default)]
    pub group_by: GroupBy,
    #[serde(default)]
    pub sort_rules: Vec<SortRule>,
    #[serde(default)]
    pub color_by: ColorBy,
    #[serde(default)]
    pub color_rules: Vec<ColorRule>,
    #[serde(default)]
    pub composition_layers: Vec<CompositionLayer>,
    #[serde(default)]
    pub overlays: Vec<Overlay>,
    #[serde(default = "default_table_columns")]
    pub table_columns: Vec<TableColumn>,
    #[serde(default, rename = "saved_views", skip_serializing_if = "Vec::is_empty")]
    pub legacy_saved_views: Vec<SavedView>,
    #[serde(default)]
    pub active_saved_view_id: Option<Uuid>,
    pub selected_event_id: Option<Uuid>,
    #[serde(default)]
    pub taria_resourcearium_root: String,
    #[serde(default = "default_taria_channel")]
    pub taria_channel: String,
    #[serde(default)]
    pub taria_last_release_id: Option<String>,
    #[serde(default)]
    pub taria_last_update_at: Option<String>,
    #[serde(default)]
    pub taria_last_update_summary: Option<String>,
}

impl Default for PersistedUiState {
    fn default() -> Self {
        Self {
            version: 10,
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
            event_type_filter: None,
            institution_filter: None,
            renderability_filter: None,
            tag_filter: None,
            status_filter: None,
            query_expression: None,
            group_by: GroupBy::Date,
            sort_rules: vec![SortRule::default()],
            color_by: ColorBy::Status,
            color_rules: Vec::new(),
            composition_layers: Vec::new(),
            overlays: Vec::new(),
            table_columns: default_table_columns(),
            legacy_saved_views: Vec::new(),
            active_saved_view_id: None,
            selected_event_id: None,
            taria_resourcearium_root: String::new(),
            taria_channel: default_taria_channel(),
            taria_last_release_id: None,
            taria_last_update_at: None,
            taria_last_update_summary: None,
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
        if state.version < 10 {
            state.version = 10;
        }
        if state.taria_channel.trim().is_empty() {
            state.taria_channel = default_taria_channel();
        }
        if state.sort_rules.is_empty() {
            state.sort_rules.push(SortRule::default());
        }
        if state.table_columns.is_empty() {
            state.table_columns = default_table_columns();
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
            event_type: self.event_type_filter.clone(),
            institution: self.institution_filter.clone(),
            renderability: self.renderability_filter.clone(),
            tag: self.tag_filter.clone(),
            status: self.status_filter,
            expression: self.query_expression.clone(),
        }
    }

    pub fn set_event_query(&mut self, query: &EventQuery) {
        self.search_query.clone_from(&query.text);
        self.domain_filter.clone_from(&query.domain);
        self.jurisdiction_filter.clone_from(&query.jurisdiction);
        self.event_type_filter.clone_from(&query.event_type);
        self.institution_filter.clone_from(&query.institution);
        self.renderability_filter.clone_from(&query.renderability);
        self.tag_filter.clone_from(&query.tag);
        self.status_filter = query.status;
        self.query_expression.clone_from(&query.expression);
    }

    pub fn clear_query(&mut self) {
        self.search_query.clear();
        self.domain_filter = None;
        self.jurisdiction_filter = None;
        self.event_type_filter = None;
        self.institution_filter = None;
        self.renderability_filter = None;
        self.tag_filter = None;
        self.status_filter = None;
        self.query_expression = None;
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
            color_rules: self.color_rules.clone(),
            composition_layers: self.composition_layers.clone(),
            overlays: self.overlays.clone(),
            table_columns: self.table_columns.clone(),
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
        self.color_rules.clone_from(&view.color_rules);
        self.composition_layers.clone_from(&view.composition_layers);
        self.overlays.clone_from(&view.overlays);
        self.table_columns.clone_from(&view.table_columns);
        if self.table_columns.is_empty() {
            self.table_columns = default_table_columns();
        }
        self.display_timezone.clone_from(&view.display_timezone);
        self.week_start_monday = view.week_start_monday;
        self.active_saved_view_id = Some(view.id);
        self.selected_event_id = None;
    }
}

fn default_taria_channel() -> String {
    "bootstrap".to_string()
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
            event_type_filter: Some("election".to_string()),
            institution_filter: Some("California Secretary of State".to_string()),
            renderability_filter: Some("ready".to_string()),
            tag_filter: Some("general".to_string()),
            query_expression: Some(QueryExpr::Predicate(crate::query::QueryPredicate::Exists {
                field: crate::query::PresenceField::Institution,
                exists: true,
            })),
            group_by: GroupBy::Jurisdiction,
            color_by: ColorBy::EventType,
            table_columns: vec![TableColumn::Title, TableColumn::Date, TableColumn::Status],
            ..PersistedUiState::default()
        };

        state.save_to_path(&path).expect("save");
        let loaded = PersistedUiState::load_from_path(&path).expect("load");

        assert_eq!(loaded.calendar_view, CalendarView::Week);
        assert_eq!(loaded.calendar_layout, CalendarLayout::Agenda);
        assert_eq!(loaded.display_timezone, "UTC");
        assert_eq!(loaded.domain_filter.as_deref(), Some("elections"));
        assert_eq!(loaded.event_type_filter.as_deref(), Some("election"));
        assert_eq!(
            loaded.institution_filter.as_deref(),
            Some("California Secretary of State")
        );
        assert_eq!(loaded.renderability_filter.as_deref(), Some("ready"));
        assert_eq!(loaded.tag_filter.as_deref(), Some("general"));
        assert_eq!(loaded.group_by, GroupBy::Jurisdiction);
        assert_eq!(loaded.color_by, ColorBy::EventType);
        assert_eq!(
            loaded.table_columns,
            vec![TableColumn::Title, TableColumn::Date, TableColumn::Status,]
        );
        assert!(loaded.color_rules.is_empty());
        assert!(loaded.overlays.is_empty());
        assert!(loaded.query_expression.is_some());
        assert!(loaded.legacy_saved_views.is_empty());
        assert_eq!(loaded.taria_channel, "bootstrap");
        assert!(loaded.taria_resourcearium_root.is_empty());
    }

    #[test]
    fn stream_layout_roundtrips_in_ui_state() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("ui-state.json");
        let state = PersistedUiState {
            calendar_layout: CalendarLayout::Stream,
            ..PersistedUiState::default()
        };

        state.save_to_path(&path).expect("save");
        let loaded = PersistedUiState::load_from_path(&path).expect("load");

        assert_eq!(loaded.calendar_layout, CalendarLayout::Stream);
    }

    #[test]
    fn compact_agenda_layout_roundtrips_in_ui_state() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("ui-state.json");
        let state = PersistedUiState {
            calendar_layout: CalendarLayout::CompactAgenda,
            ..PersistedUiState::default()
        };

        state.save_to_path(&path).expect("save");
        let loaded = PersistedUiState::load_from_path(&path).expect("load");

        assert_eq!(loaded.calendar_layout, CalendarLayout::CompactAgenda);
    }

    #[test]
    fn density_layout_roundtrips_in_ui_state() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("ui-state.json");
        let state = PersistedUiState {
            calendar_layout: CalendarLayout::Density,
            ..PersistedUiState::default()
        };

        state.save_to_path(&path).expect("save");
        let loaded = PersistedUiState::load_from_path(&path).expect("load");

        assert_eq!(loaded.calendar_layout, CalendarLayout::Density);
    }

    #[test]
    fn timeline_layout_roundtrips_in_ui_state() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("ui-state.json");
        let state = PersistedUiState {
            calendar_layout: CalendarLayout::Timeline,
            ..PersistedUiState::default()
        };

        state.save_to_path(&path).expect("save");
        let loaded = PersistedUiState::load_from_path(&path).expect("load");

        assert_eq!(loaded.calendar_layout, CalendarLayout::Timeline);
    }

    #[test]
    fn applying_saved_view_restores_query_and_presentation() {
        let mut state = PersistedUiState {
            domain_filter: Some("elections".to_string()),
            event_type_filter: Some("election".to_string()),
            institution_filter: Some("California Secretary of State".to_string()),
            renderability_filter: Some("ready".to_string()),
            tag_filter: Some("general".to_string()),
            calendar_view: CalendarView::Year,
            calendar_layout: CalendarLayout::Agenda,
            query_expression: Some(QueryExpr::Predicate(crate::query::QueryPredicate::Exists {
                field: crate::query::PresenceField::Domain,
                exists: true,
            })),
            group_by: GroupBy::Domain,
            color_by: ColorBy::Jurisdiction,
            table_columns: vec![TableColumn::Title, TableColumn::Institution],
            ..PersistedUiState::default()
        };
        let view = state.capture_saved_view("Elections");

        state.clear_query();
        state.calendar_view = CalendarView::Day;
        state.apply_saved_view(&view);

        assert_eq!(state.domain_filter.as_deref(), Some("elections"));
        assert_eq!(state.event_type_filter.as_deref(), Some("election"));
        assert_eq!(
            state.institution_filter.as_deref(),
            Some("California Secretary of State")
        );
        assert_eq!(state.renderability_filter.as_deref(), Some("ready"));
        assert_eq!(state.tag_filter.as_deref(), Some("general"));
        assert_eq!(state.calendar_view, CalendarView::Year);
        assert_eq!(state.calendar_layout, CalendarLayout::Agenda);
        assert_eq!(state.group_by, GroupBy::Domain);
        assert_eq!(state.color_by, ColorBy::Jurisdiction);
        assert_eq!(
            state.table_columns,
            vec![TableColumn::Title, TableColumn::Institution]
        );
        assert!(state.query_expression.is_some());
        assert_eq!(state.active_saved_view_id, Some(view.id));
    }
}
