use std::collections::BTreeSet;

use chrono::{Datelike, Local, NaiveDate};
use chrono_tz::Tz;
use eframe::egui::{self, Color32, RichText};
use uuid::Uuid;

use crate::calendar::{
    CalendarLayout, CalendarView, calendar_title, month_days, month_grid_start, quarter_months,
    shift_focus, week_days, window_for_view, year_months,
};
use crate::domain::{EventStatus, TemporalEvent, TemporalSource, TimeSpec};
use crate::query::{ColorBy, GroupBy, SavedView, SortDirection, SortField, SortRule};
use crate::state::PersistedUiState;
use crate::store::TemporalStore;
use crate::taria::import_reconciled_event_set_file;

pub struct EphemerisApp {
    store: TemporalStore,
    state: PersistedUiState,
    events: Vec<TemporalEvent>,
    unplaced_events: Vec<TemporalEvent>,
    sources: Vec<TemporalSource>,
    last_message: Option<String>,
    last_error: Option<String>,
    dirty_state: bool,
    saved_view_name: String,
    saved_views: Vec<SavedView>,
}

impl EphemerisApp {
    pub fn open() -> anyhow::Result<Self> {
        let store = TemporalStore::open_default()?;
        let mut state = PersistedUiState::load_or_default();

        if !state.legacy_saved_views.is_empty() {
            for view in &state.legacy_saved_views {
                store.upsert_saved_view(view)?;
            }
            state.legacy_saved_views.clear();
            state.save()?;
        }

        let saved_views = store.list_saved_views()?;
        let mut app = Self {
            store,
            state,
            events: Vec::new(),
            unplaced_events: Vec::new(),
            sources: Vec::new(),
            last_message: None,
            last_error: None,
            dirty_state: false,
            saved_view_name: String::new(),
            saved_views,
        };
        app.reload()?;
        Ok(app)
    }

