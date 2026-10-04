use std::collections::{BTreeSet, HashMap};

use chrono::{Datelike, Local, NaiveDate, Utc};
use chrono_tz::Tz;
use eframe::egui::{self, Color32, RichText};
use uuid::Uuid;

use crate::calendar::{
    CalendarLayout, CalendarView, calendar_title, month_days, month_grid_start, quarter_months,
    shift_focus, week_days, window_for_view, year_months,
};
use crate::domain::{EventStatus, TemporalEvent, TemporalSource, TimeSpec};
use crate::query::{
    ColorBy, ColorRule, EventMembership, GroupBy, IntegerField, IntegerOperator, Overlay,
    PresenceField, QueryContext, QueryExpr, QueryPredicate, RgbColor, SavedView, SortDirection,
    SortField, SortRule, TemporalKind, TextField, TextOperator,
    matches_composed_or_overlay_with_membership,
};
use crate::state::PersistedUiState;
use crate::store::{TariaProjectedCalendarChoice, TariaReleaseStatusRecord, TemporalStore};
use crate::taria::import_reconciled_event_set_file;
use crate::taria_workspace::{
    detect_resourcearium_root, normalize_resourcearium_root,
    update_taria_sources as update_taria_workspace,
};

pub struct EphemerisApp {
    store: TemporalStore,
    state: PersistedUiState,
    events: Vec<TemporalEvent>,
    unplaced_events: Vec<TemporalEvent>,
    sources: Vec<TemporalSource>,
    taria_memberships: HashMap<Uuid, EventMembership>,
    taria_release_status: Option<TariaReleaseStatusRecord>,
    taria_bundle_refs: Vec<String>,
    taria_calendar_choices: Vec<TariaProjectedCalendarChoice>,
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

        if state.taria_resourcearium_root.trim().is_empty()
            && let Some(root) = detect_resourcearium_root()
        {
            state.taria_resourcearium_root = root.display().to_string();
            state.save()?;
        }

