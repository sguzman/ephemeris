use std::collections::{BTreeSet, HashMap};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local, NaiveDate, Timelike, Utc};
use chrono_tz::Tz;
use eframe::egui::{self, Color32, RichText};
use uuid::Uuid;

use crate::calendar::{
    CalendarLayout, CalendarView, calendar_title, month_days, month_grid_start, quarter_months,
    shift_focus, week_days, window_for_view, year_months,
};
use crate::domain::{EventStatus, TemporalEvent, TemporalSource, TimeSpec};
use crate::query::{
    ColorBy, ColorRule, CompositionLayer, CompositionOperator, EventMembership, GroupBy,
    IntegerField, IntegerOperator, Overlay, PresenceField, QueryContext, QueryExpr, QueryPredicate,
    RgbColor, SavedView, SortDirection, SortField, SortRule, TableColumn, TemporalKind, TextField,
    TextOperator, matches_composed_or_overlay_with_saved_views_and_membership,
    saved_view_reference_cycle,
};
use crate::state::PersistedUiState;
use crate::store::{
    SourceRefreshAttempt, TariaProjectedCalendarChoice, TariaReleaseDiff, TariaReleaseHistoryEntry,
    TariaReleaseStatusRecord, TemporalStore,
};
use crate::taria::import_reconciled_event_set_file;
use crate::taria_workspace::{
    TariaWorkspaceUpdateReport, detect_resourcearium_root, normalize_resourcearium_root,
    update_taria_sources as update_taria_workspace,
};

const TARIA_STALE_AFTER_HOURS: i64 = 7 * 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TariaRefreshHealth {
    NeverRefreshed,
    Running,
    Healthy,
    Stale,
    Failed,
    Interrupted,
    Unknown,
}

impl TariaRefreshHealth {
    const fn label(self) -> &'static str {
        match self {
            Self::NeverRefreshed => "never refreshed",
            Self::Running => "running",
            Self::Healthy => "healthy",
            Self::Stale => "stale",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
            Self::Unknown => "unknown",
        }
    }
}

pub struct EphemerisApp {
    store: TemporalStore,
    state: PersistedUiState,
    events: Vec<TemporalEvent>,
    unplaced_events: Vec<TemporalEvent>,
    sources: Vec<TemporalSource>,
    source_event_counts: HashMap<Uuid, u64>,
    selected_source_id: Option<Uuid>,
    taria_current_source_ids: BTreeSet<Uuid>,
    taria_memberships: HashMap<Uuid, EventMembership>,
    taria_release_status: Option<TariaReleaseStatusRecord>,
    taria_release_history: Vec<TariaReleaseHistoryEntry>,
    taria_previous_release_diff: Option<TariaReleaseDiff>,
    taria_bundle_refs: Vec<String>,
    taria_calendar_choices: Vec<TariaProjectedCalendarChoice>,
    source_refresh_attempts: Vec<SourceRefreshAttempt>,
    taria_update_receiver: Option<(Uuid, Receiver<Result<TariaWorkspaceUpdateReport, String>>)>,
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
            source_event_counts: HashMap::new(),
            selected_source_id: None,
            taria_current_source_ids: BTreeSet::new(),
            taria_memberships: HashMap::new(),
            taria_release_status: None,
            taria_release_history: Vec::new(),
            taria_previous_release_diff: None,
            taria_bundle_refs: Vec::new(),
            taria_calendar_choices: Vec::new(),
            source_refresh_attempts: Vec::new(),
            taria_update_receiver: None,
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

    fn start_taria_workspace_update(&mut self) {
        if self.taria_update_receiver.is_some() {
            return;
        }

        if self.state.taria_resourcearium_root.trim().is_empty() {
            self.detect_taria_workspace();
            if self.state.taria_resourcearium_root.trim().is_empty() {
                return;
            }
        }

        let Some(database_path) = self.store.path().map(std::path::Path::to_path_buf) else {
            self.last_message = None;
            self.last_error = Some(
                "Background Taria updates require a file-backed Ephemeris database.".to_string(),
            );
            return;
        };

        let configured = std::path::PathBuf::from(&self.state.taria_resourcearium_root);
        let channel = self.state.taria_channel.clone();
        let refresh_target = format!("{channel} · {}", configured.display());
        let attempt_id = match self
            .store
            .begin_refresh_attempt("taria_workspace", &refresh_target)
        {
            Ok(attempt_id) => attempt_id,
            Err(error) => {
                self.last_message = None;
                self.last_error =
                    Some(format!("Failed to record Taria refresh attempt: {error:#}"));
                return;
            }
        };

        let (sender, receiver) = mpsc::channel();
        self.taria_update_receiver = Some((attempt_id, receiver));
        self.source_refresh_attempts = self.store.source_refresh_attempts(20).unwrap_or_default();
        self.last_message = Some(format!(
            "Updating Taria sources from local {channel} release..."
        ));
        self.last_error = None;

        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<TariaWorkspaceUpdateReport> {
                let worker_store = TemporalStore::open(database_path)?;
                let root = normalize_resourcearium_root(&configured)?;
                update_taria_workspace(&worker_store, &root, &channel)
            })()
            .map_err(|error| format!("{error:#}"));