    fn import_taria_path(&mut self, path: &std::path::Path) {
        match import_reconciled_event_set_file(&self.store, path) {
            Ok(report) => {
                if let Some(focus) = report.suggested_focus {
                    self.state.set_focus_date(focus);
                    self.state.calendar_view = CalendarView::Month;
                    self.state.selected_event_id = None;
                    self.mark_state_dirty();
                }

                self.last_message = Some(format!(
                    "Imported {}: {} created, {} updated, {} unchanged, {} retained missing; {} imprecise, {} blocked/unplaced",
                    report.projection_ref,
                    report.created,
                    report.updated,
                    report.unchanged,
                    report.retained_missing,
                    report.imprecise,
                    report.blocked_or_undated
                ));
                self.last_error = None;
                self.reload_or_report();
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to import {}: {error:#}", path.display()));
            }
        }
    }

    fn timezone(&self) -> Tz {
        self.state
            .display_timezone
            .parse::<Tz>()
            .unwrap_or(chrono_tz::America::Mexico_City)
    }

    fn reload(&mut self) -> anyhow::Result<()> {
        let timezone = self.timezone();
        let focus = self.state.focus_date();
        let window = window_for_view(
            self.state.calendar_view,
            focus,
            self.state.week_start_monday,
        );
        let include_imprecise = matches!(
            self.state.calendar_view,
            CalendarView::Year | CalendarView::Quarter | CalendarView::Month
        );
        self.events = self.store.events_in_window(
            window.start,
            window.end_exclusive,
            timezone,
            include_imprecise,
        )?;
        if self.state.calendar_view != CalendarView::Year {
            self.events
                .retain(|event| !matches!(event.time, TimeSpec::Year { .. }));
        }
        self.unplaced_events = self.store.unplaced_events()?;
        self.sources = self.store.list_sources()?;
        Ok(())
    }

    fn reload_or_report(&mut self) {
        match self.reload() {
            Ok(()) => self.last_error = None,
            Err(error) => self.last_error = Some(format!("{error:#}")),
        }
    }

    fn mark_state_dirty(&mut self) {
        self.dirty_state = true;
    }

    fn persist_state(&mut self) {
        if !self.dirty_state {
            return;
        }
        match self.state.save() {
            Ok(()) => self.dirty_state = false,
            Err(error) => self.last_error = Some(format!("{error:#}")),
        }
    }

    fn navigate(&mut self, amount: i32) {
        let next = shift_focus(self.state.calendar_view, self.state.focus_date(), amount);
        self.state.set_focus_date(next);
        self.state.selected_event_id = None;
        self.mark_state_dirty();
        self.reload_or_report();
    }

    fn go_today(&mut self) {
        self.state.set_focus_date(Local::now().date_naive());
        self.state.selected_event_id = None;
        self.mark_state_dirty();
        self.reload_or_report();
    }

    fn set_view(&mut self, view: CalendarView) {
        if self.state.calendar_view == view {
            return;
        }
        self.state.calendar_view = view;
        self.state.active_saved_view_id = None;
        self.state.selected_event_id = None;
        self.mark_state_dirty();
        self.reload_or_report();
    }

    fn set_layout(&mut self, layout: CalendarLayout) {
        if self.state.calendar_layout == layout {
            return;
        }
        self.state.calendar_layout = layout;
        self.state.active_saved_view_id = None;
        self.state.selected_event_id = None;
        self.mark_state_dirty();
    }

    fn save_current_view(&mut self) {
        let name = self.saved_view_name.trim();
        if name.is_empty() {
            return;
        }

        let view = self.state.capture_saved_view(name);
        match self.store.upsert_saved_view(&view) {
            Ok(()) => {
                self.state.active_saved_view_id = Some(view.id);
                self.saved_views.push(view);
                self.saved_views
                    .sort_by_key(|saved| saved.name.to_ascii_lowercase());
                self.saved_view_name.clear();
                self.last_message = Some("Saved programmable calendar view.".to_string());
                self.last_error = None;
                self.mark_state_dirty();
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to save view: {error:#}"));
            }
        }
    }

    fn apply_saved_view(&mut self, id: Uuid) {
        let Some(view) = self.saved_views.iter().find(|view| view.id == id).cloned() else {
            return;
        };

        self.state.apply_saved_view(&view);
        self.mark_state_dirty();
        self.reload_or_report();
    }

    fn update_active_saved_view(&mut self) {
        let Some(id) = self.state.active_saved_view_id else {
            return;
        };
        let Some(index) = self.saved_views.iter().position(|view| view.id == id) else {
            self.state.active_saved_view_id = None;
            self.mark_state_dirty();
            return;
        };

        let name = self.saved_views[index].name.clone();
        let mut replacement = self.state.capture_saved_view(name);
        replacement.id = id;

        match self.store.upsert_saved_view(&replacement) {
            Ok(()) => {
                self.saved_views[index] = replacement;
                self.last_message =
                    Some("Updated saved view from current query and presentation.".to_string());
                self.last_error = None;
                self.mark_state_dirty();
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to update saved view: {error:#}"));
            }
        }
    }

    fn delete_saved_view(&mut self, id: Uuid) {
        match self.store.delete_saved_view(id) {
            Ok(()) => {
                self.saved_views.retain(|view| view.id != id);
                if self.state.active_saved_view_id == Some(id) {
                    self.state.active_saved_view_id = None;
                }
                self.mark_state_dirty();
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to delete saved view: {error:#}"));
            }
        }
    }

    fn visible_events(&self) -> Vec<TemporalEvent> {
        let query = self.state.event_query();
        let mut events = self
            .events
            .iter()
            .filter(|event| {
                event
                    .source_id
                    .is_none_or(|source_id| !self.state.hidden_source_ids.contains(&source_id))
                    && query.matches(event)
            })
            .cloned()
            .collect::<Vec<_>>();
        sort_events(&mut events, self.timezone(), &self.state.sort_rules);
        events
    }

    fn visible_unplaced_events(&self) -> Vec<TemporalEvent> {
        let query = self.state.event_query();
        let mut events = self
            .unplaced_events
            .iter()
            .filter(|event| {
                event
                    .source_id
                    .is_none_or(|source_id| !self.state.hidden_source_ids.contains(&source_id))
                    && query.matches(event)
            })
            .cloned()
            .collect::<Vec<_>>();
        sort_events(&mut events, self.timezone(), &self.state.sort_rules);
        events
    }

    fn handle_shortcuts(&mut self, ui: &egui::Ui) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }

        let mut navigate = 0;
        let mut target_view = None;
        let mut target_layout = None;
        let mut today = false;

        ui.input(|input| {
            if input.key_pressed(egui::Key::ArrowLeft) {
                navigate = -1;
            }
            if input.key_pressed(egui::Key::ArrowRight) {
                navigate = 1;
            }
            if input.key_pressed(egui::Key::T) {
                today = true;
            }
            if input.key_pressed(egui::Key::Y) {
                target_view = Some(CalendarView::Year);
            }
            if input.key_pressed(egui::Key::Q) {
                target_view = Some(CalendarView::Quarter);
            }
            if input.key_pressed(egui::Key::M) {
                target_view = Some(CalendarView::Month);
            }
            if input.key_pressed(egui::Key::W) {
                target_view = Some(CalendarView::Week);
            }
            if input.key_pressed(egui::Key::D) {
                target_view = Some(CalendarView::Day);
            }
            if input.key_pressed(egui::Key::G) {
                target_layout = Some(CalendarLayout::Grid);
            }
            if input.key_pressed(egui::Key::A) {
                target_layout = Some(CalendarLayout::Agenda);
            }
        });

        if navigate != 0 {
            self.navigate(navigate);
        }
        if today {
            self.go_today();
        }
        if let Some(view) = target_view {
            self.set_view(view);
        }
        if let Some(layout) = target_layout {
            self.set_layout(layout);
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let title = calendar_title(
            self.state.calendar_view,
            self.state.focus_date(),
            self.state.week_start_monday,
        );

        ui.horizontal_wrapped(|ui| {
            ui.heading(RichText::new("Ephemeris").strong());
            ui.separator();
            ui.heading(title);
            ui.separator();

            if ui.button("Prev").clicked() {
                self.navigate(-1);
            }
            if ui.button("Today").clicked() {
                self.go_today();
            }
            if ui.button("Next").clicked() {
                self.navigate(1);
            }

            ui.separator();

            for view in CalendarView::ALL {
                let selected = self.state.calendar_view == view;
                if ui
                    .add(egui::Button::new(view.label()).selected(selected))
                    .clicked()
                {
                    self.set_view(view);
                }
            }

            ui.separator();

            for layout in CalendarLayout::ALL {
                let selected = self.state.calendar_layout == layout;
                if ui
                    .add(egui::Button::new(layout.label()).selected(selected))
                    .clicked()
                {
                    self.set_layout(layout);
                }
            }

            ui.separator();

            if ui
                .add(egui::Button::new("Sources").selected(self.state.show_sources))
                .clicked()
            {
                self.state.show_sources = !self.state.show_sources;
                self.mark_state_dirty();
            }

            if ui
                .add(egui::Button::new("Inspector").selected(self.state.show_inspector))
                .clicked()
            {
                self.state.show_inspector = !self.state.show_inspector;
                self.mark_state_dirty();
            }

            ui.separator();
            ui.small(format!(
                "{} events · {} unplaced/conflicted · {} sources · {}",
                self.visible_events().len(),
                self.visible_unplaced_events().len(),
                self.sources.len(),
                self.state.display_timezone
            ));
            ui.separator();
            ui.small("Drop Taria reconciled JSON anywhere to import");
        });

        if let Some(message) = self.last_message.as_deref() {
            ui.colored_label(Color32::LIGHT_GREEN, message);
        }
        if let Some(error) = self.last_error.as_deref() {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
    }

    fn render_sources(&mut self, ui: &mut egui::Ui) {
        ui.set_width(280.0);

        ui.heading("Saved Views");
        ui.small("Named calendars are queries and presentation over one event corpus.");

        let saved_views = self.saved_views.clone();
        let mut apply_view = None;
        let mut delete_view = None;
        for view in saved_views {
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(self.state.active_saved_view_id == Some(view.id), &view.name)
                    .clicked()
                {
                    apply_view = Some(view.id);
                }
                if ui
                    .small_button("×")
                    .on_hover_text("Delete saved view")
                    .clicked()
                {
                    delete_view = Some(view.id);
                }
            });
        }

        if let Some(id) = apply_view {
            self.apply_saved_view(id);
        }
        if let Some(id) = delete_view {
            self.delete_saved_view(id);
        }

        if self.state.active_saved_view_id.is_some() && ui.button("Update active view").clicked() {
            self.update_active_saved_view();
        }

        ui.horizontal(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.saved_view_name).hint_text("New view name"),
            );
            let submit =
                response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if (ui.button("Save").clicked() || submit) && !self.saved_view_name.trim().is_empty() {
                self.save_current_view();
            }
        });