        let saved_views = store.list_saved_views()?;
        let mut app = Self {
            store,
            state,
            events: Vec::new(),
            unplaced_events: Vec::new(),
            sources: Vec::new(),
            taria_memberships: HashMap::new(),
            taria_release_status: None,
            taria_bundle_refs: Vec::new(),
            taria_calendar_choices: Vec::new(),
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

    fn detect_taria_workspace(&mut self) {
        match detect_resourcearium_root() {
            Some(root) => {
                self.state.taria_resourcearium_root = root.display().to_string();
                self.last_message = Some(format!(
                    "Detected local Taria Resourcearium at {}",
                    root.display()
                ));
                self.last_error = None;
                self.mark_state_dirty();
            }
            None => {
                self.last_message = None;
                self.last_error = Some(
                    "Could not auto-detect Taria. Set the Taria repo or Resourcearium path once."
                        .to_string(),
                );
            }
        }
    }

    fn update_taria_workspace(&mut self) {
        if self.state.taria_resourcearium_root.trim().is_empty() {
            self.detect_taria_workspace();
            if self.state.taria_resourcearium_root.trim().is_empty() {
                return;
            }
        }

        let configured = std::path::PathBuf::from(&self.state.taria_resourcearium_root);
        match normalize_resourcearium_root(&configured)
            .and_then(|root| update_taria_workspace(&self.store, &root, &self.state.taria_channel))
        {
            Ok(report) => {
                self.state.taria_resourcearium_root =
                    report.resourcearium_root.display().to_string();
                self.state.taria_last_release_id = Some(report.release_id.clone());
                self.state.taria_last_update_at = Some(Utc::now().to_rfc3339());
                self.state.taria_last_update_summary = Some(report.summary());

                let skipped = if report.skipped.is_empty() {
                    String::new()
                } else {
                    format!("; skipped: {}", report.skipped.join(" | "))
                };
                self.last_message = Some(format!(
                    "Updated Taria sources: {}{}",
                    report.summary(),
                    skipped
                ));
                self.last_error = None;
                self.mark_state_dirty();
                self.reload_or_report();
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to update local Taria sources: {error:#}"));
            }
        }
    }

    fn render_taria_workspace(&mut self, ui: &mut egui::Ui) {
        ui.heading("Taria");
        ui.small("Filesystem-first. Ephemeris reads Resourcearium artifacts directly from disk; no download or network step.");

        let root_changed = ui
            .add(
                egui::TextEdit::singleline(&mut self.state.taria_resourcearium_root)
                    .hint_text("/path/to/taria or /path/to/resourcearium"),
            )
            .changed();
        if root_changed {
            self.mark_state_dirty();
        }

        ui.horizontal_wrapped(|ui| {
            if ui.button("Detect local Taria").clicked() {
                self.detect_taria_workspace();
            }

            egui::ComboBox::from_id_salt("taria.channel")
                .selected_text(&self.state.taria_channel)
                .show_ui(ui, |ui| {
                    for channel in ["bootstrap", "production"] {
                        if ui
                            .selectable_value(
                                &mut self.state.taria_channel,
                                channel.to_string(),
                                channel,
                            )
                            .changed()
                        {
                            self.mark_state_dirty();
                        }
                    }
                });
        });

        if ui
            .add_sized(
                [ui.available_width(), 30.0],
                egui::Button::new(RichText::new("Update Taria Sources").strong()),
            )
            .clicked()
        {
            self.update_taria_workspace();
        }

        if let Some(release) = self.state.taria_last_release_id.as_deref() {
            ui.small(format!("Last adopted local release: {release}"));
        }
        if let Some(updated) = self.state.taria_last_update_at.as_deref() {
            ui.small(format!("Last update: {updated}"));
        }
        if let Some(summary) = self.state.taria_last_update_summary.as_deref() {
            ui.small(summary);
        }

        self.render_taria_release_status(ui);
    }

    fn render_taria_release_status(&self, ui: &mut egui::Ui) {
        let Some(release) = self.taria_release_status.as_ref() else {
            ui.small("No adopted Taria release metadata is loaded yet.");
            return;
        };

        ui.separator();
        ui.collapsing("Adopted release posture", |ui| {
            inspector_row(ui, "Release", &release.release_id);
            inspector_row(ui, "Channel", &release.channel);
            inspector_row(ui, "Status", &release.status);
            inspector_row(
                ui,
                "Production complete",
                if release.production_complete {
                    "yes"
                } else {
                    "no"
                },
            );
            if let Some(generated_at) = release.generated_at.as_deref() {
                inspector_row(ui, "Generated", generated_at);
            }
            inspector_row(ui, "Adopted", &release.adopted_at);

            let manifest = serde_json::from_str::<serde_json::Value>(&release.manifest_json).ok();
            let coverage = serde_json::from_str::<serde_json::Value>(&release.coverage_json).ok();

            if let Some(coverage) = coverage.as_ref().and_then(serde_json::Value::as_object) {
                ui.separator();
                ui.strong("Coverage");

                let ready = coverage
                    .get("ready_events")
                    .and_then(serde_json::Value::as_u64);
                let represented = coverage
                    .get("resources_represented_in_this_release")
                    .and_then(serde_json::Value::as_u64);
                let universe = coverage
                    .get("canonical_temporal_resource_universe")
                    .and_then(serde_json::Value::as_u64);
                let populated = coverage
                    .get("populated_bundle_slots")
                    .and_then(serde_json::Value::as_u64);
                let pending = coverage
                    .get("pending_bundle_slots")
                    .and_then(serde_json::Value::as_u64);
                let gap_only = coverage
                    .get("gap_only_bundle_slots")
                    .and_then(serde_json::Value::as_u64);

                if let Some(ready) = ready {
                    inspector_row(ui, "Ready events", &ready.to_string());
                }
                if let (Some(represented), Some(universe)) = (represented, universe) {
                    inspector_row(
                        ui,
                        "Resources represented",
                        &format!("{represented} / {universe}"),
                    );
                }
                if let Some(populated) = populated {
                    inspector_row(ui, "Populated bundle slots", &populated.to_string());
                }
                if let Some(pending) = pending {
                    inspector_row(ui, "Pending bundle slots", &pending.to_string());
                }
                if let Some(gap_only) = gap_only {
                    inspector_row(ui, "Gap-only bundle slots", &gap_only.to_string());
                }
            }

            if let Some(slots) = manifest
                .as_ref()
                .and_then(|manifest| manifest.get("canonical_bundle_slots"))
                .and_then(serde_json::Value::as_array)
            {
                ui.separator();
                ui.strong("Canonical bundles");
                for slot in slots {
                    let Some(slot) = slot.as_object() else {
                        continue;
                    };
                    let bundle_ref = slot
                        .get("bundle_ref")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown bundle");
                    let state = slot
                        .get("population_state")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown");
                    let ready = slot
                        .get("ready_event_count")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    ui.small(format!(
                        "{} · {} · {} ready",
                        short_bundle_label(bundle_ref),
                        state,
                        ready
                    ));
                }
            }

            ui.separator();
            ui.small(format!(
                "{} imported bundle refs · {} projected calendars available to queries",
                self.taria_bundle_refs.len(),
                self.taria_calendar_choices.len()
            ));
        });
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
        self.taria_memberships = self
            .store
            .taria_event_memberships_for_release(self.state.taria_last_release_id.as_deref())?;
        self.taria_release_status = self
            .store
            .taria_release_status(self.state.taria_last_release_id.as_deref())?;
        self.taria_bundle_refs = self
            .store
            .taria_bundle_refs_for_release(self.state.taria_last_release_id.as_deref())?;
        self.taria_calendar_choices = self.store.taria_projected_calendar_choices_for_release(
            self.state.taria_last_release_id.as_deref(),
        )?;
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
        let context = QueryContext::for_timezone(self.timezone());
        let mut events = self
            .events
            .iter()
            .filter(|event| {
                event
                    .source_id
                    .is_none_or(|source_id| !self.state.hidden_source_ids.contains(&source_id))
                    && matches_composed_or_overlay_with_membership(
                        &query,
                        &self.state.composition_layers,
                        &self.state.overlays,
                        event,
                        &context,
                        self.taria_memberships.get(&event.id),
                    )
            })
            .cloned()
            .collect::<Vec<_>>();
        sort_events(&mut events, self.timezone(), &self.state.sort_rules);
        events
    }

    fn visible_unplaced_events(&self) -> Vec<TemporalEvent> {
        let query = self.state.event_query();
        let context = QueryContext::for_timezone(self.timezone());
        let mut events = self
            .unplaced_events
            .iter()
            .filter(|event| {
                event
                    .source_id
                    .is_none_or(|source_id| !self.state.hidden_source_ids.contains(&source_id))
                    && matches_composed_or_overlay_with_membership(
                        &query,
                        &self.state.composition_layers,
                        &self.state.overlays,
                        event,
                        &context,
                        self.taria_memberships.get(&event.id),
                    )
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

            if ui.button("Update Taria Sources").clicked() {
                self.update_taria_workspace();
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
            ui.small("Taria updates read the configured Resourcearium filesystem directly");
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

        self.render_taria_workspace(ui);
        let membership_options = MembershipPredicateOptions {
            bundles: self.taria_bundle_refs.clone(),
            calendars: self.taria_calendar_choices.clone(),
        };
        ui.separator();

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

        ui.collapsing("Advanced query", |ui| {
            ui.small("Nested boolean predicates are ANDed with the simple facets above.");

            if self.state.query_expression.is_none() {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Add condition").clicked() {
                        self.state.query_expression =
                            Some(default_query_expr(QueryExprKind::Predicate));
                        filters_changed = true;
                    }
                    if ui.button("Add AND group").clicked() {
                        self.state.query_expression = Some(default_query_expr(QueryExprKind::All));
                        filters_changed = true;
                    }
                    if ui.button("Add OR group").clicked() {
                        self.state.query_expression = Some(default_query_expr(QueryExprKind::Any));
                        filters_changed = true;
                    }
                    if ui.button("Add NOT").clicked() {
                        self.state.query_expression = Some(default_query_expr(QueryExprKind::Not));
                        filters_changed = true;
                    }
                });
            } else {
                if let Some(expression) = self.state.query_expression.as_mut() {
                    filters_changed |=
                        render_query_expr_editor(ui, expression, "root", &membership_options);
                }
                if ui.button("Clear advanced query").clicked() {
                    self.state.query_expression = None;
                    filters_changed = true;
                }
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
            || !self.state.search_query.is_empty()
            || self.state.query_expression.is_some())
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

        ui.strong("Color rules");
        ui.small("Rules are evaluated top to bottom; the first enabled match wins.");

        presentation_changed |= render_color_rules_editor(
            ui,
            &mut self.state.color_rules,
            "base-color-rule",
            &membership_options,
        );

        ui.horizontal_wrapped(|ui| {
            ui.label("Fallback:");
            egui::ComboBox::from_id_salt("presentation.color")
                .selected_text(self.state.color_by.label())
                .show_ui(ui, |ui| {
                    for color_by in ColorBy::ALL {
                        presentation_changed |= ui
                            .selectable_value(&mut self.state.color_by, color_by, color_by.label())
                            .changed();
                    }
                });
        });

        ui.separator();
        ui.strong("Overlays");
        ui.small(
            "Enabled overlays union independent queries into the current view. Topmost matching overlay controls overlay styling.",
        );

        let mut remove_overlay = None;
        let mut swap_overlay = None;
        let overlay_count = self.state.overlays.len();

        for (index, overlay) in self.state.overlays.iter_mut().enumerate() {
            ui.group(|ui| {
                ui.horizontal_wrapped(|ui| {
                    presentation_changed |= ui.checkbox(&mut overlay.enabled, "").changed();
                    presentation_changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut overlay.name).hint_text("Overlay name"),
                        )
                        .changed();

                    egui::ComboBox::from_id_salt(("overlay-color", overlay.id))
                        .selected_text(overlay.color_by.label())
                        .show_ui(ui, |ui| {
                            for color_by in ColorBy::ALL {
                                presentation_changed |= ui
                                    .selectable_value(
                                        &mut overlay.color_by,
                                        color_by,
                                        color_by.label(),
                                    )
                                    .changed();
                            }
                        });

                    if index > 0
                        && ui
                            .small_button("↑")
                            .on_hover_text("Higher overlay precedence")
                            .clicked()
                    {
                        swap_overlay = Some((index, index - 1));
                    }
                    if index + 1 < overlay_count
                        && ui
                            .small_button("↓")
                            .on_hover_text("Lower overlay precedence")
                            .clicked()
                    {
                        swap_overlay = Some((index, index + 1));
                    }
                    if ui
                        .small_button("×")
                        .on_hover_text("Delete overlay")
                        .clicked()
                    {
                        remove_overlay = Some(index);
                    }
                });

                if overlay.query.expression.is_none() {
                    if overlay.query.is_empty() {
                        ui.small("Empty query matches every visible-source event.");
                    } else {
                        ui.small(
                            "This overlay also contains saved simple facets. They remain active.",
                        );
                    }

                    if ui.button("Add overlay condition").clicked() {
                        overlay.query.expression =
                            Some(default_query_expr(QueryExprKind::Predicate));
                        presentation_changed = true;
                    }
                } else if let Some(expression) = overlay.query.expression.as_mut() {
                    presentation_changed |= render_query_expr_editor(
                        ui,
                        expression,
                        &format!("overlay-query-{}", overlay.id),
                        &membership_options,
                    );
                }

                ui.collapsing("Overlay color rules", |ui| {
                    presentation_changed |= render_color_rules_editor(
                        ui,
                        &mut overlay.color_rules,
                        &format!("overlay-color-rule-{}", overlay.id),
                        &membership_options,
                    );
                });
            });
        }

        if let Some((left, right)) = swap_overlay {
            self.state.overlays.swap(left, right);
            presentation_changed = true;
        }
        if let Some(index) = remove_overlay {
            self.state.overlays.remove(index);
            presentation_changed = true;
        }

        if ui.button("Add overlay").clicked() {
            self.state.overlays.push(Overlay {
                id: Uuid::new_v4(),
                name: format!("Overlay {}", self.state.overlays.len() + 1),
                enabled: false,
                query: crate::query::EventQuery {
                    expression: Some(default_query_expr(QueryExprKind::Predicate)),
                    ..crate::query::EventQuery::default()
                },
                color_by: ColorBy::Domain,
                color_rules: Vec::new(),
            });
            presentation_changed = true;
        }

        ui.separator();
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
                                .selectable_value(&mut rule.direction, direction, direction.label())
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
            ui.small("Use Update Taria Sources above or drop a reconciled Taria JSON artifact.");
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
                            RichText::new(label).color(event_color(
                                &event,
                                ColorPresentation {
                                    fallback: self.state.color_by,
                                    rules: &self.state.color_rules,
                                    overlays: &self.state.overlays,
                                    memberships: &self.taria_memberships,
                                    query_context: QueryContext::for_timezone(self.timezone()),
                                },
                            )),
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
                                colors: ColorPresentation {
                                    fallback: self.state.color_by,
                                    rules: &self.state.color_rules,
                                    overlays: &self.state.overlays,
                                    memberships: &self.taria_memberships,
                                    query_context: QueryContext::for_timezone(timezone),
                                },
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

#[derive(Debug, Clone)]
struct MembershipPredicateOptions {
    bundles: Vec<String>,
    calendars: Vec<TariaProjectedCalendarChoice>,
}

fn render_color_rules_editor(
    ui: &mut egui::Ui,
    rules: &mut Vec<ColorRule>,
    id_prefix: &str,
    membership_options: &MembershipPredicateOptions,
) -> bool {
    let mut changed = false;
    let mut remove_rule = None;
    let mut swap_rule = None;
    let rule_count = rules.len();

    for (index, rule) in rules.iter_mut().enumerate() {
        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                changed |= ui.checkbox(&mut rule.enabled, "").changed();
                changed |= ui
                    .add(egui::TextEdit::singleline(&mut rule.name).hint_text("Color rule name"))
                    .changed();

                ui.label("RGB");
                changed |= ui
                    .add(egui::DragValue::new(&mut rule.color.r).range(0..=255))
                    .changed();
                changed |= ui
                    .add(egui::DragValue::new(&mut rule.color.g).range(0..=255))
                    .changed();
                changed |= ui
                    .add(egui::DragValue::new(&mut rule.color.b).range(0..=255))
                    .changed();

                if index > 0
                    && ui
                        .small_button("↑")
                        .on_hover_text("Higher precedence")
                        .clicked()
                {
                    swap_rule = Some((index, index - 1));
                }
                if index + 1 < rule_count
                    && ui
                        .small_button("↓")
                        .on_hover_text("Lower precedence")
                        .clicked()
                {
                    swap_rule = Some((index, index + 1));
                }
                if ui
                    .small_button("×")
                    .on_hover_text("Delete color rule")
                    .clicked()
                {
                    remove_rule = Some(index);
                }
            });

            changed |= render_query_expr_editor(
                ui,
                &mut rule.when,
                &format!("{id_prefix}-{}", rule.id),
                membership_options,
            );
        });
    }