            let _ = sender.send(result);
        });
    }

    fn poll_taria_workspace_update(&mut self) {
        let result = match self.taria_update_receiver.as_ref() {
            Some((attempt_id, receiver)) => match receiver.try_recv() {
                Ok(result) => Some((*attempt_id, result)),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some((
                    *attempt_id,
                    Err("Taria update worker exited without returning a result.".to_string()),
                )),
            },
            None => None,
        };

        let Some((attempt_id, result)) = result else {
            return;
        };
        self.taria_update_receiver = None;

        match result {
            Ok(report) => {
                let summary = report.summary();
                let history_error = self
                    .store
                    .finish_refresh_attempt(
                        attempt_id,
                        true,
                        Some(&report.release_id),
                        Some(&summary),
                        None,
                    )
                    .err();

                self.state.taria_resourcearium_root =
                    report.resourcearium_root.display().to_string();
                self.state.taria_last_release_id = Some(report.release_id.clone());
                self.state.taria_last_update_at = Some(Utc::now().to_rfc3339());
                self.state.taria_last_update_summary = Some(summary.clone());

                let skipped = if report.skipped.is_empty() {
                    String::new()
                } else {
                    format!("; skipped: {}", report.skipped.join(" | "))
                };
                self.last_message = Some(format!("Updated Taria sources: {}{}", summary, skipped));
                let history_error = history_error.map(|error| {
                    format!(
                        "Taria update succeeded, but refresh history could not be completed: {error:#}"
                    )
                });
                self.mark_state_dirty();
                self.reload_or_report();
                if history_error.is_some() {
                    self.last_error = history_error;
                }
            }
            Err(error) => {
                let history_error = self
                    .store
                    .finish_refresh_attempt(attempt_id, false, None, None, Some(&error))
                    .err();
                self.source_refresh_attempts =
                    self.store.source_refresh_attempts(20).unwrap_or_default();
                self.last_message = None;
                self.last_error = Some(match history_error {
                    Some(history_error) => format!(
                        "Failed to update local Taria sources: {error}; refresh history also failed: {history_error:#}"
                    ),
                    None => format!("Failed to update local Taria sources: {error}"),
                });
            }
        }
    }

    fn render_taria_workspace(&mut self, ui: &mut egui::Ui) {
        ui.heading("Taria");
        ui.small("Filesystem-first. Ephemeris reads Resourcearium artifacts directly from disk; no download or network step.");

        let updating = self.taria_update_receiver.is_some();
        ui.add_enabled_ui(!updating, |ui| {
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
        });

        let button_text = if updating {
            "Updating Taria Sources..."
        } else {
            "Update Taria Sources"
        };
        if ui
            .add_enabled(
                !updating,
                egui::Button::new(RichText::new(button_text).strong())
                    .min_size(egui::vec2(ui.available_width(), 30.0)),
            )
            .clicked()
        {
            self.start_taria_workspace_update();
        }
        if updating {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.small("Adopting release in background");
            });
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

        let active_attempt_id = self
            .taria_update_receiver
            .as_ref()
            .map(|(attempt_id, _)| *attempt_id);
        let refresh_health = taria_refresh_health(
            &self.source_refresh_attempts,
            active_attempt_id,
            self.taria_release_status.is_some(),
            Utc::now(),
        );
        let health_color = match refresh_health {
            TariaRefreshHealth::Healthy => Color32::LIGHT_GREEN,
            TariaRefreshHealth::Running => Color32::LIGHT_BLUE,
            TariaRefreshHealth::Stale | TariaRefreshHealth::Interrupted => Color32::YELLOW,
            TariaRefreshHealth::Failed => Color32::LIGHT_RED,
            TariaRefreshHealth::NeverRefreshed | TariaRefreshHealth::Unknown => Color32::GRAY,
        };
        ui.horizontal_wrapped(|ui| {
            ui.small("Refresh health:");
            ui.colored_label(health_color, refresh_health.label());
            ui.small(format!(
                "· stale after {} days without a successful refresh",
                TARIA_STALE_AFTER_HOURS / 24
            ));
        });

        ui.collapsing(
            format!("Refresh history ({})", self.source_refresh_attempts.len()),
            |ui| {
                if self.source_refresh_attempts.is_empty() {
                    ui.small("No persisted refresh attempts yet.");
                }

                for attempt in &self.source_refresh_attempts {
                    let status = match attempt.success {
                        Some(true) => "success",
                        Some(false) => "failed",
                        None if active_attempt_id == Some(attempt.id) => "running",
                        None => "incomplete",
                    };
                    ui.strong(format!(
                        "{} · {} · {}",
                        status, attempt.refresh_kind, attempt.target
                    ));
                    ui.small(format!("Started {}", attempt.started_at));
                    if let Some(completed_at) = attempt.completed_at.as_deref() {
                        ui.small(format!("Completed {completed_at}"));
                    }
                    if let Some(release_id) = attempt.release_id.as_deref() {
                        ui.small(format!("Release {release_id}"));
                    }
                    if let Some(summary) = attempt.summary.as_deref() {
                        ui.small(summary);
                    }
                    if let Some(error) = attempt.error.as_deref() {
                        ui.colored_label(Color32::LIGHT_RED, error);
                    }
                    ui.add_space(6.0);
                }
            },
        );

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

        ui.collapsing(
            format!("Release history ({})", self.taria_release_history.len()),
            |ui| {
                if self.taria_release_history.is_empty() {
                    ui.small("No adopted release history yet.");
                    return;
                }

                for release in &self.taria_release_history {
                    let current = self
                        .state
                        .taria_last_release_id
                        .as_deref()
                        .is_some_and(|release_id| release_id == release.release_id);
                    let prefix = if current { "Current · " } else { "" };
                    ui.small(format!(
                        "{prefix}{} · {} · {} · {} sources · {} event snapshots · {} bundles · {} calendars · {} member events",
                        release.release_id,
                        release.channel,
                        release.status,
                        release.source_count,
                        release.snapshot_event_count,
                        release.bundle_count,
                        release.projected_calendar_count,
                        release.resolved_member_event_count
                    ));
                    ui.small(format!("Adopted {}", release.adopted_at));
                }

                if let Some(diff) = self.taria_previous_release_diff.as_ref() {
                    ui.separator();
                    ui.strong(format!(
                        "Changes from {}",
                        diff.from_release_id
                    ));
                    ui.small(format!(
                        "Sources +{} / -{} · canonical events +{} / -{} · moved {} · newly cancelled {} · status changes {} · calendar-member events +{} / -{} · bundles +{} / -{} · projected calendars +{} / -{}",
                        diff.added_source_projection_refs.len(),
                        diff.removed_source_projection_refs.len(),
                        diff.added_snapshot_event_ids.len(),
                        diff.removed_snapshot_event_ids.len(),
                        diff.moved_event_ids.len(),
                        diff.newly_cancelled_event_ids.len(),
                        diff.status_changed_event_ids.len(),
                        diff.added_member_event_ids.len(),
                        diff.removed_member_event_ids.len(),
                        diff.added_bundle_refs.len(),
                        diff.removed_bundle_refs.len(),
                        diff.added_calendar_ids.len(),
                        diff.removed_calendar_ids.len()
                    ));

                    if !diff.added_source_projection_refs.is_empty() {
                        ui.small(format!(
                            "Added sources: {}",
                            diff.added_source_projection_refs.join(", ")
                        ));
                    }
                    if !diff.removed_source_projection_refs.is_empty() {
                        ui.small(format!(
                            "Removed sources: {}",
                            diff.removed_source_projection_refs.join(", ")
                        ));
                    }
                    if !diff.added_bundle_refs.is_empty() {
                        ui.small(format!(
                            "Added bundles: {}",
                            diff.added_bundle_refs
                                .iter()
                                .map(|value| short_bundle_label(value))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                    if !diff.removed_bundle_refs.is_empty() {
                        ui.small(format!(
                            "Removed bundles: {}",
                            diff.removed_bundle_refs
                                .iter()
                                .map(|value| short_bundle_label(value))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }


                    if !diff.event_changes.is_empty() {
                        ui.collapsing(
                            format!("Canonical event changes ({})", diff.event_changes.len()),
                            |ui| {
                                for change in &diff.event_changes {
                                    let mut kinds = Vec::new();
                                    if change.added {
                                        kinds.push("added");
                                    }
                                    if change.removed {
                                        kinds.push("removed");
                                    }
                                    if change.title_changed {
                                        kinds.push("renamed");
                                    }
                                    if change.moved {
                                        kinds.push("moved");
                                    }
                                    if change.newly_cancelled {
                                        kinds.push("cancelled");
                                    } else if change.status_changed {
                                        kinds.push("status");
                                    }

                                    ui.collapsing(
                                        format!("{} · {}", kinds.join(" + "), change.title),
                                        |ui| {
                                            inspector_row(
                                                ui,
                                                "Event ID",
                                                &change.event_id.to_string(),
                                            );

                                            if change.title_changed {
                                                if let Some(from_title) =
                                                    change.from_title.as_deref()
                                                {
                                                    inspector_row(
                                                        ui,
                                                        "Previous title",
                                                        from_title,
                                                    );
                                                }
                                                if let Some(to_title) = change.to_title.as_deref() {
                                                    inspector_row(ui, "New title", to_title);
                                                }
                                            }

                                            if change.status_changed {
                                                inspector_row(
                                                    ui,
                                                    "Previous status",
                                                    change
                                                        .from_status
                                                        .as_deref()
                                                        .unwrap_or("unknown"),
                                                );
                                                inspector_row(
                                                    ui,
                                                    "New status",
                                                    change.to_status.as_deref().unwrap_or("unknown"),
                                                );
                                            } else if change.added || change.removed {
                                                let status = change
                                                    .to_status
                                                    .as_deref()
                                                    .or(change.from_status.as_deref())
                                                    .unwrap_or("unknown");
                                                inspector_row(ui, "Status", status);
                                            }

                                            if change.moved {
                                                inspector_row(
                                                    ui,
                                                    "Previous time",
                                                    &snapshot_time_label(
                                                        change.from_time_json.as_deref(),
                                                        self.timezone(),
                                                    ),
                                                );
                                                inspector_row(
                                                    ui,
                                                    "New time",
                                                    &snapshot_time_label(
                                                        change.to_time_json.as_deref(),
                                                        self.timezone(),
                                                    ),
                                                );
                                            } else if change.added || change.removed {
                                                let time = change
                                                    .to_time_json
                                                    .as_deref()
                                                    .or(change.from_time_json.as_deref());
                                                inspector_row(
                                                    ui,
                                                    "Time",
                                                    &snapshot_time_label(time, self.timezone()),
                                                );
                                            }
                                        },
                                    );
                                }
                            },
                        );
                    }
                } else if self.taria_release_history.len() > 1 {
                    ui.small("No previous release on the current channel.");
                }

                ui.small(
                    "Canonical event deltas come from immutable per-release snapshots; CalendarSet member-event deltas remain a separate membership view.",
                );
            },
        );
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
        self.source_event_counts = self.store.source_event_counts()?;
        self.taria_current_source_ids = self
            .store
            .taria_source_ids_for_release(self.state.taria_last_release_id.as_deref())?;
        if self
            .selected_source_id
            .is_some_and(|selected| !self.sources.iter().any(|source| source.id == selected))
        {
            self.selected_source_id = None;
        }
        self.taria_memberships = self
            .store
            .taria_event_memberships_for_release(self.state.taria_last_release_id.as_deref())?;
        self.taria_release_status = self
            .store
            .taria_release_status(self.state.taria_last_release_id.as_deref())?;
        self.taria_release_history = self.store.taria_release_history()?;
        self.taria_previous_release_diff = self
            .state
            .taria_last_release_id
            .as_deref()
            .map(|release_id| self.store.taria_previous_release_diff(release_id))
            .transpose()?
            .flatten();
        self.taria_bundle_refs = self
            .store
            .taria_bundle_refs_for_release(self.state.taria_last_release_id.as_deref())?;
        self.taria_calendar_choices = self.store.taria_projected_calendar_choices_for_release(
            self.state.taria_last_release_id.as_deref(),
        )?;
        self.source_refresh_attempts = self.store.source_refresh_attempts(20)?;
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

    fn saved_view_cycle_error(&self, candidate: &SavedView) -> Option<String> {
        let mut prospective = self.saved_views.clone();
        if let Some(index) = prospective.iter().position(|view| view.id == candidate.id) {
            prospective[index] = candidate.clone();
        } else {
            prospective.push(candidate.clone());
        }

        let cycle = saved_view_reference_cycle(&prospective, candidate.id)?;
        let names = cycle
            .into_iter()
            .map(|id| {
                prospective
                    .iter()
                    .find(|view| view.id == id)
                    .map_or_else(|| id.to_string(), |view| view.name.clone())
            })
            .collect::<Vec<_>>();

        Some(format!(
            "Saved-view composition cycle rejected: {}",
            names.join(" -> ")
        ))
    }

    fn save_current_view(&mut self) {
        let name = self.saved_view_name.trim();
        if name.is_empty() {
            return;
        }

        let view = self.state.capture_saved_view(name);
        if let Some(error) = self.saved_view_cycle_error(&view) {
            self.last_message = None;
            self.last_error = Some(error);
            return;
        }

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

        if let Some(error) = self.saved_view_cycle_error(&replacement) {
            self.last_message = None;
            self.last_error = Some(error);
            return;
        }

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
                    && matches_composed_or_overlay_with_saved_views_and_membership(
                        &query,
                        &self.state.composition_layers,
                        &self.state.overlays,
                        &self.saved_views,
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
                    && matches_composed_or_overlay_with_saved_views_and_membership(
                        &query,
                        &self.state.composition_layers,
                        &self.state.overlays,
                        &self.saved_views,
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
            if input.key_pressed(egui::Key::C) {
                target_layout = Some(CalendarLayout::CompactAgenda);
            }
            if input.key_pressed(egui::Key::S) {
                target_layout = Some(CalendarLayout::Stream);
            }
            if input.key_pressed(egui::Key::L) {
                target_layout = Some(CalendarLayout::Timeline);
            }
            if input.key_pressed(egui::Key::H) {
                target_layout = Some(CalendarLayout::Density);
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

            let updating_taria = self.taria_update_receiver.is_some();
            if ui
                .add_enabled(
                    !updating_taria,
                    egui::Button::new(if updating_taria {
                        "Updating Taria..."
                    } else {
                        "Update Taria Sources"
                    }),
                )
                .clicked()
            {
                self.start_taria_workspace_update();
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

        let event_types = self
            .events
            .iter()
            .chain(self.unplaced_events.iter())
            .filter_map(|event| event.event_type.clone())
            .collect::<BTreeSet<_>>();
        egui::ComboBox::from_id_salt("query.event_type")
            .selected_text(
                self.state
                    .event_type_filter
                    .as_deref()
                    .unwrap_or("All event types"),
            )
            .show_ui(ui, |ui| {
                filters_changed |= ui
                    .selectable_value(&mut self.state.event_type_filter, None, "All event types")
                    .changed();
                for event_type in event_types {
                    filters_changed |= ui
                        .selectable_value(
                            &mut self.state.event_type_filter,
                            Some(event_type.clone()),
                            event_type,
                        )
                        .changed();
                }
            });

        let institutions = self
            .events
            .iter()
            .chain(self.unplaced_events.iter())
            .filter_map(|event| event.institution.clone())
            .collect::<BTreeSet<_>>();
        egui::ComboBox::from_id_salt("query.institution")
            .selected_text(
                self.state
                    .institution_filter
                    .as_deref()
                    .unwrap_or("All institutions"),
            )
            .show_ui(ui, |ui| {
                filters_changed |= ui
                    .selectable_value(&mut self.state.institution_filter, None, "All institutions")
                    .changed();
                for institution in institutions {
                    filters_changed |= ui
                        .selectable_value(
                            &mut self.state.institution_filter,
                            Some(institution.clone()),
                            institution,
                        )
                        .changed();
                }
            });

        let renderabilities = self
            .events
            .iter()
            .chain(self.unplaced_events.iter())
            .filter_map(|event| event.renderability.clone())
            .collect::<BTreeSet<_>>();
        egui::ComboBox::from_id_salt("query.renderability")
            .selected_text(
                self.state
                    .renderability_filter
                    .as_deref()
                    .unwrap_or("All renderability"),
            )
            .show_ui(ui, |ui| {
                filters_changed |= ui
                    .selectable_value(
                        &mut self.state.renderability_filter,
                        None,
                        "All renderability",
                    )
                    .changed();
                for renderability in renderabilities {
                    filters_changed |= ui
                        .selectable_value(
                            &mut self.state.renderability_filter,
                            Some(renderability.clone()),
                            renderability,
                        )
                        .changed();
                }
            });

        let tags = self
            .events
            .iter()
            .chain(self.unplaced_events.iter())
            .flat_map(|event| event.tags.iter().cloned())
            .collect::<BTreeSet<_>>();
        egui::ComboBox::from_id_salt("query.tag")
            .selected_text(self.state.tag_filter.as_deref().unwrap_or("All tags"))
            .show_ui(ui, |ui| {
                filters_changed |= ui
                    .selectable_value(&mut self.state.tag_filter, None, "All tags")
                    .changed();
                for tag in tags {
                    filters_changed |= ui
                        .selectable_value(&mut self.state.tag_filter, Some(tag.clone()), tag)
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

        let ordering_locked = matches!(
            self.state.calendar_layout,
            CalendarLayout::Stream | CalendarLayout::Timeline | CalendarLayout::Density
        );
        let density_layout = self.state.calendar_layout == CalendarLayout::Density;
        if ordering_locked {
            ui.small(match self.state.calendar_layout {
                CalendarLayout::Density => {
                    "Density aggregates by day. Saved grouping and sort rules are preserved but do not alter this aggregate layout."
                }
                _ => {
                    "Stream/Timeline own temporal ordering. Saved grouping and sort rules are preserved for Agenda/Compact/Table but do not alter these chronological layouts."
                }
            });
        }
        ui.add_enabled_ui(!ordering_locked, |ui| {
            egui::ComboBox::from_id_salt("presentation.group")
                .selected_text(self.state.group_by.label())
                .show_ui(ui, |ui| {
                    for group_by in GroupBy::ALL {
                        presentation_changed |= ui
                            .selectable_value(&mut self.state.group_by, group_by, group_by.label())
                            .changed();
                    }
                });
        });

        ui.add_enabled_ui(!density_layout, |ui| {
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
                                .selectable_value(
                                    &mut self.state.color_by,
                                    color_by,
                                    color_by.label(),
                                )
                                .changed();
                        }
                    });
            });
        });
        if density_layout {
            ui.small(
                "Density uses aggregate intensity rather than per-event color rules. Saved color settings are preserved for other layouts.",
            );
        }

        ui.separator();
        ui.strong("Calendar algebra");
        ui.small(
            "Layers run top to bottom over the base query. Union adds, Intersect narrows, Subtract removes. Overlays are applied afterward.",
        );

        let mut remove_composition_layer = None;
        let mut swap_composition_layer = None;
        let composition_layer_count = self.state.composition_layers.len();
        let active_saved_view_id = self.state.active_saved_view_id;
        let saved_view_options = self
            .saved_views
            .iter()
            .map(|view| (view.id, view.name.clone()))
            .collect::<Vec<_>>();

        for (index, layer) in self.state.composition_layers.iter_mut().enumerate() {
            ui.group(|ui| {
                ui.horizontal_wrapped(|ui| {
                    presentation_changed |= ui.checkbox(&mut layer.enabled, "").changed();
                    presentation_changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut layer.name)
                                .hint_text("Algebra layer name"),
                        )
                        .changed();

                    egui::ComboBox::from_id_salt(("composition-operator", layer.id))
                        .selected_text(format!(
                            "{} {}",
                            layer.operator.symbol(),
                            layer.operator.label()
                        ))
                        .show_ui(ui, |ui| {
                            for operator in CompositionOperator::ALL {
                                presentation_changed |= ui
                                    .selectable_value(
                                        &mut layer.operator,
                                        operator,
                                        format!("{} {}", operator.symbol(), operator.label()),
                                    )
                                    .changed();
                            }
                        });

                    if index > 0
                        && ui
                            .small_button("↑")
                            .on_hover_text("Evaluate earlier")
                            .clicked()
                    {
                        swap_composition_layer = Some((index, index - 1));
                    }
                    if index + 1 < composition_layer_count
                        && ui
                            .small_button("↓")
                            .on_hover_text("Evaluate later")
                            .clicked()
                    {
                        swap_composition_layer = Some((index, index + 1));
                    }
                    if ui
                        .small_button("×")
                        .on_hover_text("Delete algebra layer")
                        .clicked()
                    {
                        remove_composition_layer = Some(index);
                    }
                });

                ui.horizontal_wrapped(|ui| {
                    ui.label("Operand");
                    let selected_operand = layer
                        .saved_view_id
                        .and_then(|id| {
                            saved_view_options
                                .iter()
                                .find(|(candidate_id, _)| *candidate_id == id)
                                .map(|(_, name)| name.clone())
                        })
                        .unwrap_or_else(|| {
                            if layer.saved_view_id.is_some() {
                                "Missing saved view".to_string()
                            } else {
                                "Embedded query".to_string()
                            }
                        });

                    egui::ComboBox::from_id_salt(("composition-operand", layer.id))
                        .selected_text(selected_operand)
                        .show_ui(ui, |ui| {
                            presentation_changed |= ui
                                .selectable_value(
                                    &mut layer.saved_view_id,
                                    None,
                                    "Embedded query",
                                )
                                .changed();

                            for (saved_view_id, name) in &saved_view_options {
                                if Some(*saved_view_id) == active_saved_view_id {
                                    continue;
                                }
                                presentation_changed |= ui
                                    .selectable_value(
                                        &mut layer.saved_view_id,
                                        Some(*saved_view_id),
                                        name,
                                    )
                                    .changed();
                            }
                        });
                });

                if let Some(saved_view_id) = layer.saved_view_id {
                    if let Some((_, name)) = saved_view_options
                        .iter()
                        .find(|(candidate_id, _)| *candidate_id == saved_view_id)
                    {
                        ui.small(format!(
                            "Uses the logical event set of saved view '{name}'. Its query, source visibility, composition, and overlays participate; presentation does not."
                        ));
                    } else {
                        ui.colored_label(
                            Color32::LIGHT_RED,
                            "Referenced saved view is missing. This layer is a no-op until the reference resolves or is changed.",
                        );
                    }
                } else if layer.query.expression.is_none() {
                    if layer.query.is_empty() {
                        ui.small("Empty query matches every visible-source event.");
                    } else {
                        ui.small(
                            "This layer also contains saved simple facets. They remain active.",
                        );
                    }

                    if ui.button("Add layer condition").clicked() {
                        layer.query.expression = Some(default_query_expr(QueryExprKind::Predicate));
                        presentation_changed = true;
                    }
                } else if let Some(expression) = layer.query.expression.as_mut() {
                    presentation_changed |= render_query_expr_editor(
                        ui,
                        expression,
                        &format!("composition-query-{}", layer.id),
                        &membership_options,
                    );
                }
            });
        }

        if let Some((left, right)) = swap_composition_layer {
            self.state.composition_layers.swap(left, right);
            presentation_changed = true;
        }
        if let Some(index) = remove_composition_layer {
            self.state.composition_layers.remove(index);
            presentation_changed = true;
        }

        if ui.button("Add algebra layer").clicked() {
            self.state.composition_layers.push(CompositionLayer {
                id: Uuid::new_v4(),
                name: format!("Layer {}", self.state.composition_layers.len() + 1),
                enabled: false,
                operator: CompositionOperator::Union,
                saved_view_id: None,
                query: crate::query::EventQuery {
                    expression: Some(default_query_expr(QueryExprKind::Predicate)),
                    ..crate::query::EventQuery::default()
                },
            });
            presentation_changed = true;
        }

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
        ui.collapsing(
            format!("Table columns ({})", self.state.table_columns.len()),
            |ui| {
                ui.small("The ordered list is the visible Table schema for this view.");

                let mut remove_column = None;
                let mut swap_column = None;
                let column_count = self.state.table_columns.len();

                for (index, column) in self.state.table_columns.iter().copied().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(column.label());
                        if index > 0 && ui.small_button("↑").on_hover_text("Move left").clicked()
                        {
                            swap_column = Some((index, index - 1));
                        }
                        if index + 1 < column_count
                            && ui.small_button("↓").on_hover_text("Move right").clicked()
                        {
                            swap_column = Some((index, index + 1));
                        }
                        if column_count > 1
                            && ui.small_button("×").on_hover_text("Hide column").clicked()
                        {
                            remove_column = Some(index);
                        }
                    });
                }

                if let Some((left, right)) = swap_column {
                    self.state.table_columns.swap(left, right);
                    presentation_changed = true;
                }
                if let Some(index) = remove_column {
                    self.state.table_columns.remove(index);
                    presentation_changed = true;
                }

                ui.menu_button("Add column", |ui| {
                    for column in TableColumn::ALL {
                        if !self.state.table_columns.contains(&column)
                            && ui.button(column.label()).clicked()
                        {
                            self.state.table_columns.push(column);
                            presentation_changed = true;
                            ui.close();
                        }
                    }
                });

                if ui.button("Reset default columns").clicked() {
                    self.state.table_columns = crate::query::default_table_columns();
                    presentation_changed = true;
                }
            },
        );

        ui.separator();
        ui.strong("Sort rules");
        ui.add_enabled_ui(!ordering_locked, |ui| {
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
        });

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
            for source in &sources {
                ui.horizontal(|ui| {
                    let mut visible = !self.state.hidden_source_ids.contains(&source.id);
                    if ui.checkbox(&mut visible, "").changed() {
                        if visible {
                            self.state.hidden_source_ids.remove(&source.id);
                        } else {
                            self.state.hidden_source_ids.insert(source.id);
                        }
                        self.state.active_saved_view_id = None;
                        self.mark_state_dirty();
                    }

                    if ui
                        .selectable_label(self.selected_source_id == Some(source.id), &source.name)
                        .on_hover_text("Inspect source metadata")
                        .clicked()
                    {
                        self.selected_source_id = Some(source.id);
                    }
                });
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

            if let Some(source) = self
                .selected_source_id
                .and_then(|id| sources.iter().find(|source| source.id == id))
            {
                ui.separator();
                ui.strong("Source inspector");
                inspector_row(ui, "Name", &source.name);
                inspector_row(ui, "ID", &source.id.to_string());
                if let Some(external_ref) = source.external_ref.as_deref() {
                    inspector_row(ui, "External ref", external_ref);
                }
                if let Some(publisher) = source.publisher.as_deref() {
                    inspector_row(ui, "Publisher", publisher);
                }
                inspector_row(ui, "Kind", source.kind.as_str());
                inspector_row(ui, "Authority", source.authority.as_str());
                inspector_row(ui, "Enabled", if source.enabled { "yes" } else { "no" });
                inspector_row(ui, "Read only", if source.read_only { "yes" } else { "no" });
                if let Some(locator) = source.locator.as_deref() {
                    inspector_row(ui, "Locator", locator);
                }
                inspector_row(
                    ui,
                    "Canonical events",
                    &self
                        .source_event_counts
                        .get(&source.id)
                        .copied()
                        .unwrap_or_default()
                        .to_string(),
                );
                if source.kind == crate::domain::SourceKind::Taria {
                    let posture = if self.state.taria_last_release_id.is_none() {
                        "No adopted release"
                    } else if self.taria_current_source_ids.is_empty() {
                        "No current-release source links recorded"
                    } else if self.taria_current_source_ids.contains(&source.id) {
                        "Current adopted release"
                    } else {
                        "Historical / not in current release"
                    };
                    inspector_row(ui, "Release posture", posture);
                    if let Some(release_id) = self.state.taria_last_release_id.as_deref() {
                        inspector_row(ui, "Current release", release_id);
                    }
                }
                inspector_row(ui, "Created", &source.created_at.to_rfc3339());
                inspector_row(ui, "Last refreshed", &source.updated_at.to_rfc3339());
                if let Some(upstream_generated) = source
                    .properties
                    .get("taria")
                    .and_then(serde_json::Value::as_object)
                    .and_then(|taria| taria.get("generated_at"))
                    .and_then(serde_json::Value::as_str)
                {
                    inspector_row(ui, "Upstream generated", upstream_generated);
                }

                ui.collapsing("Properties", |ui| {
                    let pretty = serde_json::to_string_pretty(&source.properties)
                        .unwrap_or_else(|_| source.properties.to_string());
                    ui.code(pretty);
                });
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
                if self.state.calendar_layout == CalendarLayout::Density {
                    self.state.calendar_layout = CalendarLayout::Agenda;
                }
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
        self.poll_taria_workspace_update();
        if self.taria_update_receiver.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }

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
                                table_columns: &self.state.table_columns,
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
                changed |= render_query_predicate_editor(ui, predicate, path, membership_options);
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
                changed |=
                    render_query_expr_editor(ui, child, &format!("{path}.not"), membership_options);
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
                            .selectable_value(calendar_id, calendar.calendar_id.clone(), label)
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
    table_columns: &'a [TableColumn],
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

#[derive(Clone, Copy)]
struct TimelineRenderContext<'a> {
    view: CalendarView,
    focus: NaiveDate,
    timezone: Tz,
    monday_start: bool,
    selected: Option<Uuid>,
    colors: ColorPresentation<'a>,
}

#[derive(Clone, Copy)]
struct TableRenderContext<'a> {
    selected: Option<Uuid>,
    group_by: GroupBy,
    sort_rules: &'a [SortRule],
    table_columns: &'a [TableColumn],
    colors: ColorPresentation<'a>,
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
        table_columns,
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
        CalendarLayout::CompactAgenda => {
            render_compact_agenda(ui, events, timezone, selected, group_by, sort_rules, colors)
        }
        CalendarLayout::Stream => render_stream(ui, events, timezone, selected, colors),
        CalendarLayout::Timeline => render_timeline(
            ui,
            events,
            TimelineRenderContext {
                view,
                focus,
                timezone,
                monday_start,
                selected,
                colors,
            },
        ),
        CalendarLayout::Density => {
            render_density(ui, events, view, focus, timezone, monday_start)
        }
        CalendarLayout::Table => render_table(
            ui,
            events,
            timezone,
            TableRenderContext {
                selected,
                group_by,
                sort_rules,
                table_columns,
                colors,
            },
        ),
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

fn render_density(
    ui: &mut egui::Ui,
    events: &[TemporalEvent],
    view: CalendarView,
    focus: NaiveDate,
    timezone: Tz,
    monday_start: bool,
) -> Option<CalendarAction> {
    let window = window_for_view(view, focus, monday_start);
    let mut days = Vec::new();
    let mut cursor = window.start;

    while cursor < window.end_exclusive {
        let count = events
            .iter()
            .filter(|event| event.time.occurs_on(cursor, timezone))
            .count();
        days.push((cursor, count));
        cursor += chrono::Duration::days(1);
    }

    let max_count = days.iter().map(|(_, count)| *count).max().unwrap_or(0);
    let imprecise_count = events
        .iter()
        .filter(|event| matches!(event.time, TimeSpec::Month { .. } | TimeSpec::Year { .. }))
        .count();

    ui.horizontal_wrapped(|ui| {
        ui.strong("Density");
        ui.small(format!("{} positioned day(s)", days.len()));
        ui.small(format!("· peak {max_count} event(s)/day"));
        if imprecise_count > 0 {
            ui.small(format!(
                "· {imprecise_count} coarse-precision event(s) not assigned to fake days"
            ));
        }
    });
    ui.separator();

    let weekday_labels = if monday_start {
        ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
    } else {
        ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
    };

    let mut action = None;
    egui::Grid::new("density-grid")
        .num_columns(7)
        .spacing([4.0, 4.0])
        .show(ui, |ui| {
            for label in weekday_labels {
                ui.small(RichText::new(label).strong());
            }
            ui.end_row();

            let offset = if monday_start {
                window.start.weekday().num_days_from_monday()
            } else {
                window.start.weekday().num_days_from_sunday()
            };
            for _ in 0..offset {
                ui.label("");
            }
            let mut column = offset as usize;

            for (day, count) in &days {
                let fill = density_fill(ui, *count, max_count);
                let text = if *count == 0 {
                    day.day().to_string()
                } else {
                    format!("{}\n{}", day.day(), count)
                };
                let response = ui.add_sized(
                    [52.0, 42.0],
                    egui::Button::new(text)
                        .fill(fill)
                        .selected(false),
                );
                if response
                    .on_hover_text(format!("{} · {} event(s)", day, count))
                    .clicked()
                {
                    action = Some(CalendarAction::OpenDay(*day));
                }

                column += 1;
                if column % 7 == 0 {
                    ui.end_row();
                }
            }
        });

    action
}

fn density_fill(ui: &egui::Ui, count: usize, max_count: usize) -> Color32 {
    if count == 0 || max_count == 0 {
        return ui.visuals().extreme_bg_color;
    }

    let ratio = count as f32 / max_count as f32;
    ui.visuals()
        .selection
        .bg_fill
        .gamma_multiply(0.35 + ratio * 0.65)
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct TimelineSpan {
    start_fraction: f32,
    end_fraction: f32,
    point: bool,
}

fn render_timeline(
    ui: &mut egui::Ui,
    events: &[TemporalEvent],
    context: TimelineRenderContext<'_>,
) -> Option<CalendarAction> {
    let TimelineRenderContext {
        view,
        focus,
        timezone,
        monday_start,
        selected,
        colors,
    } = context;
    let window = window_for_view(view, focus, monday_start);
    let mut positioned = events
        .iter()
        .filter_map(|event| timeline_span(event, window, timezone).map(|span| (event, span)))
        .collect::<Vec<_>>();
    positioned.sort_by(|(left_event, left_span), (right_event, right_span)| {
        left_span
            .start_fraction
            .total_cmp(&right_span.start_fraction)
            .then_with(|| left_span.end_fraction.total_cmp(&right_span.end_fraction))
            .then_with(|| {
                left_event
                    .normalized_title
                    .cmp(&right_event.normalized_title)
            })
            .then_with(|| left_event.id.cmp(&right_event.id))
    });

    let unpositioned = events.len().saturating_sub(positioned.len());
    let axis_label = format!(
        "{} → {}",
        window.start,
        window.end_exclusive - chrono::Duration::days(1)
    );
    ui.horizontal_wrapped(|ui| {
        ui.strong("Timeline");
        ui.small(axis_label);
        if unpositioned > 0 {
            ui.small(format!(
                "· {unpositioned} unresolved event(s) not positioned"
            ));
        }
    });
    ui.separator();

    ui.horizontal(|ui| {
        ui.add_sized([220.0, 24.0], egui::Label::new(""));
        let desired = egui::vec2(ui.available_width().max(80.0), 24.0);
        let (rect, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        let y = rect.top() + 5.0;
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(1.0, Color32::GRAY),
        );

        for (fraction, label) in timeline_ticks(view, window) {
            let x = rect.left() + rect.width() * fraction.clamp(0.0, 1.0);
            painter.line_segment(
                [egui::pos2(x, y - 3.0), egui::pos2(x, y + 4.0)],
                egui::Stroke::new(1.0, Color32::GRAY),
            );

            let align = if fraction <= 0.01 {
                egui::Align2::LEFT_TOP
            } else if fraction >= 0.99 {
                egui::Align2::RIGHT_TOP
            } else {
                egui::Align2::CENTER_TOP
            };
            painter.text(
                egui::pos2(x, y + 6.0),
                align,
                label,
                egui::FontId::monospace(10.0),
                Color32::GRAY,
            );
        }
    });

    let mut action = None;
    for (event, span) in positioned {
        ui.horizontal(|ui| {
            let label = ui.add_sized(
                [220.0, 22.0],
                egui::Button::new(
                    RichText::new(&event.normalized_title)
                        .color(event_color(event, colors))
                        .strong(),
                )
                .selected(selected == Some(event.id)),
            );
            if label.clicked() {
                action = Some(CalendarAction::Select(event.id));
            }

            let desired = egui::vec2(ui.available_width().max(80.0), 22.0);
            let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());
            let painter = ui.painter_at(rect);
            let center_y = rect.center().y;

            painter.line_segment(
                [
                    egui::pos2(rect.left(), center_y),
                    egui::pos2(rect.right(), center_y),
                ],
                egui::Stroke::new(1.0, Color32::DARK_GRAY),
            );

            let x_for = |fraction: f32| rect.left() + rect.width() * fraction.clamp(0.0, 1.0);
            let start_x = x_for(span.start_fraction);
            let end_x = x_for(span.end_fraction);
            let color = event_color(event, colors);

            if span.point {
                painter.circle_filled(egui::pos2(start_x, center_y), 4.0, color);
            } else {
                let width = (end_x - start_x).abs().max(4.0);
                let bar = egui::Rect::from_min_size(
                    egui::pos2(start_x.min(end_x), center_y - 4.0),
                    egui::vec2(width, 8.0),
                );
                painter.rect_filled(bar, 2.0, color);
            }

            if response.clicked() {
                action = Some(CalendarAction::Select(event.id));
            }
            response.on_hover_text(format!(
                "{} · {}",
                event.display_time_label(timezone),
                event.status.as_str()
            ));
        });
    }

    action
}

fn timeline_ticks(view: CalendarView, window: crate::calendar::DateWindow) -> Vec<(f32, String)> {
    let total_days = (window.end_exclusive - window.start).num_days();
    if total_days <= 0 {
        return Vec::new();
    }

    let date_fraction = |date: NaiveDate| {
        ((date - window.start).num_days() as f32 / total_days as f32).clamp(0.0, 1.0)
    };

    match view {
        CalendarView::Year | CalendarView::Quarter => {
            let mut ticks = Vec::new();
            let mut year = window.start.year();
            let mut month = window.start.month();

            while let Some(date) = NaiveDate::from_ymd_opt(year, month, 1) {
                if date >= window.end_exclusive {
                    break;
                }
                ticks.push((date_fraction(date), date.format("%b").to_string()));

                if month == 12 {
                    month = 1;
                    year += 1;
                } else {
                    month += 1;
                }
            }
            ticks
        }
        CalendarView::Month => (0..5)
            .filter_map(|week| {
                let date = window
                    .start
                    .checked_add_signed(chrono::Duration::days(i64::from(week) * 7))?;
                (date < window.end_exclusive).then(|| (date_fraction(date), date.day().to_string()))
            })
            .collect(),
        CalendarView::Week => (0..7)
            .filter_map(|day| {
                let date = window
                    .start
                    .checked_add_signed(chrono::Duration::days(i64::from(day)))?;
                Some((date_fraction(date), date.format("%a %-d").to_string()))
            })
            .collect(),
        CalendarView::Day => vec![
            (0.0, "00:00".to_string()),
            (0.25, "06:00".to_string()),
            (0.5, "12:00".to_string()),
            (0.75, "18:00".to_string()),
            (1.0, "24:00".to_string()),
        ],
    }
}

fn timeline_span(
    event: &TemporalEvent,
    window: crate::calendar::DateWindow,
    timezone: Tz,
) -> Option<TimelineSpan> {
    let window_days = (window.end_exclusive - window.start).num_days();
    if window_days <= 0 {
        return None;
    }
    let total_seconds = window_days as f64 * 86_400.0;

    let date_fraction = |date: NaiveDate, seconds: u32| {
        let days = (date - window.start).num_days() as f64;
        (days * 86_400.0 + f64::from(seconds)) / total_seconds
    };

    let (raw_start, raw_end, point) = match event.time {
        TimeSpec::DateOnly {
            start,
            end_exclusive,
        }
        | TimeSpec::AllDay {
            start,
            end_exclusive,
        } => {
            let end = end_exclusive.unwrap_or(start + chrono::Duration::days(1));
            (date_fraction(start, 0), date_fraction(end, 0), false)
        }
        TimeSpec::Instant {
            start_utc, end_utc, ..
        } => {
            let start = start_utc.with_timezone(&timezone).naive_local();
            let end = end_utc
                .map(|value| value.with_timezone(&timezone).naive_local())
                .unwrap_or(start);
            (
                date_fraction(start.date(), start.time().num_seconds_from_midnight()),
                date_fraction(end.date(), end.time().num_seconds_from_midnight()),
                end_utc.is_none(),
            )
        }
        TimeSpec::Floating { start, end, .. } => {
            let point = end.is_none();
            let end = end.unwrap_or(start);
            (
                date_fraction(start.date(), start.time().num_seconds_from_midnight()),
                date_fraction(end.date(), end.time().num_seconds_from_midnight()),
                point,
            )
        }
        TimeSpec::Month { year, month } => {
            let start = NaiveDate::from_ymd_opt(year, month, 1)?;
            let end = if month == 12 {
                NaiveDate::from_ymd_opt(year + 1, 1, 1)?
            } else {
                NaiveDate::from_ymd_opt(year, month + 1, 1)?
            };
            (date_fraction(start, 0), date_fraction(end, 0), false)
        }
        TimeSpec::Year { year } => {
            let start = NaiveDate::from_ymd_opt(year, 1, 1)?;
            let end = NaiveDate::from_ymd_opt(year + 1, 1, 1)?;
            (date_fraction(start, 0), date_fraction(end, 0), false)
        }
        TimeSpec::Unknown { .. } => return None,
    };

    let outside_window = if point {
        raw_start < 0.0 || raw_start >= 1.0
    } else {
        raw_end <= 0.0 || raw_start >= 1.0
    };
    if outside_window {
        return None;
    }

    Some(TimelineSpan {
        start_fraction: raw_start.clamp(0.0, 1.0) as f32,
        end_fraction: raw_end.clamp(0.0, 1.0) as f32,
        point,
    })
}

fn render_stream(
    ui: &mut egui::Ui,
    events: &[TemporalEvent],
    timezone: Tz,
    selected: Option<Uuid>,
    colors: ColorPresentation<'_>,
) -> Option<CalendarAction> {
    let mut ordered = events.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| compare_stream_events(left, right, timezone));

    let mut action = None;
    let mut previous_marker: Option<String> = None;

    for event in ordered {
        let marker = date_group_label(event, timezone);
        if previous_marker.as_deref() != Some(marker.as_str()) {
            if previous_marker.is_some() {
                ui.add_space(6.0);
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new("●").color(Color32::GRAY));
                ui.strong(&marker);
            });
            previous_marker = Some(marker);
        }

        ui.horizontal_top(|ui| {
            ui.add_space(3.0);
            ui.label(RichText::new("│").monospace().color(Color32::DARK_GRAY));
            ui.vertical(|ui| {
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
                    ui.small(event.status.as_str());
                });

                let mut metadata = Vec::new();
                if let Some(event_type) = event.event_type.as_deref() {
                    metadata.push(event_type);
                }
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
        });
    }

    action
}