        ui.separator();
        ui.heading("Query");
        ui.small("Filtering is independent from source organization.");

        let mut filters_changed = false;
        ui.horizontal(|ui| {
            ui.label("Search");
            filters_changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut self.state.search_query)
                        .hint_text("title, institution, tags…"),
                )
                .changed();
            if !self.state.search_query.is_empty() && ui.small_button("×").clicked() {
                self.state.search_query.clear();
                filters_changed = true;
            }
        });

        let domains = self
            .events
            .iter()
            .chain(self.unplaced_events.iter())
            .filter_map(|event| event.domain.clone())
            .collect::<BTreeSet<_>>();
        egui::ComboBox::from_id_salt("query.domain")
            .selected_text(self.state.domain_filter.as_deref().unwrap_or("All domains"))
            .show_ui(ui, |ui| {
                filters_changed |= ui
                    .selectable_value(&mut self.state.domain_filter, None, "All domains")
                    .changed();
                for domain in domains {
                    filters_changed |= ui
                        .selectable_value(
                            &mut self.state.domain_filter,
                            Some(domain.clone()),
                            domain,
                        )
                        .changed();
                }
            });

        let jurisdictions = self
            .events
            .iter()
            .chain(self.unplaced_events.iter())
            .filter_map(|event| event.jurisdiction.clone())
            .collect::<BTreeSet<_>>();
        egui::ComboBox::from_id_salt("query.jurisdiction")
            .selected_text(
                self.state
                    .jurisdiction_filter
                    .as_deref()
                    .unwrap_or("All jurisdictions"),
            )
            .show_ui(ui, |ui| {
                filters_changed |= ui
                    .selectable_value(
                        &mut self.state.jurisdiction_filter,
                        None,
                        "All jurisdictions",
                    )
                    .changed();
                for jurisdiction in jurisdictions {
                    filters_changed |= ui
                        .selectable_value(
                            &mut self.state.jurisdiction_filter,
                            Some(jurisdiction.clone()),
                            jurisdiction,
                        )
                        .changed();
                }
            });

        egui::ComboBox::from_id_salt("query.status")
            .selected_text(
                self.state
                    .status_filter
                    .map(EventStatus::as_str)
                    .unwrap_or("All statuses"),
            )
            .show_ui(ui, |ui| {
                filters_changed |= ui
                    .selectable_value(&mut self.state.status_filter, None, "All statuses")
                    .changed();
                for status in EventStatus::ALL {
                    filters_changed |= ui
                        .selectable_value(
                            &mut self.state.status_filter,
                            Some(status),
                            status.as_str(),
                        )
                        .changed();
                }
            });

        if filters_changed {
            self.state.active_saved_view_id = None;
            self.state.selected_event_id = None;
            self.mark_state_dirty();
        }

        if (self.state.domain_filter.is_some()
            || self.state.jurisdiction_filter.is_some()
            || self.state.status_filter.is_some()
            || !self.state.search_query.is_empty())
            && ui.button("Clear query").clicked()
        {
            self.state.clear_query();
            self.state.selected_event_id = None;
            self.mark_state_dirty();
        }

        ui.separator();
        ui.heading("Presentation");
        ui.small("Grouping, sorting, and coloring are independent from filtering.");

        let mut presentation_changed = false;

        egui::ComboBox::from_id_salt("presentation.group")
            .selected_text(self.state.group_by.label())
            .show_ui(ui, |ui| {
                for group_by in GroupBy::ALL {
                    presentation_changed |= ui
                        .selectable_value(&mut self.state.group_by, group_by, group_by.label())
                        .changed();
                }
            });

        egui::ComboBox::from_id_salt("presentation.color")
            .selected_text(self.state.color_by.label())
            .show_ui(ui, |ui| {
                for color_by in ColorBy::ALL {
                    presentation_changed |= ui
                        .selectable_value(&mut self.state.color_by, color_by, color_by.label())
                        .changed();
                }
            });

        ui.strong("Sort rules");
        let mut remove_sort = None;
        let can_remove_sort = self.state.sort_rules.len() > 1;
        for (index, rule) in self.state.sort_rules.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt(("presentation.sort.field", index))
                    .selected_text(rule.field.label())
                    .show_ui(ui, |ui| {
                        for field in SortField::ALL {
                            presentation_changed |= ui
                                .selectable_value(&mut rule.field, field, field.label())
                                .changed();
                        }
                    });
                egui::ComboBox::from_id_salt(("presentation.sort.direction", index))
                    .selected_text(rule.direction.label())
                    .show_ui(ui, |ui| {
                        for direction in SortDirection::ALL {
                            presentation_changed |= ui
                                .selectable_value(
                                    &mut rule.direction,
                                    direction,
                                    direction.label(),
                                )
                                .changed();
                        }
                    });
                if can_remove_sort && ui.small_button("×").clicked() {
                    remove_sort = Some(index);
                }
            });
        }

        if let Some(index) = remove_sort {
            self.state.sort_rules.remove(index);
            presentation_changed = true;
        }
        if ui.button("Add sort key").clicked() {
            self.state.sort_rules.push(SortRule::default());
            presentation_changed = true;
        }

        if presentation_changed {
            self.state.active_saved_view_id = None;
            self.state.selected_event_id = None;
            self.mark_state_dirty();
        }

        ui.separator();
        ui.heading("Sources");
        ui.small("Source visibility is another independent dimension.");
        ui.separator();

        if self.sources.is_empty() {
            ui.label(RichText::new("No sources yet.").italics());
            ui.small("Taria and ICS ingestion are the next data-path milestones.");
            return;
        }

        let sources = self.sources.clone();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for source in sources {
                let mut visible = !self.state.hidden_source_ids.contains(&source.id);
                if ui.checkbox(&mut visible, &source.name).changed() {
                    if visible {
                        self.state.hidden_source_ids.remove(&source.id);
                    } else {
                        self.state.hidden_source_ids.insert(source.id);
                    }
                    self.state.active_saved_view_id = None;
                    self.mark_state_dirty();
                }
                ui.small(format!(
                    "{} · {}{}",
                    source.kind.as_str(),
                    source.authority.as_str(),
                    if source.read_only {
                        " · read-only"
                    } else {
                        ""
                    }
                ));
                ui.add_space(6.0);
            }

            let unplaced = self.visible_unplaced_events();
            if !unplaced.is_empty() {
                ui.separator();
                ui.strong(format!("Unplaced / conflicts ({})", unplaced.len()));
                ui.small(
                    "Retained temporal records that cannot honestly be assigned to a day grid.",
                );
                ui.add_space(4.0);
                for event in unplaced {
                    let label = match event.renderability.as_deref() {
                        Some(state) if state != "ready" => {
                            format!("{} · {}", event.normalized_title, state)
                        }
                        _ => event.normalized_title.clone(),
                    };
                    if ui
                        .selectable_label(
                            self.state.selected_event_id == Some(event.id),
                            RichText::new(label).color(status_color(event.status)),
                        )
                        .clicked()
                    {
                        self.state.selected_event_id = Some(event.id);
                        self.state.show_inspector = true;
                        self.mark_state_dirty();
                    }
                }
            }
        });
    }

    fn render_inspector(&self, ui: &mut egui::Ui, events: &[TemporalEvent]) {
        ui.set_width(320.0);
        ui.heading("Event Inspector");
        ui.separator();

        let Some(selected_id) = self.state.selected_event_id else {
            ui.label("Select an event to inspect its temporal structure.");
            return;
        };

        let event = events
            .iter()
            .find(|event| event.id == selected_id)
            .or_else(|| {
                self.unplaced_events
                    .iter()
                    .find(|event| event.id == selected_id)
            });
        let Some(event) = event else {
            ui.label("The selected event is not in the current view.");
            return;
        };

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading(&event.normalized_title);
            ui.label(
                RichText::new(event.status.as_str())
                    .color(status_color(event.status))
                    .strong(),
            );
            ui.add_space(8.0);

            inspector_row(ui, "Time kind", event.time.kind_name());
            inspector_row(ui, "Display", &event.display_time_label(self.timezone()));
            if let Some(raw) = event.raw_title.as_deref() {
                inspector_row(ui, "Raw title", raw);
            }
            if let Some(value) = event.event_type.as_deref() {
                inspector_row(ui, "Type", value);
            }
            if let Some(value) = event.domain.as_deref() {
                inspector_row(ui, "Domain", value);
            }
            if let Some(value) = event.jurisdiction.as_deref() {
                inspector_row(ui, "Jurisdiction", value);
            }
            if let Some(value) = event.institution.as_deref() {
                inspector_row(ui, "Institution", value);
            }
            if let Some(value) = event.confidence {
                inspector_row(ui, "Confidence", &format!("{value:.3}"));
            }
            if let Some(value) = event.importance {
                inspector_row(ui, "Importance", &value.to_string());
            }
            if let Some(value) = event.personal_relevance {
                inspector_row(ui, "Personal relevance", &value.to_string());
            }
            if let Some(source_id) = event.source_id {
                let source_name = self
                    .sources
                    .iter()
                    .find(|source| source.id == source_id)
                    .map(|source| source.name.as_str())
                    .unwrap_or("Unknown source");
                inspector_row(ui, "Source", source_name);
                inspector_row(ui, "Source ID", &source_id.to_string());
            }
            if let Some(key) = event.source_record_key.as_deref() {
                inspector_row(ui, "Source record", key);
            }
            if let Some(value) = event.upstream_event_ref.as_deref() {
                inspector_row(ui, "Taria event ref", value);
            }
            if let Some(value) = event.upstream_reconciled_key.as_deref() {
                inspector_row(ui, "Reconciled key", value);
            }
            if let Some(value) = event.renderability.as_deref() {
                inspector_row(ui, "Renderability", value);
            }
            if !event.assertion_refs.is_empty() {
                inspector_row(ui, "Assertions", &event.assertion_refs.join(", "));
            }
            if !event.source_refs.is_empty() {
                inspector_row(ui, "Upstream sources", &event.source_refs.join(", "));
            }
            if !event.provenance_refs.is_empty() {
                inspector_row(ui, "Provenance", &event.provenance_refs.join(", "));
            }

            render_time_spec(ui, &event.time, self.timezone());

            if !event.tags.is_empty() {
                ui.separator();
                ui.strong("Tags");
                ui.label(event.tags.join(", "));
            }

            if let Some(description) = event.description.as_deref()
                && !description.trim().is_empty()
            {
                ui.separator();
                ui.strong("Description");
                ui.label(description);
            }

            if !event.properties.is_null()
                && event
                    .properties
                    .as_object()
                    .is_some_and(|properties| !properties.is_empty())
            {
                ui.separator();
                ui.strong("Extensible properties");
                ui.monospace(
                    serde_json::to_string_pretty(&event.properties)
                        .unwrap_or_else(|_| "<invalid properties>".to_string()),
                );
            }

            ui.separator();
            inspector_row(ui, "Event ID", &event.id.to_string());
            inspector_row(ui, "Created", &event.created_at.to_rfc3339());
            inspector_row(ui, "Updated", &event.updated_at.to_rfc3339());
        });
    }

    fn apply_calendar_action(&mut self, action: CalendarAction) {
        match action {
            CalendarAction::Select(id) => {
                self.state.selected_event_id = Some(id);
                self.state.show_inspector = true;
                self.mark_state_dirty();
            }
            CalendarAction::OpenDay(day) => {
                self.state.set_focus_date(day);
                self.state.calendar_view = CalendarView::Day;
                self.state.selected_event_id = None;
                self.mark_state_dirty();
                self.reload_or_report();
            }
            CalendarAction::OpenMonth(month) => {
                self.state.set_focus_date(month);
                self.state.calendar_view = CalendarView::Month;
                self.state.selected_event_id = None;
                self.mark_state_dirty();
                self.reload_or_report();
            }
        }
    }
}