    if let Some((left, right)) = swap_rule {
        rules.swap(left, right);
        changed = true;
    }
    if let Some(index) = remove_rule {
        rules.remove(index);
        changed = true;
    }

    if ui.button("Add color rule").clicked() {
        rules.push(ColorRule {
            id: Uuid::new_v4(),
            name: format!("Rule {}", rules.len() + 1),
            enabled: false,
            when: default_query_expr(QueryExprKind::Predicate),
            color: RgbColor::default(),
        });
        changed = true;
    }

    changed
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueryExprKind {
    Predicate,
    All,
    Any,
    Not,
}

impl QueryExprKind {
    const ALL: [Self; 4] = [Self::Predicate, Self::All, Self::Any, Self::Not];

    const fn label(self) -> &'static str {
        match self {
            Self::Predicate => "Condition",
            Self::All => "AND",
            Self::Any => "OR",
            Self::Not => "NOT",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueryPredicateKind {
    Text,
    TextAnyOf,
    StatusAnyOf,
    Integer,
    Exists,
    TemporalKindAnyOf,
    DateOverlaps,
    RelativeDateOverlaps,
    BundleMembership,
    ProjectedCalendarMembership,
}

impl QueryPredicateKind {
    const ALL: [Self; 10] = [
        Self::Text,
        Self::TextAnyOf,
        Self::StatusAnyOf,
        Self::Integer,
        Self::Exists,
        Self::TemporalKindAnyOf,
        Self::DateOverlaps,
        Self::RelativeDateOverlaps,
        Self::BundleMembership,
        Self::ProjectedCalendarMembership,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::TextAnyOf => "Text set",
            Self::StatusAnyOf => "Status set",
            Self::Integer => "Integer",
            Self::Exists => "Exists / missing",
            Self::TemporalKindAnyOf => "Time kind set",
            Self::DateOverlaps => "Date overlap",
            Self::RelativeDateOverlaps => "Relative date window",
            Self::BundleMembership => "Taria bundle membership",
            Self::ProjectedCalendarMembership => "Taria projected calendar membership",
        }
    }
}

fn default_query_expr(kind: QueryExprKind) -> QueryExpr {
    let condition = || {
        QueryExpr::Predicate(QueryPredicate::Text {
            field: TextField::Title,
            operator: TextOperator::Contains,
            value: String::new(),
            case_sensitive: false,
        })
    };

    match kind {
        QueryExprKind::Predicate => condition(),
        QueryExprKind::All => QueryExpr::All(vec![condition()]),
        QueryExprKind::Any => QueryExpr::Any(vec![condition()]),
        QueryExprKind::Not => QueryExpr::Not(Box::new(condition())),
    }
}

fn query_expr_kind(expression: &QueryExpr) -> QueryExprKind {
    match expression {
        QueryExpr::Predicate(_) => QueryExprKind::Predicate,
        QueryExpr::All(_) => QueryExprKind::All,
        QueryExpr::Any(_) => QueryExprKind::Any,
        QueryExpr::Not(_) => QueryExprKind::Not,
    }
}

fn default_query_predicate(kind: QueryPredicateKind) -> QueryPredicate {
    match kind {
        QueryPredicateKind::Text => QueryPredicate::Text {
            field: TextField::Title,
            operator: TextOperator::Contains,
            value: String::new(),
            case_sensitive: false,
        },
        QueryPredicateKind::TextAnyOf => QueryPredicate::TextAnyOf {
            field: TextField::Tags,
            values: Vec::new(),
            case_sensitive: false,
        },
        QueryPredicateKind::StatusAnyOf => QueryPredicate::StatusAnyOf { values: Vec::new() },
        QueryPredicateKind::Integer => QueryPredicate::Integer {
            field: IntegerField::Importance,
            operator: IntegerOperator::GreaterThanOrEqual,
            value: 0,
        },
        QueryPredicateKind::Exists => QueryPredicate::Exists {
            field: PresenceField::Institution,
            exists: true,
        },
        QueryPredicateKind::TemporalKindAnyOf => QueryPredicate::TemporalKindAnyOf {
            values: vec![
                TemporalKind::DateOnly,
                TemporalKind::AllDay,
                TemporalKind::Instant,
            ],
        },
        QueryPredicateKind::DateOverlaps => QueryPredicate::DateOverlaps {
            start: None,
            end_exclusive: None,
            include_imprecise: false,
        },
        QueryPredicateKind::RelativeDateOverlaps => QueryPredicate::RelativeDateOverlaps {
            start_offset_days: 1,
            end_offset_days_exclusive: 31,
            include_imprecise: false,
        },
        QueryPredicateKind::BundleMembership => QueryPredicate::BundleMembership {
            bundle_ref: String::new(),
        },
        QueryPredicateKind::ProjectedCalendarMembership => {
            QueryPredicate::ProjectedCalendarMembership {
                calendar_id: String::new(),
            }
        }
    }
}

fn query_predicate_kind(predicate: &QueryPredicate) -> QueryPredicateKind {
    match predicate {
        QueryPredicate::Text { .. } => QueryPredicateKind::Text,
        QueryPredicate::TextAnyOf { .. } => QueryPredicateKind::TextAnyOf,
        QueryPredicate::StatusAnyOf { .. } => QueryPredicateKind::StatusAnyOf,
        QueryPredicate::Integer { .. } => QueryPredicateKind::Integer,
        QueryPredicate::Exists { .. } => QueryPredicateKind::Exists,
        QueryPredicate::TemporalKindAnyOf { .. } => QueryPredicateKind::TemporalKindAnyOf,
        QueryPredicate::DateOverlaps { .. } => QueryPredicateKind::DateOverlaps,
        QueryPredicate::RelativeDateOverlaps { .. } => QueryPredicateKind::RelativeDateOverlaps,
        QueryPredicate::BundleMembership { .. } => QueryPredicateKind::BundleMembership,
        QueryPredicate::ProjectedCalendarMembership { .. } => {
            QueryPredicateKind::ProjectedCalendarMembership
        }
    }
}

fn render_query_expr_editor(
    ui: &mut egui::Ui,
    expression: &mut QueryExpr,
    path: &str,
    membership_options: &MembershipPredicateOptions,
) -> bool {
    let mut changed = false;
    let mut kind = query_expr_kind(expression);

    ui.group(|ui| {
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(("advanced-query-kind", path))
                .selected_text(kind.label())
                .show_ui(ui, |ui| {
                    for candidate in QueryExprKind::ALL {
                        changed |= ui
                            .selectable_value(&mut kind, candidate, candidate.label())
                            .changed();
                    }
                });

            if kind != query_expr_kind(expression) {
                *expression = default_query_expr(kind);
                changed = true;
            }
        });

        match expression {
            QueryExpr::Predicate(predicate) => {
                changed |=
                    render_query_predicate_editor(ui, predicate, path, membership_options);
            }
            QueryExpr::All(children) | QueryExpr::Any(children) => {
                let mut remove = None;
                for (index, child) in children.iter_mut().enumerate() {
                    let child_path = format!("{path}.{index}");
                    ui.horizontal_top(|ui| {
                        if ui
                            .small_button("×")
                            .on_hover_text("Remove clause")
                            .clicked()
                        {
                            remove = Some(index);
                        }
                        ui.vertical(|ui| {
                            changed |= render_query_expr_editor(
                                ui,
                                child,
                                &child_path,
                                membership_options,
                            );
                        });
                    });
                }

                if let Some(index) = remove {
                    children.remove(index);
                    changed = true;
                }

                ui.horizontal_wrapped(|ui| {
                    if ui.button("+ condition").clicked() {
                        children.push(default_query_expr(QueryExprKind::Predicate));
                        changed = true;
                    }
                    if ui.button("+ AND").clicked() {
                        children.push(default_query_expr(QueryExprKind::All));
                        changed = true;
                    }
                    if ui.button("+ OR").clicked() {
                        children.push(default_query_expr(QueryExprKind::Any));
                        changed = true;
                    }
                    if ui.button("+ NOT").clicked() {
                        children.push(default_query_expr(QueryExprKind::Not));
                        changed = true;
                    }
                });
            }
            QueryExpr::Not(child) => {
                ui.strong("Negates:");
                changed |= render_query_expr_editor(
                    ui,
                    child,
                    &format!("{path}.not"),
                    membership_options,
                );
            }
        }
    });