fn render_compact_agenda(
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

    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;

        for (group, group_events) in groups {
            if let Some(group) = group {
                ui.add_space(3.0);
                ui.small(RichText::new(group).strong());
            }

            for event in group_events {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [82.0, 18.0],
                        egui::Label::new(
                            RichText::new(table_date_label(event, timezone))
                                .monospace()
                                .color(Color32::GRAY),
                        ),
                    );
                    ui.add_sized(
                        [72.0, 18.0],
                        egui::Label::new(
                            RichText::new(event.display_time_label(timezone))
                                .monospace()
                                .color(Color32::GRAY),
                        ),
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
                });
            }
        }
    });

    action
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
    context: TableRenderContext<'_>,
) -> Option<CalendarAction> {
    let TableRenderContext {
        selected,
        group_by,
        sort_rules,
        table_columns,
        colors,
    } = context;
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
                for column in table_columns {
                    ui.strong(column.label());
                }
                ui.end_row();

                for event in group_events {
                    for column in table_columns {
                        let text = table_cell_text(event, timezone, *column);
                        let rich_text = match column {
                            TableColumn::Title => RichText::new(text)
                                .color(event_color(event, colors))
                                .strong(),
                            TableColumn::Date | TableColumn::Time => {
                                RichText::new(text).monospace()
                            }
                            _ => RichText::new(text),
                        };

                        if ui
                            .selectable_label(selected == Some(event.id), rich_text)
                            .clicked()
                        {
                            action = Some(CalendarAction::Select(event.id));
                        }
                    }
                    ui.end_row();
                }
            });

        ui.add_space(10.0);
    }

    action
}