impl eframe::App for EphemerisApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_shortcuts(ui);

        let dropped_paths = ui.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .collect::<Vec<_>>()
        });
        for path in dropped_paths {
            self.import_taria_path(&path);
        }

        self.toolbar(ui);
        ui.separator();

        let events = self.visible_events();
        let timezone = self.timezone();
        let focus = self.state.focus_date();

        let mut action = None;

        ui.horizontal_top(|ui| {
            if self.state.show_sources {
                self.render_sources(ui);
                ui.separator();
            }

            ui.vertical(|ui| {
                ui.set_min_width(560.0);
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        action = render_calendar(
                            ui,
                            CalendarRenderContext {
                                events: &events,
                                view: self.state.calendar_view,
                                layout: self.state.calendar_layout,
                                focus,
                                timezone,
                                monday_start: self.state.week_start_monday,
                                selected: self.state.selected_event_id,
                                group_by: self.state.group_by,
                                sort_rules: &self.state.sort_rules,
                                color_by: self.state.color_by,
                            },
                        );
                    });
            });

            if self.state.show_inspector {
                ui.separator();
                self.render_inspector(ui, &events);
            }
        });

        if let Some(action) = action {
            self.apply_calendar_action(action);
        }

        self.persist_state();
    }
}

#[derive(Debug, Clone, Copy)]
enum CalendarAction {
    Select(Uuid),
    OpenDay(NaiveDate),
    OpenMonth(NaiveDate),
}