    changed
}

fn render_query_predicate_editor(
    ui: &mut egui::Ui,
    predicate: &mut QueryPredicate,
    path: &str,
    membership_options: &MembershipPredicateOptions,
) -> bool {
    let mut changed = false;
    let mut kind = query_predicate_kind(predicate);

    egui::ComboBox::from_id_salt(("advanced-predicate-kind", path))
        .selected_text(kind.label())
        .show_ui(ui, |ui| {
            for candidate in QueryPredicateKind::ALL {
                changed |= ui
                    .selectable_value(&mut kind, candidate, candidate.label())
                    .changed();
            }
        });

    if kind != query_predicate_kind(predicate) {
        *predicate = default_query_predicate(kind);
        changed = true;
    }

    match predicate {
        QueryPredicate::Text {
            field,
            operator,
            value,
            case_sensitive,
        } => {
            ui.horizontal_wrapped(|ui| {
                egui::ComboBox::from_id_salt(("advanced-text-field", path))
                    .selected_text(field.label())
                    .show_ui(ui, |ui| {
                        for candidate in TextField::ALL {
                            changed |= ui
                                .selectable_value(field, candidate, candidate.label())
                                .changed();
                        }
                    });
                egui::ComboBox::from_id_salt(("advanced-text-op", path))
                    .selected_text(operator.label())
                    .show_ui(ui, |ui| {
                        for candidate in TextOperator::ALL {
                            changed |= ui
                                .selectable_value(operator, candidate, candidate.label())
                                .changed();
                        }
                    });
            });
            changed |= ui
                .add(egui::TextEdit::singleline(value).hint_text("value"))
                .changed();
            changed |= ui.checkbox(case_sensitive, "Case sensitive").changed();
        }
        QueryPredicate::TextAnyOf {
            field,
            values,
            case_sensitive,
        } => {
            egui::ComboBox::from_id_salt(("advanced-set-field", path))
                .selected_text(field.label())
                .show_ui(ui, |ui| {
                    for candidate in TextField::ALL {
                        changed |= ui
                            .selectable_value(field, candidate, candidate.label())
                            .changed();
                    }
                });
            let mut joined = values.join(", ");
            if ui
                .add(
                    egui::TextEdit::singleline(&mut joined)
                        .hint_text("comma-separated accepted values"),
                )
                .changed()
            {
                *values = joined
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .collect();
                changed = true;
            }
            changed |= ui.checkbox(case_sensitive, "Case sensitive").changed();
        }
        QueryPredicate::StatusAnyOf { values } => {
            ui.horizontal_wrapped(|ui| {
                for status in EventStatus::ALL {
                    let mut selected = values.contains(&status);
                    if ui.checkbox(&mut selected, status.as_str()).changed() {
                        if selected {
                            values.push(status);
                        } else {
                            values.retain(|value| *value != status);
                        }
                        changed = true;
                    }
                }
            });
        }
        QueryPredicate::Integer {
            field,
            operator,
            value,
        } => {
            ui.horizontal_wrapped(|ui| {
                egui::ComboBox::from_id_salt(("advanced-int-field", path))
                    .selected_text(field.label())
                    .show_ui(ui, |ui| {
                        for candidate in IntegerField::ALL {
                            changed |= ui
                                .selectable_value(field, candidate, candidate.label())
                                .changed();
                        }
                    });
                egui::ComboBox::from_id_salt(("advanced-int-op", path))
                    .selected_text(operator.label())
                    .show_ui(ui, |ui| {
                        for candidate in IntegerOperator::ALL {
                            changed |= ui
                                .selectable_value(operator, candidate, candidate.label())
                                .changed();
                        }
                    });
                changed |= ui.add(egui::DragValue::new(value)).changed();
            });
        }
        QueryPredicate::Exists { field, exists } => {
            ui.horizontal_wrapped(|ui| {
                egui::ComboBox::from_id_salt(("advanced-exists-field", path))
                    .selected_text(field.label())
                    .show_ui(ui, |ui| {
                        for candidate in PresenceField::ALL {
                            changed |= ui
                                .selectable_value(field, candidate, candidate.label())
                                .changed();
                        }
                    });
                changed |= ui.selectable_value(exists, true, "exists").changed();
                changed |= ui.selectable_value(exists, false, "is missing").changed();
            });
        }
        QueryPredicate::TemporalKindAnyOf { values } => {
            ui.horizontal_wrapped(|ui| {
                for kind in TemporalKind::ALL {
                    let mut selected = values.contains(&kind);
                    if ui.checkbox(&mut selected, kind.label()).changed() {
                        if selected {
                            values.push(kind);
                        } else {
                            values.retain(|value| *value != kind);
                        }
                        changed = true;
                    }
                }
            });
        }
        QueryPredicate::DateOverlaps {
            start,
            end_exclusive,
            include_imprecise,
        } => {
            ui.small("Civil-date overlap in the current view timezone.");
            changed |= render_optional_date_editor(ui, "Start inclusive", start);
            changed |= render_optional_date_editor(ui, "End exclusive", end_exclusive);
            changed |= ui
                .checkbox(
                    include_imprecise,
                    "Include month/year-precision events by their full known span",
                )
                .changed();
        }
        QueryPredicate::RelativeDateOverlaps {
            start_offset_days,
            end_offset_days_exclusive,
            include_imprecise,
        } => {
            ui.small("Offsets are civil days from today; the end offset is exclusive.");
            ui.horizontal_wrapped(|ui| {
                changed |= ui
                    .add(
                        egui::DragValue::new(start_offset_days)
                            .prefix("start ")
                            .suffix(" d"),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::DragValue::new(end_offset_days_exclusive)
                            .prefix("end ")
                            .suffix(" d"),
                    )
                    .changed();
            });
            ui.horizontal_wrapped(|ui| {
                if ui.button("Today").clicked() {
                    *start_offset_days = 0;
                    *end_offset_days_exclusive = 1;
                    changed = true;
                }
                if ui.button("Next 7 days").clicked() {
                    *start_offset_days = 1;
                    *end_offset_days_exclusive = 8;
                    changed = true;
                }
                if ui.button("Next 30 days").clicked() {
                    *start_offset_days = 1;
                    *end_offset_days_exclusive = 31;
                    changed = true;
                }
                if ui.button("Previous 7 days").clicked() {
                    *start_offset_days = -7;
                    *end_offset_days_exclusive = 0;
                    changed = true;
                }
            });
            if *end_offset_days_exclusive <= *start_offset_days {
                ui.colored_label(Color32::LIGHT_RED, "End offset must be greater than start.");
            }
            changed |= ui
                .checkbox(
                    include_imprecise,
                    "Include month/year-precision events by their full known span",
                )
                .changed();
        }
        QueryPredicate::BundleMembership { bundle_ref } => {
            ui.small(
                "Matches current adopted-release membership; this does not change event ownership.",
            );
            let selected = if bundle_ref.is_empty() {
                "Select bundle…".to_string()
            } else {
                short_bundle_label(bundle_ref)
            };
            egui::ComboBox::from_id_salt(("advanced-bundle-ref", path))
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for candidate in &membership_options.bundles {
                        changed |= ui
                            .selectable_value(
                                bundle_ref,
                                candidate.clone(),
                                short_bundle_label(candidate),
                            )
                            .changed();
                    }
                });
            changed |= ui
                .add(
                    egui::TextEdit::singleline(bundle_ref)
                        .hint_text("bundle:temporal/politics-government"),
                )
                .changed();
        }
        QueryPredicate::ProjectedCalendarMembership { calendar_id } => {
            ui.small(
                "Matches a stable Resourcearium projected calendar ID in the current adopted release.",
            );
            let selected = membership_options
                .calendars
                .iter()
                .find(|calendar| calendar.calendar_id == *calendar_id)
                .map(|calendar| {
                    format!(
                        "{} · {}",
                        calendar.name,
                        short_bundle_label(&calendar.bundle_ref)
                    )
                })
                .unwrap_or_else(|| {
                    if calendar_id.is_empty() {
                        "Select projected calendar…".to_string()
                    } else {
                        calendar_id.clone()
                    }
                });
            egui::ComboBox::from_id_salt(("advanced-calendar-ref", path))
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for calendar in &membership_options.calendars {
                        let label = format!(
                            "{} · {}",
                            calendar.name,
                            short_bundle_label(&calendar.bundle_ref)
                        );
                        changed |= ui
                            .selectable_value(
                                calendar_id,
                                calendar.calendar_id.clone(),
                                label,
                            )
                            .changed();
                    }
                });
            changed |= ui
                .add(egui::TextEdit::singleline(calendar_id).hint_text("projected-calendar:..."))
                .changed();
        }
    }

    changed
}