fn table_cell_text(event: &TemporalEvent, timezone: Tz, column: TableColumn) -> String {
    match column {
        TableColumn::Date => table_date_label(event, timezone),
        TableColumn::Time => event.display_time_label(timezone),
        TableColumn::Title => event.normalized_title.clone(),
        TableColumn::EventType => event.event_type.clone().unwrap_or_else(|| "—".to_string()),
        TableColumn::Domain => event.domain.clone().unwrap_or_else(|| "—".to_string()),
        TableColumn::Jurisdiction => event
            .jurisdiction
            .clone()
            .unwrap_or_else(|| "—".to_string()),
        TableColumn::Institution => event.institution.clone().unwrap_or_else(|| "—".to_string()),
        TableColumn::Status => event.status.as_str().to_string(),
        TableColumn::Importance => event
            .importance
            .map(|value| value.to_string())
            .unwrap_or_else(|| "—".to_string()),
        TableColumn::PersonalRelevance => event
            .personal_relevance
            .map(|value| value.to_string())
            .unwrap_or_else(|| "—".to_string()),
        TableColumn::Source => event_source_key(event).unwrap_or_else(|| "—".to_string()),
        TableColumn::Renderability => event
            .renderability
            .clone()
            .unwrap_or_else(|| "—".to_string()),
        TableColumn::Confidence => event
            .confidence
            .map(|value| format!("{value:.2}"))
            .unwrap_or_else(|| "—".to_string()),
        TableColumn::Tags => {
            if event.tags.is_empty() {
                "—".to_string()
            } else {
                event.tags.join(", ")
            }
        }
        TableColumn::UpstreamEventRef => event
            .upstream_event_ref
            .clone()
            .unwrap_or_else(|| "—".to_string()),
        TableColumn::ReconciledEventRef => event
            .upstream_reconciled_key
            .clone()
            .unwrap_or_else(|| "—".to_string()),
    }
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

fn compare_stream_events(
    left: &TemporalEvent,
    right: &TemporalEvent,
    timezone: Tz,
) -> std::cmp::Ordering {
    agenda_sort_date(left, timezone)
        .cmp(&agenda_sort_date(right, timezone))
        .then_with(|| stream_precision_rank(&left.time).cmp(&stream_precision_rank(&right.time)))
        .then_with(|| {
            stream_clock_time(&left.time, timezone).cmp(&stream_clock_time(&right.time, timezone))
        })
        .then_with(|| left.normalized_title.cmp(&right.normalized_title))
        .then_with(|| left.id.cmp(&right.id))
}

const fn stream_precision_rank(time: &TimeSpec) -> u8 {
    match time {
        TimeSpec::Year { .. } | TimeSpec::Month { .. } => 0,
        TimeSpec::DateOnly { .. } | TimeSpec::AllDay { .. } => 1,
        TimeSpec::Instant { .. } | TimeSpec::Floating { .. } => 2,
        TimeSpec::Unknown { .. } => 3,
    }
}

fn stream_clock_time(time: &TimeSpec, timezone: Tz) -> Option<chrono::NaiveTime> {
    match time {
        TimeSpec::Instant { start_utc, .. } => Some(start_utc.with_timezone(&timezone).time()),
        TimeSpec::Floating { start, .. } => Some(start.time()),
        _ => None,
    }
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

fn taria_refresh_health(
    attempts: &[SourceRefreshAttempt],
    active_attempt_id: Option<Uuid>,
    has_adopted_release: bool,
    now: DateTime<Utc>,
) -> TariaRefreshHealth {
    if active_attempt_id.is_some() {
        return TariaRefreshHealth::Running;
    }

    let Some(latest) = attempts
        .iter()
        .find(|attempt| attempt.refresh_kind == "taria_workspace")
    else {
        return if has_adopted_release {
            TariaRefreshHealth::Unknown
        } else {
            TariaRefreshHealth::NeverRefreshed
        };
    };

    match latest.success {
        None => TariaRefreshHealth::Interrupted,
        Some(false) => TariaRefreshHealth::Failed,
        Some(true) => {
            let Some(completed_at) = latest.completed_at.as_deref() else {
                return TariaRefreshHealth::Unknown;
            };
            let Ok(completed_at) = DateTime::parse_from_rfc3339(completed_at) else {
                return TariaRefreshHealth::Unknown;
            };
            let completed_at = completed_at.with_timezone(&Utc);
            if now.signed_duration_since(completed_at)
                > ChronoDuration::hours(TARIA_STALE_AFTER_HOURS)
            {
                TariaRefreshHealth::Stale
            } else {
                TariaRefreshHealth::Healthy
            }
        }
    }
}

fn snapshot_time_label(raw: Option<&str>, timezone: Tz) -> String {
    let Some(raw) = raw else {
        return "—".to_string();
    };
    let Ok(time) = serde_json::from_str::<TimeSpec>(raw) else {
        return raw.to_string();
    };

    match time {
        TimeSpec::DateOnly {
            start,
            end_exclusive,
        } => end_exclusive.map_or_else(
            || start.to_string(),
            |end| format!("{start} → {end} exclusive"),
        ),
        TimeSpec::AllDay {
            start,
            end_exclusive,
        } => end_exclusive.map_or_else(
            || format!("{start} · all day"),
            |end| format!("{start} → {end} exclusive · all day"),
        ),
        TimeSpec::Instant {
            start_utc, end_utc, ..
        } => {
            let start = start_utc.with_timezone(&timezone).to_rfc3339();
            end_utc.map_or(start.clone(), |end| {
                format!("{start} → {}", end.with_timezone(&timezone).to_rfc3339())
            })
        }
        TimeSpec::Floating {
            start,
            end,
            source_timezone,
        } => {
            let suffix = source_timezone
                .as_deref()
                .map_or_else(|| "floating".to_string(), |zone| format!("local {zone}"));
            end.map_or_else(
                || format!("{start} · {suffix}"),
                |end| format!("{start} → {end} · {suffix}"),
            )
        }
        TimeSpec::Month { year, month } => format!("{year}-{month:02} · month precision"),
        TimeSpec::Year { year } => format!("{year} · year precision"),
        TimeSpec::Unknown { original_value } => original_value
            .map(|value| format!("{value} · unresolved"))
            .unwrap_or_else(|| "unresolved".to_string()),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn refresh_attempt(
        id: Uuid,
        success: Option<bool>,
        completed_at: Option<&str>,
    ) -> SourceRefreshAttempt {
        SourceRefreshAttempt {
            id,
            refresh_kind: "taria_workspace".to_string(),
            target: "bootstrap".to_string(),
            started_at: "2026-10-01T00:00:00Z".to_string(),
            completed_at: completed_at.map(ToOwned::to_owned),
            success,
            release_id: None,
            summary: None,
            error: None,
        }
    }

    #[test]
    fn timeline_ticks_follow_active_calendar_scale() {
        let year = crate::calendar::DateWindow {
            start: NaiveDate::from_ymd_opt(2026, 1, 1).expect("year start"),
            end_exclusive: NaiveDate::from_ymd_opt(2027, 1, 1).expect("year end"),
        };
        let year_ticks = timeline_ticks(CalendarView::Year, year);
        assert_eq!(year_ticks.len(), 12);
        assert_eq!(
            year_ticks.first().map(|(_, label)| label.as_str()),
            Some("Jan")
        );
        assert_eq!(
            year_ticks.last().map(|(_, label)| label.as_str()),
            Some("Dec")
        );

        let day = crate::calendar::DateWindow {
            start: NaiveDate::from_ymd_opt(2026, 10, 5).expect("day start"),
            end_exclusive: NaiveDate::from_ymd_opt(2026, 10, 6).expect("day end"),
        };
        let day_ticks = timeline_ticks(CalendarView::Day, day);
        assert_eq!(day_ticks.len(), 5);
        assert_eq!(day_ticks[2], (0.5, "12:00".to_string()));
    }

    #[test]
    fn timeline_span_positions_timed_and_day_precision_events() {
        let window = crate::calendar::DateWindow {
            start: NaiveDate::from_ymd_opt(2026, 10, 5).expect("start"),
            end_exclusive: NaiveDate::from_ymd_opt(2026, 10, 6).expect("end"),
        };

        let timed = TemporalEvent::new(
            "Noon",
            TimeSpec::Floating {
                start: window.start.and_hms_opt(12, 0, 0).expect("noon"),
                end: None,
                source_timezone: None,
            },
        );
        let timed_span = timeline_span(&timed, window, chrono_tz::UTC).expect("timed span");
        assert!(timed_span.point);
        assert!((timed_span.start_fraction - 0.5).abs() < 0.001);

        let all_day = TemporalEvent::new(
            "All day",
            TimeSpec::AllDay {
                start: window.start,
                end_exclusive: None,
            },
        );
        let day_span = timeline_span(&all_day, window, chrono_tz::UTC).expect("day span");
        assert!(!day_span.point);
        assert!((day_span.start_fraction - 0.0).abs() < 0.001);
        assert!((day_span.end_fraction - 1.0).abs() < 0.001);
    }

    #[test]
    fn timeline_span_preserves_month_precision_as_interval() {
        let window = crate::calendar::DateWindow {
            start: NaiveDate::from_ymd_opt(2026, 1, 1).expect("start"),
            end_exclusive: NaiveDate::from_ymd_opt(2027, 1, 1).expect("end"),
        };
        let event = TemporalEvent::new(
            "October",
            TimeSpec::Month {
                year: 2026,
                month: 10,
            },
        );

        let span = timeline_span(&event, window, chrono_tz::UTC).expect("month span");
        assert!(!span.point);
        assert!(span.end_fraction > span.start_fraction);
        assert!(span.start_fraction > 0.7);
        assert!(span.end_fraction < 0.95);
    }

    #[test]
    fn chronological_stream_orders_precision_and_clock_time_semantically() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");

        let month = TemporalEvent::new(
            "Month precision",
            TimeSpec::Month {
                year: 2026,
                month: 10,
            },
        );
        let date_only = TemporalEvent::new(
            "Date only",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let later = TemporalEvent::new(
            "Later",
            TimeSpec::Floating {
                start: day.and_hms_opt(14, 0, 0).expect("later"),
                end: None,
                source_timezone: None,
            },
        );
        let earlier = TemporalEvent::new(
            "Earlier",
            TimeSpec::Floating {
                start: day.and_hms_opt(9, 0, 0).expect("earlier"),
                end: None,
                source_timezone: None,
            },
        );

        let mut events = [later, date_only, earlier, month];
        events.sort_by(|left, right| compare_stream_events(left, right, chrono_tz::UTC));

        assert_eq!(
            events
                .iter()
                .map(|event| event.normalized_title.as_str())
                .collect::<Vec<_>>(),
            vec!["Month precision", "Date only", "Earlier", "Later"]
        );
    }

    #[test]
    fn taria_refresh_health_distinguishes_running_failure_interruption_and_staleness() {
        let now = DateTime::parse_from_rfc3339("2026-10-10T00:00:00Z")
            .expect("now")
            .with_timezone(&Utc);
        let active_id = Uuid::new_v4();

        assert_eq!(
            taria_refresh_health(&[], None, false, now),
            TariaRefreshHealth::NeverRefreshed
        );
        assert_eq!(
            taria_refresh_health(&[], None, true, now),
            TariaRefreshHealth::Unknown
        );
        assert_eq!(
            taria_refresh_health(&[], Some(active_id), false, now),
            TariaRefreshHealth::Running
        );
        assert_eq!(
            taria_refresh_health(
                &[refresh_attempt(
                    Uuid::new_v4(),
                    Some(false),
                    Some("2026-10-09T00:00:00Z")
                )],
                None,
                false,
                now,
            ),
            TariaRefreshHealth::Failed
        );
        assert_eq!(
            taria_refresh_health(
                &[refresh_attempt(Uuid::new_v4(), None, None)],
                None,
                false,
                now,
            ),
            TariaRefreshHealth::Interrupted
        );
        assert_eq!(
            taria_refresh_health(
                &[refresh_attempt(
                    Uuid::new_v4(),
                    Some(true),
                    Some("2026-10-09T00:00:00Z")
                )],
                None,
                false,
                now,
            ),
            TariaRefreshHealth::Healthy
        );
        assert_eq!(
            taria_refresh_health(
                &[refresh_attempt(
                    Uuid::new_v4(),
                    Some(true),
                    Some("2026-10-01T00:00:00Z")
                )],
                None,
                false,
                now,
            ),
            TariaRefreshHealth::Stale
        );
    }
}