struct CalendarRenderContext<'a> {
    events: &'a [TemporalEvent],
    view: CalendarView,
    layout: CalendarLayout,
    focus: NaiveDate,
    timezone: Tz,
    monday_start: bool,
    selected: Option<Uuid>,
    group_by: GroupBy,
    sort_rules: &'a [SortRule],
    color_by: ColorBy,
}

fn render_calendar(
    ui: &mut egui::Ui,
    context: CalendarRenderContext<'_>,
) -> Option<CalendarAction> {
    let CalendarRenderContext {
        events,
        view,
        layout,
        focus,
        timezone,
        monday_start,
        selected,
        group_by,
        sort_rules,
        color_by,
    } = context;

    if events.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.heading("No events in this period");
            ui.label("The calendar is live; the corpus is simply empty.");
        });
    }

    if layout == CalendarLayout::Agenda {
        return render_agenda(
            ui,
            events,
            timezone,
            selected,
            group_by,
            sort_rules,
            color_by,
        );
    }

    match view {
        CalendarView::Year => render_year(ui, events, focus, timezone, selected),
        CalendarView::Quarter => render_quarter(ui, events, focus, timezone, selected),
        CalendarView::Month => render_month(ui, events, focus, timezone, monday_start, selected),
        CalendarView::Week => render_week(ui, events, focus, timezone, monday_start, selected),
        CalendarView::Day => render_day(ui, events, focus, timezone, selected),
    }
}

fn render_agenda(
    ui: &mut egui::Ui,
    events: &[TemporalEvent],
    timezone: Tz,
    selected: Option<Uuid>,
    group_by: GroupBy,
    sort_rules: &[SortRule],
    color_by: ColorBy,
) -> Option<CalendarAction> {
    let mut ordered = events.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| compare_events(left, right, timezone, sort_rules));

    let mut action = None;
    let mut previous_group: Option<String> = None;

    for event in ordered {
        let group = agenda_group_label(event, timezone, group_by);
        if group_by != GroupBy::None
            && previous_group.as_deref() != Some(group.as_str())
        {
            if previous_group.is_some() {
                ui.add_space(8.0);
            }
            ui.heading(&group);
            ui.separator();
            previous_group = Some(group);
        }

        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(event.display_time_label(timezone))
                    .monospace()
                    .color(Color32::GRAY),
            );
            if ui
                .selectable_label(
                    selected == Some(event.id),
                    RichText::new(&event.normalized_title)
                        .color(event_color(event, color_by))
                        .strong(),
                )
                .clicked()
            {
                action = Some(CalendarAction::Select(event.id));
            }

            if let Some(domain) = event.domain.as_deref() {
                ui.small(domain);
            }
            if let Some(jurisdiction) = event.jurisdiction.as_deref() {
                ui.small(format!("· {jurisdiction}"));
            }
            if let Some(institution) = event.institution.as_deref() {
                ui.small(format!("· {institution}"));
            }
        });
    }

    action
}

fn sort_events(events: &mut [TemporalEvent], timezone: Tz, sort_rules: &[SortRule]) {
    events.sort_by(|left, right| compare_events(left, right, timezone, sort_rules));
}

fn compare_events(
    left: &TemporalEvent,
    right: &TemporalEvent,
    timezone: Tz,
    sort_rules: &[SortRule],
) -> std::cmp::Ordering {
    let effective_rules = if sort_rules.is_empty() {
        std::slice::from_ref(&DEFAULT_SORT_RULE)
    } else {
        sort_rules
    };

    for rule in effective_rules {
        let ordering = compare_event_field(left, right, timezone, rule.field);
        let ordering = match rule.direction {
            SortDirection::Ascending => ordering,
            SortDirection::Descending => ordering.reverse(),
        };
        if ordering != std::cmp::Ordering::Equal {
            return ordering;
        }
    }

    left.normalized_title
        .cmp(&right.normalized_title)
        .then_with(|| left.id.cmp(&right.id))
}

const DEFAULT_SORT_RULE: SortRule = SortRule {
    field: SortField::Time,
    direction: SortDirection::Ascending,
};