fn render_optional_date_editor(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut Option<NaiveDate>,
) -> bool {
    let mut changed = false;
    let mut enabled = value.is_some();

    ui.horizontal_wrapped(|ui| {
        if ui.checkbox(&mut enabled, label).changed() {
            *value = if enabled {
                Some(Local::now().date_naive())
            } else {
                None
            };
            changed = true;
        }

        if let Some(date) = value.as_mut() {
            let mut year = date.year();
            let mut month = date.month();
            let mut day = date.day();

            let parts_changed = ui
                .add(egui::DragValue::new(&mut year).prefix("Y ").speed(1))
                .changed()
                | ui.add(egui::DragValue::new(&mut month).prefix("M ").range(1..=12))
                    .changed()
                | ui.add(egui::DragValue::new(&mut day).prefix("D ").range(1..=31))
                    .changed();

            if parts_changed {
                *date = clamped_date(year, month, day);
                changed = true;
            }
        }
    });

    changed
}

fn clamped_date(year: i32, month: u32, mut day: u32) -> NaiveDate {
    while day > 1 {
        if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
            return date;
        }
        day -= 1;
    }

    NaiveDate::from_ymd_opt(year, month, 1).unwrap_or_else(|| Local::now().date_naive())
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
    colors: ColorPresentation<'a>,
}