fn compare_event_field(
    left: &TemporalEvent,
    right: &TemporalEvent,
    timezone: Tz,
    field: SortField,
) -> std::cmp::Ordering {
    match field {
        SortField::Time => agenda_sort_date(left, timezone)
            .cmp(&agenda_sort_date(right, timezone))
            .then_with(|| {
                left.display_time_label(timezone)
                    .cmp(&right.display_time_label(timezone))
            }),
        SortField::Title => left.normalized_title.cmp(&right.normalized_title),
        SortField::Importance => compare_optional(left.importance, right.importance),
        SortField::PersonalRelevance => {
            compare_optional(left.personal_relevance, right.personal_relevance)
        }
        SortField::Source => compare_optional(
            event_source_key(left).as_deref(),
            event_source_key(right).as_deref(),
        ),
        SortField::Domain => compare_optional(left.domain.as_deref(), right.domain.as_deref()),
        SortField::Jurisdiction => {
            compare_optional(left.jurisdiction.as_deref(), right.jurisdiction.as_deref())
        }
        SortField::Institution => {
            compare_optional(left.institution.as_deref(), right.institution.as_deref())
        }
        SortField::EventType => {
            compare_optional(left.event_type.as_deref(), right.event_type.as_deref())
        }
        SortField::Status => left.status.as_str().cmp(right.status.as_str()),
    }
}

fn compare_optional<T: Ord>(left: Option<T>, right: Option<T>) -> std::cmp::Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

fn event_source_key(event: &TemporalEvent) -> Option<String> {
    event
        .source_refs
        .first()
        .cloned()
        .or_else(|| event.source_id.map(|id| id.to_string()))
}

fn agenda_sort_date(event: &TemporalEvent, timezone: Tz) -> Option<NaiveDate> {
    match event.time {
        TimeSpec::Month { year, month } => NaiveDate::from_ymd_opt(year, month, 1),
        TimeSpec::Year { year } => NaiveDate::from_ymd_opt(year, 1, 1),
        TimeSpec::Unknown { .. } => None,
        _ => event.display_date(timezone),
    }
}

fn agenda_group_label(event: &TemporalEvent, timezone: Tz, group_by: GroupBy) -> String {
    match group_by {
        GroupBy::None => String::new(),
        GroupBy::Date => date_group_label(event, timezone),
        GroupBy::Week => agenda_sort_date(event, timezone)
            .map(|date| {
                let week = date.iso_week();
                format!("{} · week {:02}", week.year(), week.week())
            })
            .unwrap_or_else(|| "Unplaced / unresolved".to_string()),
        GroupBy::Month => agenda_sort_date(event, timezone)
            .map(|date| date.format("%B %Y").to_string())
            .unwrap_or_else(|| "Unplaced / unresolved".to_string()),
        GroupBy::Source => event_source_key(event)
            .unwrap_or_else(|| "No source".to_string()),
        GroupBy::Domain => event
            .domain
            .clone()
            .unwrap_or_else(|| "No domain".to_string()),
        GroupBy::Jurisdiction => event
            .jurisdiction
            .clone()
            .unwrap_or_else(|| "No jurisdiction".to_string()),
        GroupBy::Institution => event
            .institution
            .clone()
            .unwrap_or_else(|| "No institution".to_string()),
        GroupBy::EventType => event
            .event_type
            .clone()
            .unwrap_or_else(|| "No event type".to_string()),
        GroupBy::Status => event.status.as_str().to_string(),
    }
}

fn date_group_label(event: &TemporalEvent, timezone: Tz) -> String {
    match event.time {
        TimeSpec::Month { year, month } => NaiveDate::from_ymd_opt(year, month, 1)
            .map(|date| format!("{} · month precision", date.format("%B %Y")))
            .unwrap_or_else(|| format!("{year}-{month:02} · month precision")),
        TimeSpec::Year { year } => format!("{year} · year precision"),
        TimeSpec::Unknown { .. } => "Unplaced / unresolved".to_string(),
        _ => event
            .display_date(timezone)
            .map(|date| date.format("%A, %B %e, %Y").to_string())
            .unwrap_or_else(|| "Unplaced / unresolved".to_string()),
    }
}

fn event_color(event: &TemporalEvent, color_by: ColorBy) -> Color32 {
    match color_by {
        ColorBy::None => Color32::WHITE,
        ColorBy::Status => status_color(event.status),
        ColorBy::Source => category_color(event_source_key(event).as_deref()),
        ColorBy::Domain => category_color(event.domain.as_deref()),
        ColorBy::Jurisdiction => category_color(event.jurisdiction.as_deref()),
        ColorBy::Institution => category_color(event.institution.as_deref()),
        ColorBy::EventType => category_color(event.event_type.as_deref()),
    }
}

fn category_color(value: Option<&str>) -> Color32 {
    let Some(value) = value else {
        return Color32::GRAY;
    };

    const PALETTE: [Color32; 12] = [
        Color32::from_rgb(116, 185, 255),
        Color32::from_rgb(162, 155, 254),
        Color32::from_rgb(85, 239, 196),
        Color32::from_rgb(255, 234, 167),
        Color32::from_rgb(250, 177, 160),
        Color32::from_rgb(129, 236, 236),
        Color32::from_rgb(223, 230, 233),
        Color32::from_rgb(253, 121, 168),
        Color32::from_rgb(255, 118, 117),
        Color32::from_rgb(178, 190, 195),
        Color32::from_rgb(129, 236, 236),
        Color32::from_rgb(214, 162, 232),
    ];

    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    PALETTE[(hash as usize) % PALETTE.len()]
}

fn render_year(
    ui: &mut egui::Ui,
    events: &[TemporalEvent],
    focus: NaiveDate,
    timezone: Tz,
    selected: Option<Uuid>,
) -> Option<CalendarAction> {
    let months = year_months(focus);
    let mut action = None;

    let year_precision = events
        .iter()
        .filter(|event| matches!(event.time, TimeSpec::Year { year } if year == focus.year()))
        .collect::<Vec<_>>();
    if !year_precision.is_empty() {
        ui.group(|ui| {
            ui.strong("Year-precision events");
            ui.small("Taria knows the year, but not a month or day. No fake date is assigned.");
            for event in year_precision {
                if render_event_button(ui, event, timezone, selected).clicked() {
                    action = Some(CalendarAction::Select(event.id));
                }
            }
        });
        ui.add_space(8.0);
    }

    egui::Grid::new("year-grid")
        .num_columns(4)
        .spacing([12.0, 12.0])
        .show(ui, |ui| {
            for (index, month) in months.iter().enumerate() {
                ui.group(|ui| {
                    ui.set_min_width(180.0);
                    if ui.button(month.format("%B").to_string()).clicked() {
                        action = Some(CalendarAction::OpenMonth(*month));
                    }
                    let count = events
                        .iter()
                        .filter(|event| event.time.belongs_to_month(*month, timezone))
                        .count();
                    ui.label(format!("{count} events"));

                    for event in events.iter().filter(|event| {
                        matches!(
                            event.time,
                            TimeSpec::Month {
                                year,
                                month: event_month
                            } if year == month.year() && event_month == month.month()
                        )
                    }) {
                        if render_event_button(ui, event, timezone, selected).clicked() {
                            action = Some(CalendarAction::Select(event.id));
                        }
                    }

                    render_mini_month_counts(ui, events, *month, timezone);
                });
                if (index + 1) % 4 == 0 {
                    ui.end_row();
                }
            }
        });

    action
}

fn render_quarter(
    ui: &mut egui::Ui,
    events: &[TemporalEvent],
    focus: NaiveDate,
    timezone: Tz,
    selected: Option<Uuid>,
) -> Option<CalendarAction> {
    let months = quarter_months(focus);
    let mut action = None;

    ui.horizontal_top(|ui| {
        for month in months {
            ui.group(|ui| {
                ui.set_min_width(220.0);
                if ui.button(month.format("%B %Y").to_string()).clicked() {
                    action = Some(CalendarAction::OpenMonth(month));
                }
                let count = events
                    .iter()
                    .filter(|event| event.time.belongs_to_month(month, timezone))
                    .count();
                ui.label(format!("{count} events"));

                for event in events.iter().filter(|event| {
                    matches!(
                        event.time,
                        TimeSpec::Month {
                            year,
                            month: event_month
                        } if year == month.year() && event_month == month.month()
                    )
                }) {
                    if render_event_button(ui, event, timezone, selected).clicked() {
                        action = Some(CalendarAction::Select(event.id));
                    }
                }

                render_mini_month_counts(ui, events, month, timezone);
            });
        }
    });

    action
}

fn render_mini_month_counts(
    ui: &mut egui::Ui,
    events: &[TemporalEvent],
    month: NaiveDate,
    timezone: Tz,
) {
    let start = month_grid_start(month, false);
    let days = month_days(start);

    egui::Grid::new(format!("mini-{}-{}", month.year(), month.month()))
        .num_columns(7)
        .spacing([4.0, 2.0])
        .show(ui, |ui| {
            for (index, day) in days.iter().enumerate() {
                let count = events
                    .iter()
                    .filter(|event| event.time.occurs_on(*day, timezone))
                    .count();
                let text = if day.month() == month.month() {
                    if count == 0 {
                        day.day().to_string()
                    } else {
                        format!("{}·{}", day.day(), count)
                    }
                } else {
                    String::new()
                };
                ui.small(text);
                if (index + 1) % 7 == 0 {
                    ui.end_row();
                }
            }
        });
}

fn render_month(
    ui: &mut egui::Ui,
    events: &[TemporalEvent],
    focus: NaiveDate,
    timezone: Tz,
    monday_start: bool,
    selected: Option<Uuid>,
) -> Option<CalendarAction> {
    let start = month_grid_start(focus, monday_start);
    let days = month_days(start);
    let mut action = None;

    let month_precision = events
        .iter()
        .filter(|event| {
            matches!(
                event.time,
                TimeSpec::Month {
                    year,
                    month: event_month
                } if year == focus.year() && event_month == focus.month()
            )
        })
        .collect::<Vec<_>>();

    if !month_precision.is_empty() {
        ui.group(|ui| {
            ui.strong("Month-precision events");
            ui.small(
                "The source does not support a specific day, so these stay above the day grid.",
            );
            for event in month_precision {
                if render_event_button(ui, event, timezone, selected).clicked() {
                    action = Some(CalendarAction::Select(event.id));
                }
            }
        });
        ui.add_space(8.0);
    }

    let weekday_labels = if monday_start {
        ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
    } else {
        ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
    };

    egui::Grid::new("month-grid")
        .num_columns(7)
        .spacing([6.0, 6.0])
        .striped(true)
        .show(ui, |ui| {
            for label in weekday_labels {
                ui.strong(label);
            }
            ui.end_row();

            for (index, day) in days.iter().enumerate() {
                ui.group(|ui| {
                    ui.set_min_width(132.0);
                    ui.set_min_height(112.0);

                    let in_month = day.month() == focus.month();
                    let day_text = if in_month {
                        RichText::new(day.day().to_string()).strong()
                    } else {
                        RichText::new(day.day().to_string()).weak()
                    };

                    if ui.add(egui::Button::new(day_text).frame(false)).clicked() {
                        action = Some(CalendarAction::OpenDay(*day));
                    }

                    let day_events = events_for_day(events, *day, timezone);
                    for event in day_events.iter().take(4) {
                        if render_event_button(ui, event, timezone, selected).clicked() {
                            action = Some(CalendarAction::Select(event.id));
                        }
                    }
                    if day_events.len() > 4 {
                        ui.small(format!("+{} more", day_events.len() - 4));
                    }
                });

                if (index + 1) % 7 == 0 {
                    ui.end_row();
                }
            }
        });

    action
}