#[derive(Clone, Copy)]
struct ColorPresentation<'a> {
    fallback: ColorBy,
    rules: &'a [ColorRule],
    overlays: &'a [Overlay],
    memberships: &'a HashMap<Uuid, EventMembership>,
    query_context: QueryContext,
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
        colors,
    } = context;

    if events.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.heading("No events in this period");
            ui.label("The calendar is live; the corpus is simply empty.");
        });
    }

    match layout {
        CalendarLayout::Agenda => {
            render_agenda(ui, events, timezone, selected, group_by, sort_rules, colors)
        }
        CalendarLayout::Table => {
            render_table(ui, events, timezone, selected, group_by, sort_rules, colors)
        }
        CalendarLayout::Grid => match view {
            CalendarView::Year => render_year(ui, events, focus, timezone, selected, colors),
            CalendarView::Quarter => render_quarter(ui, events, focus, timezone, selected, colors),
            CalendarView::Month => {
                render_month(ui, events, focus, timezone, monday_start, selected, colors)
            }
            CalendarView::Week => {
                render_week(ui, events, focus, timezone, monday_start, selected, colors)
            }
            CalendarView::Day => render_day(ui, events, focus, timezone, selected, colors),
        },
    }
}

fn render_agenda(
    ui: &mut egui::Ui,
    events: &[TemporalEvent],
    timezone: Tz,
    selected: Option<Uuid>,
    group_by: GroupBy,
    sort_rules: &[SortRule],
    colors: ColorPresentation<'_>,
) -> Option<CalendarAction> {
    let groups = grouped_events(events, timezone, group_by, sort_rules);
    let mut action = None;

    for (group, group_events) in groups {
        if let Some(group) = group {
            ui.heading(group);
            ui.separator();
        }

        for event in group_events {
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
                            .color(event_color(event, colors))
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

        ui.add_space(8.0);
    }

    action
}

fn render_table(
    ui: &mut egui::Ui,
    events: &[TemporalEvent],
    timezone: Tz,
    selected: Option<Uuid>,
    group_by: GroupBy,
    sort_rules: &[SortRule],
    colors: ColorPresentation<'_>,
) -> Option<CalendarAction> {
    let groups = grouped_events(events, timezone, group_by, sort_rules);
    let mut action = None;

    for (group_index, (group, group_events)) in groups.into_iter().enumerate() {
        if let Some(group) = group {
            ui.heading(group);
        }

        egui::Grid::new(("event-table", group_index))
            .striped(true)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                for heading in [
                    "Date",
                    "Time / precision",
                    "Title",
                    "Type",
                    "Domain",
                    "Jurisdiction",
                    "Institution",
                    "Status",
                    "Importance",
                    "Relevance",
                    "Source",
                ] {
                    ui.strong(heading);
                }
                ui.end_row();

                for event in group_events {
                    ui.monospace(table_date_label(event, timezone));
                    ui.monospace(event.display_time_label(timezone));

                    if ui
                        .selectable_label(
                            selected == Some(event.id),
                            RichText::new(&event.normalized_title)
                                .color(event_color(event, colors))
                                .strong(),
                        )
                        .clicked()
                    {
                        action = Some(CalendarAction::Select(event.id));
                    }

                    ui.label(event.event_type.as_deref().unwrap_or("—"));
                    ui.label(event.domain.as_deref().unwrap_or("—"));
                    ui.label(event.jurisdiction.as_deref().unwrap_or("—"));
                    ui.label(event.institution.as_deref().unwrap_or("—"));
                    ui.label(event.status.as_str());
                    ui.label(
                        event
                            .importance
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "—".to_string()),
                    );
                    ui.label(
                        event
                            .personal_relevance
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "—".to_string()),
                    );
                    ui.label(event_source_key(event).unwrap_or_else(|| "—".to_string()));
                    ui.end_row();
                }
            });

        ui.add_space(10.0);
    }

    action
}

fn table_date_label(event: &TemporalEvent, timezone: Tz) -> String {
    match event.time {
        TimeSpec::Month { year, month } => format!("{year}-{month:02}"),
        TimeSpec::Year { year } => year.to_string(),
        TimeSpec::Unknown { .. } => "—".to_string(),
        _ => event
            .display_date(timezone)
            .map(|date| date.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "—".to_string()),
    }
}

fn grouped_events<'a>(
    events: &'a [TemporalEvent],
    timezone: Tz,
    group_by: GroupBy,
    sort_rules: &[SortRule],
) -> Vec<(Option<String>, Vec<&'a TemporalEvent>)> {
    let mut ordered = events.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| compare_events(left, right, timezone, sort_rules));

    if group_by == GroupBy::None {
        return vec![(None, ordered)];
    }

    let mut groups: Vec<(Option<String>, Vec<&TemporalEvent>)> = Vec::new();
    let mut indices = HashMap::<String, usize>::new();

    for event in ordered {
        let label = agenda_group_label(event, timezone, group_by);
        if let Some(index) = indices.get(&label).copied() {
            groups[index].1.push(event);
        } else {
            let index = groups.len();
            indices.insert(label.clone(), index);
            groups.push((Some(label), vec![event]));
        }
    }

    groups
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
        GroupBy::Source => event_source_key(event).unwrap_or_else(|| "No source".to_string()),
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