fn render_week(
    ui: &mut egui::Ui,
    events: &[TemporalEvent],
    focus: NaiveDate,
    timezone: Tz,
    monday_start: bool,
    selected: Option<Uuid>,
) -> Option<CalendarAction> {
    let days = week_days(focus, monday_start);
    let mut action = None;

    ui.horizontal_top(|ui| {
        for day in days {
            ui.group(|ui| {
                ui.set_min_width(170.0);
                if ui.button(day.format("%a %b %e").to_string()).clicked() {
                    action = Some(CalendarAction::OpenDay(day));
                }
                ui.separator();

                let day_events = events_for_day(events, day, timezone);
                if day_events.is_empty() {
                    ui.small("No events");
                }
                for event in day_events {
                    if render_event_button(ui, event, timezone, selected).clicked() {
                        action = Some(CalendarAction::Select(event.id));
                    }
                }
            });
        }
    });

    action
}

fn render_day(
    ui: &mut egui::Ui,
    events: &[TemporalEvent],
    focus: NaiveDate,
    timezone: Tz,
    selected: Option<Uuid>,
) -> Option<CalendarAction> {
    let mut action = None;
    let day_events = events_for_day(events, focus, timezone);

    if day_events.is_empty() {
        ui.label("No events on this day.");
        return None;
    }

    for event in day_events {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(event.display_time_label(timezone))
                        .monospace()
                        .color(Color32::GRAY),
                );
                let response = ui.selectable_label(
                    selected == Some(event.id),
                    RichText::new(&event.normalized_title)
                        .color(status_color(event.status))
                        .strong(),
                );
                if response.clicked() {
                    action = Some(CalendarAction::Select(event.id));
                }
            });

            let mut metadata = Vec::new();
            if let Some(domain) = event.domain.as_deref() {
                metadata.push(domain);
            }
            if let Some(jurisdiction) = event.jurisdiction.as_deref() {
                metadata.push(jurisdiction);
            }
            if let Some(institution) = event.institution.as_deref() {
                metadata.push(institution);
            }
            if !metadata.is_empty() {
                ui.small(metadata.join(" · "));
            }
        });
        ui.add_space(4.0);
    }

    action
}

fn render_event_button(
    ui: &mut egui::Ui,
    event: &TemporalEvent,
    timezone: Tz,
    selected: Option<Uuid>,
) -> egui::Response {
    let time = event.display_time_label(timezone);
    let label = format!("{time}  {}", event.normalized_title);
    ui.selectable_label(
        selected == Some(event.id),
        RichText::new(label).color(status_color(event.status)),
    )
}

fn events_for_day(events: &[TemporalEvent], day: NaiveDate, timezone: Tz) -> Vec<&TemporalEvent> {
    let mut out = events
        .iter()
        .filter(|event| event.time.occurs_on(day, timezone))
        .collect::<Vec<_>>();

    out.sort_by(|left, right| {
        left.display_time_label(timezone)
            .cmp(&right.display_time_label(timezone))
            .then_with(|| left.normalized_title.cmp(&right.normalized_title))
    });

    out
}

fn render_time_spec(ui: &mut egui::Ui, time: &TimeSpec, timezone: Tz) {
    ui.separator();
    ui.strong("Temporal representation");

    match time {
        TimeSpec::DateOnly {
            start,
            end_exclusive,
        } => {
            inspector_row(ui, "Civil-date start", &start.to_string());
            if let Some(end) = end_exclusive {
                inspector_row(ui, "End (exclusive)", &end.to_string());
            }
            ui.small("Source precision is date-only; no all-day semantics were invented.");
        }
        TimeSpec::AllDay {
            start,
            end_exclusive,
        } => {
            inspector_row(ui, "All-day start", &start.to_string());
            if let Some(end) = end_exclusive {
                inspector_row(ui, "End (exclusive)", &end.to_string());
            }
        }
        TimeSpec::Instant {
            start_utc,
            end_utc,
            source_timezone,
        } => {
            inspector_row(ui, "Start UTC", &start_utc.to_rfc3339());
            inspector_row(
                ui,
                "Start display",
                &start_utc.with_timezone(&timezone).to_rfc3339(),
            );
            if let Some(end) = end_utc {
                inspector_row(ui, "End UTC", &end.to_rfc3339());
            }
            if let Some(source_timezone) = source_timezone {
                inspector_row(ui, "Source timezone", source_timezone);
            }
        }
        TimeSpec::Floating {
            start,
            end,
            source_timezone,
        } => {
            inspector_row(ui, "Local start", &start.to_string());
            if let Some(end) = end {
                inspector_row(ui, "Local end", &end.to_string());
            }
            if let Some(source_timezone) = source_timezone {
                inspector_row(ui, "Source timezone", source_timezone);
            } else {
                ui.small("Floating local time: no timezone conversion is asserted.");
            }
        }
        TimeSpec::Month { year, month } => {
            inspector_row(ui, "Year", &year.to_string());
            inspector_row(ui, "Month", &month.to_string());
            ui.small("Month precision: Ephemeris does not invent a day.");
        }
        TimeSpec::Year { year } => {
            inspector_row(ui, "Year", &year.to_string());
            ui.small("Year precision: Ephemeris does not invent a month or day.");
        }
        TimeSpec::Unknown { original_value } => {
            if let Some(original_value) = original_value {
                inspector_row(ui, "Original value", original_value);
            }
            ui.small("This event is retained but cannot currently be placed on the calendar.");
        }
    }
}

fn inspector_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.strong(format!("{label}:"));
        ui.label(value);
    });
}

fn status_color(status: EventStatus) -> Color32 {
    match status {
        EventStatus::Announced => Color32::LIGHT_BLUE,
        EventStatus::Tentative => Color32::YELLOW,
        EventStatus::Scheduled => Color32::WHITE,
        EventStatus::Confirmed => Color32::LIGHT_GREEN,
        EventStatus::Rescheduled => Color32::from_rgb(232, 188, 92),
        EventStatus::Postponed => Color32::from_rgb(246, 210, 110),
        EventStatus::Cancelled => Color32::LIGHT_RED,
        EventStatus::Completed => Color32::GRAY,
        EventStatus::Observed => Color32::LIGHT_GREEN,
        EventStatus::Superseded => Color32::DARK_GRAY,
        EventStatus::Estimated => Color32::LIGHT_BLUE,
        EventStatus::Projected => Color32::LIGHT_BLUE,
        EventStatus::Disputed => Color32::LIGHT_RED,
        EventStatus::Unknown => Color32::GRAY,
    }
}