fn event_color(event: &TemporalEvent, colors: ColorPresentation<'_>) -> Color32 {
    let membership = colors.memberships.get(&event.id);

    if let Some(overlay) = colors
        .overlays
        .iter()
        .find(|overlay| overlay.matches_with_membership(event, &colors.query_context, membership))
    {
        if let Some(rule) = overlay
            .color_rules
            .iter()
            .find(|rule| rule.matches_with_membership(event, &colors.query_context, membership))
        {
            return rgb_color(rule.color);
        }
        return semantic_color(event, overlay.color_by);
    }

    if let Some(rule) = colors
        .rules
        .iter()
        .find(|rule| rule.matches_with_membership(event, &colors.query_context, membership))
    {
        return rgb_color(rule.color);
    }

    semantic_color(event, colors.fallback)
}

fn rgb_color(color: RgbColor) -> Color32 {
    Color32::from_rgb(color.r, color.g, color.b)
}

fn semantic_color(event: &TemporalEvent, color_by: ColorBy) -> Color32 {
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
    colors: ColorPresentation<'_>,
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
                if render_event_button(ui, event, timezone, selected, colors).clicked() {
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
                        if render_event_button(ui, event, timezone, selected, colors).clicked() {
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
    colors: ColorPresentation<'_>,
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
                    if render_event_button(ui, event, timezone, selected, colors).clicked() {
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
    colors: ColorPresentation<'_>,
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
                if render_event_button(ui, event, timezone, selected, colors).clicked() {
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
                        if render_event_button(ui, event, timezone, selected, colors).clicked() {
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
    colors: ColorPresentation<'_>,
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
                    if render_event_button(ui, event, timezone, selected, colors).clicked() {
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
    colors: ColorPresentation<'_>,
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
                        .color(event_color(event, colors))
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
    colors: ColorPresentation<'_>,
) -> egui::Response {
    let time = event.display_time_label(timezone);
    let label = format!("{time}  {}", event.normalized_title);
    ui.selectable_label(
        selected == Some(event.id),
        RichText::new(label).color(event_color(event, colors)),
    )
}

fn events_for_day(events: &[TemporalEvent], day: NaiveDate, timezone: Tz) -> Vec<&TemporalEvent> {
    events
        .iter()
        .filter(|event| event.time.occurs_on(day, timezone))
        .collect()
}

fn short_bundle_label(bundle_ref: &str) -> String {
    bundle_ref
        .strip_prefix("bundle:temporal/")
        .unwrap_or(bundle_ref)
        .replace('-', " ")
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
