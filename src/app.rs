use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use chrono::{
    DateTime, Datelike, Duration as ChronoDuration, Local, NaiveDate, NaiveDateTime, NaiveTime,
    TimeZone, Timelike, Utc,
};
use chrono_tz::Tz;
use eframe::egui::{self, Color32, RichText};
use uuid::Uuid;

use crate::availability::{
    AlternativeSlots, BusyKind, FreeInterval, SlotSearch, alternative_days_for_candidate,
    alternative_slots_for_canceled_recurring_occurrence, alternative_slots_for_candidate,
    availability_for_materialized_date_window, conflicts_for_canceled_recurring_occurrence,
    conflicts_for_candidate_event, conflicts_for_recurring_occurrence, suggest_slots,
};
use crate::calendar::{
    CalendarLayout, CalendarView, calendar_title, month_days, month_grid_start, quarter_months,
    shift_focus, week_days, window_for_view, year_months,
};
use crate::csv::{export_source_csv_by_id, import_csv_file};
use crate::domain::{
    AvailabilityBehavior, CanonicalEntity, EventAnnotation, EventCollection, EventCollectionMember,
    EventIdentityAssessment, EventIdentityState, EventLocation, EventParticipant,
    EventProvenanceRecord, EventProvenanceRole, EventRelation, EventStatus, NotificationRule,
    NotificationTarget, RecurrenceFrequency, RecurrenceOccurrenceOrigin, RecurrenceOrdinalWeekday,
    RecurrenceOverride, RecurrenceRule, RecurrenceWeekday, TemporalEvent, TemporalSource, TimeSpec,
    TimeUncertainty,
};
use crate::ics::{IcsImportReport, export_ics_source_by_id, import_ics_file, import_remote_ics};
use crate::interchange::import_canonical_json_file;
use crate::notifications::{
    NotificationDelivery, NotificationOccurrence, NotificationSkip, evaluate_notification_rules,
};
use crate::query::{
    ColorBy, ColorRule, CompositionLayer, CompositionOperator, EventMembership, GroupBy,
    IntegerField, IntegerOperator, Overlay, PresenceField, QueryContext, QueryExpr, QueryPredicate,
    RelationDirection, RgbColor, SavedView, SortDirection, SortField, SortRule, TableColumn,
    TemporalKind, TextField, TextOperator,
    matches_composed_or_overlay_with_saved_views_and_membership, saved_view_reference_cycle,
};
use crate::scheduling::move_uncertain_placement;
use crate::state::PersistedUiState;
use crate::store::{
    EventRevision, ParticipantEntityResolution, SourceRefreshAttempt, TariaProjectedCalendarChoice,
    TariaReleaseDiff, TariaReleaseHistoryEntry, TariaReleaseStatusRecord, TemporalStore,
};
use crate::taria::import_reconciled_event_set_file;
use crate::taria_workspace::{
    TariaWorkspaceUpdateReport, detect_resourcearium_root, normalize_resourcearium_root,
    update_taria_sources as update_taria_workspace,
};

const TARIA_STALE_AFTER_HOURS: i64 = 7 * 24;

fn multiline_values(value: &str) -> Vec<String> {
    value
        .lines()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_new_local_event_time(
    draft: &NewLocalEventDraft,
    timezone: Tz,
) -> anyhow::Result<(TimeSpec, NaiveDate)> {
    let date = NaiveDate::parse_from_str(draft.date.trim(), "%Y-%m-%d")
        .map_err(|_| anyhow::anyhow!("date must use YYYY-MM-DD"))?;

    if draft.all_day {
        return Ok((
            TimeSpec::AllDay {
                start: date,
                end_exclusive: parse_optional_end_date(&draft.end_date, date)?,
            },
            date,
        ));
    }

    let time = NaiveTime::parse_from_str(draft.start_time.trim(), "%H:%M")
        .map_err(|_| anyhow::anyhow!("start time must use 24-hour HH:MM"))?;
    let duration_minutes = draft
        .duration_minutes
        .trim()
        .parse::<i64>()
        .map_err(|_| anyhow::anyhow!("duration must be a whole number of minutes"))?;
    if duration_minutes <= 0 {
        anyhow::bail!("duration must be greater than zero minutes");
    }

    let local = date.and_time(time);
    let start = match timezone.from_local_datetime(&local) {
        chrono::LocalResult::Single(value) => value,
        chrono::LocalResult::Ambiguous(_, _) => {
            anyhow::bail!("start time is ambiguous in {timezone}; choose an unambiguous local time")
        }
        chrono::LocalResult::None => {
            anyhow::bail!(
                "start time does not exist in {timezone} because of a timezone transition"
            )
        }
    };
    let start_utc = start.with_timezone(&Utc);
    let end_utc = start_utc
        .checked_add_signed(ChronoDuration::minutes(duration_minutes))
        .ok_or_else(|| anyhow::anyhow!("event duration overflows supported time range"))?;

    Ok((
        TimeSpec::Instant {
            start_utc,
            end_utc: Some(end_utc),
            source_timezone: Some(timezone.name().to_string()),
        },
        date,
    ))
}

fn new_local_event_from_draft(
    draft: &NewLocalEventDraft,
    timezone: Tz,
) -> anyhow::Result<(TemporalEvent, NaiveDate)> {
    let title = draft.title.trim();
    if title.is_empty() {
        anyhow::bail!("event title cannot be empty");
    }
    let (time, focus_date) = parse_new_local_event_time(draft, timezone)?;
    let mut event = TemporalEvent::new(title, time);
    event.description = optional_trimmed(&draft.description);
    event.event_type = optional_trimmed(&draft.event_type);
    event.domain = optional_trimmed(&draft.domain);
    event.status = draft.status;
    event.availability = draft.availability;
    event.participants = multiline_values(&draft.participant_names)
        .into_iter()
        .map(EventParticipant::new)
        .collect();

    let location = EventLocation {
        name: optional_trimmed(&draft.location_name),
        address: optional_trimmed(&draft.location_address),
        virtual_url: optional_trimmed(&draft.location_virtual_url),
        ..EventLocation::default()
    };
    if location.name.is_some() || location.address.is_some() || location.virtual_url.is_some() {
        location.validate()?;
        event.location = Some(location);
    }
    event.validate_participants()?;
    Ok((event, focus_date))
}

fn parse_slot_search(
    duration_minutes: &str,
    step_minutes: &str,
    day_start: &str,
    day_end: &str,
    workdays: [bool; 7],
) -> anyhow::Result<SlotSearch> {
    let duration_minutes = duration_minutes
        .trim()
        .parse::<u32>()
        .map_err(|_| anyhow::anyhow!("slot duration must be a positive whole number of minutes"))?;
    let step_minutes = step_minutes
        .trim()
        .parse::<u32>()
        .map_err(|_| anyhow::anyhow!("slot step must be a positive whole number of minutes"))?;
    let day_start = NaiveTime::parse_from_str(day_start.trim(), "%H:%M")
        .map_err(|_| anyhow::anyhow!("availability start must use HH:MM"))?;
    let day_end = NaiveTime::parse_from_str(day_end.trim(), "%H:%M")
        .map_err(|_| anyhow::anyhow!("availability end must use HH:MM"))?;

    SlotSearch {
        duration_minutes,
        step_minutes,
        day_start,
        day_end,
        workdays,
    }
    .validate()
}

fn new_event_draft_for_suggested_slot(
    interval: &FreeInterval,
    timezone: Tz,
) -> anyhow::Result<NewLocalEventDraft> {
    let mut draft = new_event_draft_for_free_interval(interval, timezone)?;
    let duration_minutes = (interval.end_utc - interval.start_utc).num_minutes();
    if duration_minutes <= 0 {
        anyhow::bail!("suggested slot has no positive duration");
    }
    draft.duration_minutes = duration_minutes.to_string();
    Ok(draft)
}

fn new_event_draft_for_free_interval(
    interval: &FreeInterval,
    timezone: Tz,
) -> anyhow::Result<NewLocalEventDraft> {
    let duration_minutes = (interval.end_utc - interval.start_utc).num_minutes();
    if duration_minutes <= 0 {
        anyhow::bail!("free interval has no positive duration");
    }
    let local_start = interval.start_utc.with_timezone(&timezone);
    let mut draft = NewLocalEventDraft::for_date(local_start.date_naive());
    draft.start_time = local_start.format("%H:%M").to_string();
    draft.duration_minutes = duration_minutes.min(60).to_string();
    Ok(draft)
}

fn format_availability_interval(
    start_utc: DateTime<Utc>,
    end_utc: DateTime<Utc>,
    timezone: Tz,
) -> String {
    let start = start_utc.with_timezone(&timezone);
    let end = end_utc.with_timezone(&timezone);
    if start.date_naive() == end.date_naive() {
        format!(
            "{} {}-{}",
            start.format("%a %b %-d"),
            start.format("%H:%M"),
            end.format("%H:%M")
        )
    } else {
        format!(
            "{} -> {}",
            start.format("%a %b %-d %H:%M"),
            end.format("%a %b %-d %H:%M")
        )
    }
}

fn new_conflict_confirmation_is_current(
    draft: &NewLocalEventDraft,
    candidate: &TemporalEvent,
    timezone: Tz,
) -> bool {
    draft
        .conflict_confirmation
        .as_ref()
        .is_some_and(|confirmation| confirmation.matches(candidate))
        && draft.status == candidate.status
        && draft.availability == candidate.availability
        && parse_new_local_event_time(draft, timezone).is_ok_and(|(time, _)| time == candidate.time)
}

fn time_conflict_confirmation_is_current(
    draft: &EventTimeEditDraft,
    candidate: &TemporalEvent,
    event_id: Uuid,
) -> bool {
    draft.event_id == event_id
        && draft
            .conflict_confirmation
            .as_ref()
            .is_some_and(|confirmation| confirmation.matches(candidate))
        && draft.parsed_time().is_ok_and(|time| time == candidate.time)
}

fn render_conflict_alternatives(
    ui: &mut egui::Ui,
    confirmation: &ConflictConfirmation,
    timezone: Tz,
) -> Option<FreeInterval> {
    if let Some(note) = confirmation.alternative_note.as_deref() {
        ui.small(note);
    }
    if confirmation.alternatives.is_empty() {
        return None;
    }

    let all_day = matches!(confirmation.time, TimeSpec::AllDay { .. });
    if all_day {
        ui.small("Try a later all-day start (full stored calendar, enabled weekdays):");
    } else {
        ui.small("Try a later open slot (full stored calendar, configured work hours):");
    }
    let mut selected = None;
    ui.horizontal_wrapped(|ui| {
        for slot in &confirmation.alternatives {
            let label = if all_day {
                format_civil_alternative(slot, timezone)
            } else {
                format_availability_interval(slot.start_utc, slot.end_utc, timezone)
            };
            if ui.small_button(label).clicked() {
                selected = Some(slot.clone());
            }
        }
    });
    selected
}

fn civil_alternative_dates(
    slot: &FreeInterval,
    timezone: Tz,
) -> anyhow::Result<(NaiveDate, NaiveDate)> {
    let local_start = slot.start_utc.with_timezone(&timezone);
    let local_end = slot.end_utc.with_timezone(&timezone);
    if local_start.time() != NaiveTime::MIN
        || local_end.time() != NaiveTime::MIN
        || local_end.date_naive() <= local_start.date_naive()
        || timezone
            .from_local_datetime(&local_start.naive_local())
            .single()
            .is_none_or(|value| value.with_timezone(&Utc) != slot.start_utc)
        || timezone
            .from_local_datetime(&local_end.naive_local())
            .single()
            .is_none_or(|value| value.with_timezone(&Utc) != slot.end_utc)
    {
        anyhow::bail!("all-day alternative has an ambiguous or non-midnight civil boundary");
    }
    Ok((local_start.date_naive(), local_end.date_naive()))
}

fn format_civil_alternative(slot: &FreeInterval, timezone: Tz) -> String {
    let Ok((start, end_exclusive)) = civil_alternative_dates(slot, timezone) else {
        return "Unrepresentable civil-day alternative".to_string();
    };
    let days = (end_exclusive - start).num_days();
    if days == 1 {
        format!("{} (all day)", start.format("%a %b %-d"))
    } else {
        format!("{} · {days} days", start.format("%a %b %-d"))
    }
}

fn apply_alternative_to_new_draft(
    draft: &mut NewLocalEventDraft,
    slot: &FreeInterval,
    timezone: Tz,
) -> anyhow::Result<()> {
    let mut next = draft.clone();
    if next.all_day {
        let (start, end) = civil_alternative_dates(slot, timezone)?;
        let (previous_time, _) = parse_new_local_event_time(&next, timezone)?;
        let TimeSpec::AllDay {
            start: previous_start,
            end_exclusive: previous_end,
        } = previous_time
        else {
            anyhow::bail!("quick-create no longer represents an all-day event");
        };
        let previous_duration = previous_end.map_or(1, |value| (value - previous_start).num_days());
        if previous_duration != (end - start).num_days() {
            anyhow::bail!("suggested all-day interval changes the original civil-day duration");
        }
        next.date = start.to_string();
        next.end_date = previous_end.map_or_else(String::new, |_| end.to_string());
        next.conflict_confirmation = None;
        let (time, _) = parse_new_local_event_time(&next, timezone)?;
        let expected = TimeSpec::AllDay {
            start,
            end_exclusive: previous_end.map(|_| end),
        };
        if time != expected {
            anyhow::bail!("suggested all-day interval cannot be represented by quick-create");
        }
        *draft = next;
        return Ok(());
    }
    let local = slot.start_utc.with_timezone(&timezone);
    next.date = local.date_naive().to_string();
    next.start_time = local.format("%H:%M").to_string();
    next.duration_minutes = (slot.end_utc - slot.start_utc).num_minutes().to_string();
    next.all_day = false;
    next.conflict_confirmation = None;

    let (time, _) = parse_new_local_event_time(&next, timezone)?;
    if !matches!(
        time,
        TimeSpec::Instant {
            start_utc,
            end_utc: Some(end_utc),
            ..
        } if start_utc == slot.start_utc && end_utc == slot.end_utc
    ) {
        anyhow::bail!("suggested slot cannot be represented exactly in the current timezone");
    }
    *draft = next;
    Ok(())
}

fn apply_alternative_to_time_draft(
    draft: &mut EventTimeEditDraft,
    slot: &FreeInterval,
    display_timezone: Tz,
) -> anyhow::Result<()> {
    if matches!(draft.kind, EventTimeEditKind::AllDay) {
        let (start, end_exclusive) = civil_alternative_dates(slot, display_timezone)?;
        let original = draft.parsed_time()?;
        let TimeSpec::AllDay {
            start: previous_start,
            end_exclusive: previous_end,
        } = original
        else {
            anyhow::bail!("all-day draft no longer represents an all-day event");
        };
        let previous_duration = previous_end.map_or(1, |end| (end - previous_start).num_days());
        if previous_duration != (end_exclusive - start).num_days() {
            anyhow::bail!("suggested all-day interval changes the original civil-day duration");
        }
        let mut next = draft.clone();
        next.date = start.to_string();
        next.end_date = if previous_end.is_some() {
            end_exclusive.to_string()
        } else {
            String::new()
        };
        next.conflict_confirmation = None;
        let expected = TimeSpec::AllDay {
            start,
            end_exclusive: previous_end.map(|_| end_exclusive),
        };
        if next.parsed_time()? != expected {
            anyhow::bail!("suggested all-day interval cannot be represented by this editor");
        }
        *draft = next;
        return Ok(());
    }

    let timezone = match &draft.kind {
        EventTimeEditKind::Instant { edit_timezone, .. } => *edit_timezone,
        EventTimeEditKind::Floating {
            source_timezone: Some(raw),
        } => raw
            .parse::<Tz>()
            .map_err(|_| anyhow::anyhow!("invalid floating source timezone {raw:?}"))?,
        EventTimeEditKind::Floating {
            source_timezone: None,
        } => display_timezone,
        EventTimeEditKind::AllDay | EventTimeEditKind::DateOnly => {
            anyhow::bail!("timed alternatives do not apply to date-only or all-day edits")
        }
    };

    let mut next = draft.clone();
    let local = slot.start_utc.with_timezone(&timezone);
    next.date = local.date_naive().to_string();
    next.start_time = local.format("%H:%M").to_string();
    next.duration_minutes = (slot.end_utc - slot.start_utc).num_minutes().to_string();
    next.conflict_confirmation = None;

    let exact = match next.parsed_time()? {
        TimeSpec::Instant {
            start_utc,
            end_utc: Some(end_utc),
            ..
        } => start_utc == slot.start_utc && end_utc == slot.end_utc,
        TimeSpec::Floating {
            start,
            end: Some(end),
            ..
        } => {
            let resolved_start = timezone.from_local_datetime(&start).single();
            let resolved_end = timezone.from_local_datetime(&end).single();
            resolved_start.is_some_and(|value| value.with_timezone(&Utc) == slot.start_utc)
                && resolved_end.is_some_and(|value| value.with_timezone(&Utc) == slot.end_utc)
        }
        _ => false,
    };
    if !exact {
        anyhow::bail!("suggested slot cannot be represented exactly in the source clock context");
    }
    *draft = next;
    Ok(())
}

/// Apply only a candidate that was computed for the unchanged focused series.
/// The original recurrence slot never moves: only the replacement start is
/// edited, and the resulting canonical interval must match the offered slot.
fn apply_recurring_alternative_to_draft(
    draft: &mut RecurrenceEditDraft,
    slot: &FreeInterval,
    display_timezone: Tz,
) -> anyhow::Result<()> {
    if !draft.alternative_slots.contains(slot)
        || draft.alternative_rule.as_ref() != draft.parsed_rule().ok().as_ref()
    {
        anyhow::bail!("the suggested opening is stale; search again");
    }
    let original = draft
        .focused_occurrence_original
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no original recurrence slot is selected"))?;
    let token = match &draft.base_time {
        TimeSpec::Instant { .. } => slot
            .start_utc
            .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true),
        TimeSpec::Floating {
            source_timezone, ..
        } => {
            let timezone = if let Some(raw) = source_timezone {
                raw.parse::<Tz>()
                    .map_err(|_| anyhow::anyhow!("invalid floating source timezone {raw:?}"))?
            } else {
                display_timezone
            };
            slot.start_utc
                .with_timezone(&timezone)
                .naive_local()
                .format("%Y-%m-%dT%H:%M:%S%.f")
                .to_string()
        }
        TimeSpec::AllDay { .. } => {
            let (date, _) = civil_alternative_dates(slot, display_timezone)?;
            date.to_string()
        }
        _ => anyhow::bail!("only exact, floating, and all-day occurrences can be moved"),
    };
    let replacement = parse_exception_start_value(&token, &draft.base_time, "Suggested occurrence")
        .map_err(anyhow::Error::msg)?;

    let matches_interval = match &replacement {
        TimeSpec::Instant {
            start_utc,
            end_utc: Some(end_utc),
            ..
        } => *start_utc == slot.start_utc && *end_utc == slot.end_utc,
        TimeSpec::Floating {
            start,
            end: Some(end),
            source_timezone,
        } => {
            let timezone = match source_timezone {
                Some(raw) => raw
                    .parse::<Tz>()
                    .map_err(|_| anyhow::anyhow!("invalid floating source timezone {raw:?}"))?,
                None => display_timezone,
            };
            let utc_start = timezone.from_local_datetime(start).single();
            let utc_end = timezone.from_local_datetime(end).single();
            utc_start.is_some_and(|value| value.with_timezone(&Utc) == slot.start_utc)
                && utc_end.is_some_and(|value| value.with_timezone(&Utc) == slot.end_utc)
        }
        TimeSpec::AllDay {
            start,
            end_exclusive,
        } => {
            let (date, end) = civil_alternative_dates(slot, display_timezone)?;
            *start == date
                && end_exclusive
                    .map_or_else(|| date.succ_opt() == Some(end), |actual| actual == end)
        }
        _ => false,
    };
    if !matches_interval {
        anyhow::bail!(
            "suggestion cannot preserve the original duration and clock semantics exactly"
        );
    }
    let mut next = draft.clone();
    let original_text = format_exception_start_value(original);
    if let Some(row) = next
        .override_rows
        .iter_mut()
        .find(|row| row.original_text == original_text)
    {
        row.action = RecurrenceOverrideEditAction::Move;
        row.replacement_text = token;
    } else {
        next.override_rows.push(RecurrenceOverrideEditRow {
            original_text,
            action: RecurrenceOverrideEditAction::Move,
            replacement_text: token,
        });
    }
    next.override_text = format_recurrence_override_edit_rows(&next.override_rows);
    next.parsed_rule().map_err(anyhow::Error::msg)?;
    next.alternative_slots.clear();
    next.alternative_rule = None;
    next.alternative_note = Some(
        "Replacement drafted for this occurrence only. Save to commit the override.".to_string(),
    );
    *draft = next;
    Ok(())
}

fn parse_notification_lead_minutes(value: &str) -> anyhow::Result<u32> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("reminder lead time is required");
    }
    value.parse::<u32>().map_err(|_| {
        anyhow::anyhow!("reminder lead time must be a non-negative whole number of minutes")
    })
}

fn parse_optional_confidence(value: &str) -> anyhow::Result<Option<f32>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let parsed = value
        .parse::<f32>()
        .map_err(|_| anyhow::anyhow!("confidence must be a number from 0 to 1"))?;
    if !parsed.is_finite() || !(0.0..=1.0).contains(&parsed) {
        anyhow::bail!("confidence must be finite and between 0 and 1");
    }
    Ok(Some(parsed))
}

fn parse_optional_i32(value: &str, label: &str) -> anyhow::Result<Option<i32>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    value
        .parse::<i32>()
        .map(Some)
        .map_err(|_| anyhow::anyhow!("{label} must be a whole number"))
}

fn optional_trimmed(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_snooze_minutes(raw: &str) -> anyhow::Result<i64> {
    let minutes = raw
        .trim()
        .parse::<u32>()
        .map_err(|_| anyhow::anyhow!("snooze minutes must be a whole number"))?;
    if !(1..=10_080).contains(&minutes) {
        anyhow::bail!("snooze minutes must be between 1 and 10080");
    }
    Ok(i64::from(minutes))
}

fn is_ics_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| {
            value.eq_ignore_ascii_case("ics") || value.eq_ignore_ascii_case("ical")
        })
}

fn is_canonical_json_path(path: &std::path::Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.ends_with(".ephemeris.json"))
}

fn is_csv_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("csv"))
}

fn default_csv_export_path(source: &TemporalSource) -> String {
    if source.kind == crate::domain::SourceKind::Csv
        && let Some(locator) = source.locator.as_deref()
    {
        let path = std::path::Path::new(locator);
        if let Some(stem) = path.file_stem().and_then(|value| value.to_str()) {
            let file_name = format!("{stem}-ephemeris.csv");
            if let Some(parent) = path.parent() {
                return parent.join(&file_name).display().to_string();
            }
            return file_name;
        }
    }

    format!("ephemeris-{}.csv", source.id)
}

fn default_ics_export_path(source: &TemporalSource) -> String {
    if source.kind == crate::domain::SourceKind::Ics
        && let Some(locator) = source.locator.as_deref()
    {
        let path = std::path::Path::new(locator);
        if let Some(stem) = path.file_stem().and_then(|value| value.to_str()) {
            let file_name = format!("{stem}-ephemeris.ics");
            if let Some(parent) = path.parent() {
                return parent.join(&file_name).display().to_string();
            }
            return file_name;
        }
    }

    format!("ephemeris-{}.ics", source.id)
}

fn remote_locator_display(locator: &str) -> String {
    let Some((scheme, rest)) = locator.split_once("://") else {
        return "remote calendar URL".to_string();
    };
    let authority = rest.split('/').next().unwrap_or(rest);
    if authority.is_empty() {
        "remote calendar URL".to_string()
    } else {
        format!("{scheme}://{authority}/…")
    }
}

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

#[derive(Debug, Clone)]
struct OccurrenceContext {
    event_id: Uuid,
    recurrence_index: Option<u32>,
    origin: RecurrenceOccurrenceOrigin,
    original_time: TimeSpec,
    override_applied: bool,
    cancelled_by_override: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecurrencePreset {
    Daily,
    Weekdays,
    Weekly,
    Monthly,
    LastWeekdayOfMonth,
    Yearly,
}

impl RecurrencePreset {
    const ALL: [Self; 6] = [
        Self::Daily,
        Self::Weekdays,
        Self::Weekly,
        Self::Monthly,
        Self::LastWeekdayOfMonth,
        Self::Yearly,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Daily => "Daily",
            Self::Weekdays => "Weekdays",
            Self::Weekly => "Weekly",
            Self::Monthly => "Monthly",
            Self::LastWeekdayOfMonth => "Last weekday/month",
            Self::Yearly => "Yearly",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecurrenceEditorSelector {
    WeekNo,
    YearDay,
    MonthDay,
    OrdinalByDay,
    Hour,
    Minute,
    Second,
}

impl RecurrenceEditorSelector {
    const fn unavailable_reason(self) -> &'static str {
        match self {
            Self::WeekNo => "BYWEEKNO is available only for YEARLY recurrence.",
            Self::YearDay => {
                "BYYEARDAY is available for SECONDLY, MINUTELY, HOURLY, and YEARLY recurrence."
            }
            Self::MonthDay => "BYMONTHDAY is unavailable for WEEKLY recurrence.",
            Self::OrdinalByDay => {
                "Ordinal BYDAY is available only for MONTHLY and YEARLY recurrence."
            }
            Self::Hour | Self::Minute | Self::Second => {
                "Time-of-day selectors require a floating or exact date-time series."
            }
        }
    }
}

fn recurrence_editor_base_is_datetime(base_time: &TimeSpec) -> bool {
    matches!(
        base_time,
        TimeSpec::Floating { .. } | TimeSpec::Instant { .. }
    )
}

fn recurrence_editor_frequency_available(
    frequency: RecurrenceFrequency,
    base_time: &TimeSpec,
) -> bool {
    recurrence_editor_base_is_datetime(base_time)
        || !matches!(
            frequency,
            RecurrenceFrequency::Secondly
                | RecurrenceFrequency::Minutely
                | RecurrenceFrequency::Hourly
        )
}

fn recurrence_editor_selector_available(
    selector: RecurrenceEditorSelector,
    frequency: RecurrenceFrequency,
    base_time: &TimeSpec,
) -> bool {
    match selector {
        RecurrenceEditorSelector::WeekNo => frequency == RecurrenceFrequency::Yearly,
        RecurrenceEditorSelector::YearDay => matches!(
            frequency,
            RecurrenceFrequency::Secondly
                | RecurrenceFrequency::Minutely
                | RecurrenceFrequency::Hourly
                | RecurrenceFrequency::Yearly
        ),
        RecurrenceEditorSelector::MonthDay => frequency != RecurrenceFrequency::Weekly,
        RecurrenceEditorSelector::OrdinalByDay => matches!(
            frequency,
            RecurrenceFrequency::Monthly | RecurrenceFrequency::Yearly
        ),
        RecurrenceEditorSelector::Hour
        | RecurrenceEditorSelector::Minute
        | RecurrenceEditorSelector::Second => recurrence_editor_base_is_datetime(base_time),
    }
}

fn recurrence_editor_week_start_available(
    frequency: RecurrenceFrequency,
    has_plain_byday: bool,
    has_week_no: bool,
) -> bool {
    (frequency == RecurrenceFrequency::Weekly && has_plain_byday)
        || (frequency == RecurrenceFrequency::Yearly && has_week_no)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecurrenceOverrideEditAction {
    Move,
    Cancel,
    CancelMove,
    Keep,
}

impl RecurrenceOverrideEditAction {
    const fn label(self) -> &'static str {
        match self {
            Self::Move => "Move",
            Self::Cancel => "Cancel",
            Self::CancelMove => "Cancel + move",
            Self::Keep => "Keep",
        }
    }

    const fn needs_replacement(self) -> bool {
        matches!(self, Self::Move | Self::CancelMove)
    }
}

fn recurrence_weekno_edit_rows(values: &[i8]) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}

fn format_recurrence_weekno_edit_rows(rows: &[String]) -> String {
    rows.iter()
        .map(|value| value.trim())
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_recurrence_weekno_edit_rows(raw: &str) -> Result<Vec<String>, String> {
    parse_i8_selector_values(raw, "BYWEEKNO")
        .map(|values| values.iter().map(ToString::to_string).collect())
}

fn validate_recurrence_weekno_edit_rows(rows: &[String]) -> Result<(), String> {
    for (index, value) in rows.iter().enumerate() {
        let raw = value.trim();
        if raw.is_empty() {
            return Err(format!(
                "BYWEEKNO row #{} needs a signed week number or must be removed.",
                index + 1
            ));
        }
        raw.parse::<i8>().map_err(|_| {
            format!(
                "BYWEEKNO row #{} has invalid signed week number '{}'.",
                index + 1,
                value
            )
        })?;
    }
    Ok(())
}

fn recurrence_yearday_edit_rows(values: &[i16]) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}

fn format_recurrence_yearday_edit_rows(rows: &[String]) -> String {
    rows.iter()
        .map(|value| value.trim())
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_recurrence_yearday_edit_rows(raw: &str) -> Result<Vec<String>, String> {
    parse_i16_selector_values(raw, "BYYEARDAY")
        .map(|values| values.iter().map(ToString::to_string).collect())
}

fn validate_recurrence_yearday_edit_rows(rows: &[String]) -> Result<(), String> {
    for (index, value) in rows.iter().enumerate() {
        let raw = value.trim();
        if raw.is_empty() {
            return Err(format!(
                "BYYEARDAY row #{} needs a signed year day or must be removed.",
                index + 1
            ));
        }
        raw.parse::<i16>().map_err(|_| {
            format!(
                "BYYEARDAY row #{} has invalid signed year day '{}'.",
                index + 1,
                value
            )
        })?;
    }
    Ok(())
}

fn recurrence_hour_edit_rows(values: &[u8]) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}

fn format_recurrence_hour_edit_rows(rows: &[String]) -> String {
    rows.iter()
        .map(|value| value.trim())
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_recurrence_hour_edit_rows(raw: &str) -> Result<Vec<String>, String> {
    parse_u8_selector_values(raw, "BYHOUR")
        .map(|values| values.iter().map(ToString::to_string).collect())
}

fn validate_recurrence_hour_edit_rows(rows: &[String]) -> Result<(), String> {
    for (index, value) in rows.iter().enumerate() {
        let raw = value.trim();
        if raw.is_empty() {
            return Err(format!(
                "BYHOUR row #{} needs an hour or must be removed.",
                index + 1
            ));
        }
        raw.parse::<u8>()
            .map_err(|_| format!("BYHOUR row #{} has invalid hour '{}'.", index + 1, value))?;
    }
    Ok(())
}

fn recurrence_minute_edit_rows(values: &[u8]) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}

fn format_recurrence_minute_edit_rows(rows: &[String]) -> String {
    rows.iter()
        .map(|value| value.trim())
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_recurrence_minute_edit_rows(raw: &str) -> Result<Vec<String>, String> {
    parse_u8_selector_values(raw, "BYMINUTE")
        .map(|values| values.iter().map(ToString::to_string).collect())
}

fn validate_recurrence_minute_edit_rows(rows: &[String]) -> Result<(), String> {
    for (index, value) in rows.iter().enumerate() {
        let raw = value.trim();
        if raw.is_empty() {
            return Err(format!(
                "BYMINUTE row #{} needs a minute or must be removed.",
                index + 1
            ));
        }
        raw.parse::<u8>().map_err(|_| {
            format!(
                "BYMINUTE row #{} has invalid minute '{}'.",
                index + 1,
                value
            )
        })?;
    }
    Ok(())
}

fn recurrence_second_edit_rows(values: &[u8]) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}

fn format_recurrence_second_edit_rows(rows: &[String]) -> String {
    rows.iter()
        .map(|value| value.trim())
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_recurrence_second_edit_rows(raw: &str) -> Result<Vec<String>, String> {
    parse_u8_selector_values(raw, "BYSECOND")
        .map(|values| values.iter().map(ToString::to_string).collect())
}

fn validate_recurrence_second_edit_rows(rows: &[String]) -> Result<(), String> {
    for (index, value) in rows.iter().enumerate() {
        let raw = value.trim();
        if raw.is_empty() {
            return Err(format!(
                "BYSECOND row #{} needs a second or must be removed.",
                index + 1
            ));
        }
        raw.parse::<u8>().map_err(|_| {
            format!(
                "BYSECOND row #{} has invalid second '{}'.",
                index + 1,
                value
            )
        })?;
    }
    Ok(())
}

fn recurrence_setpos_edit_rows(values: &[i16]) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}

fn format_recurrence_setpos_edit_rows(rows: &[String]) -> String {
    rows.iter()
        .map(|value| value.trim())
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_recurrence_setpos_edit_rows(raw: &str) -> Result<Vec<String>, String> {
    parse_i16_selector_values(raw, "BYSETPOS")
        .map(|values| values.iter().map(ToString::to_string).collect())
}

fn validate_recurrence_setpos_edit_rows(rows: &[String]) -> Result<(), String> {
    for (index, value) in rows.iter().enumerate() {
        let raw = value.trim();
        if raw.is_empty() {
            return Err(format!(
                "BYSETPOS row #{} needs a position or must be removed.",
                index + 1
            ));
        }
        raw.parse::<i16>().map_err(|_| {
            format!(
                "BYSETPOS row #{} has invalid position '{}'.",
                index + 1,
                value
            )
        })?;
    }
    Ok(())
}

fn recurrence_monthday_edit_rows(values: &[i8]) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}

fn format_recurrence_monthday_edit_rows(rows: &[String]) -> String {
    rows.iter()
        .map(|value| value.trim())
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_recurrence_monthday_edit_rows(raw: &str) -> Result<Vec<String>, String> {
    parse_i8_selector_values(raw, "BYMONTHDAY")
        .map(|values| values.iter().map(ToString::to_string).collect())
}

fn validate_recurrence_monthday_edit_rows(rows: &[String]) -> Result<(), String> {
    for (index, value) in rows.iter().enumerate() {
        let raw = value.trim();
        if raw.is_empty() {
            return Err(format!(
                "BYMONTHDAY row #{} needs a signed day or must be removed.",
                index + 1
            ));
        }
        raw.parse::<i8>().map_err(|_| {
            format!(
                "BYMONTHDAY row #{} has invalid signed day '{}'.",
                index + 1,
                value
            )
        })?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecurrenceOrdinalByDayEditRow {
    ordinal_text: String,
    weekday: RecurrenceWeekday,
}

impl RecurrenceOrdinalByDayEditRow {
    fn from_selector(selector: RecurrenceOrdinalWeekday) -> Self {
        Self {
            ordinal_text: selector.ordinal.to_string(),
            weekday: selector.weekday,
        }
    }

    fn compact_token(&self) -> String {
        format!(
            "{}{}",
            self.ordinal_text.trim(),
            recurrence_weekday_rrule_code(self.weekday)
        )
    }
}

fn recurrence_ordinal_byday_edit_rows(
    values: &[RecurrenceOrdinalWeekday],
) -> Vec<RecurrenceOrdinalByDayEditRow> {
    values
        .iter()
        .copied()
        .map(RecurrenceOrdinalByDayEditRow::from_selector)
        .collect()
}

fn format_recurrence_ordinal_byday_edit_rows(rows: &[RecurrenceOrdinalByDayEditRow]) -> String {
    rows.iter()
        .map(RecurrenceOrdinalByDayEditRow::compact_token)
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_recurrence_ordinal_byday_edit_rows(
    raw: &str,
) -> Result<Vec<RecurrenceOrdinalByDayEditRow>, String> {
    parse_ordinal_byday_values(raw).map(|values| recurrence_ordinal_byday_edit_rows(&values))
}

fn validate_recurrence_ordinal_byday_edit_rows(
    rows: &[RecurrenceOrdinalByDayEditRow],
) -> Result<(), String> {
    for (index, row) in rows.iter().enumerate() {
        let raw = row.ordinal_text.trim();
        if raw.is_empty() {
            return Err(format!(
                "Ordinal BYDAY row #{} needs an ordinal or must be removed.",
                index + 1
            ));
        }
        raw.parse::<i8>().map_err(|_| {
            format!(
                "Ordinal BYDAY row #{} has invalid ordinal '{}'.",
                index + 1,
                row.ordinal_text
            )
        })?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecurrenceOverrideEditRow {
    original_text: String,
    action: RecurrenceOverrideEditAction,
    replacement_text: String,
}

impl RecurrenceOverrideEditRow {
    fn from_override(occurrence_override: &RecurrenceOverride) -> Self {
        let action = match (
            occurrence_override.cancelled,
            occurrence_override.replacement.as_ref(),
        ) {
            (false, Some(_)) => RecurrenceOverrideEditAction::Move,
            (true, None) => RecurrenceOverrideEditAction::Cancel,
            (true, Some(_)) => RecurrenceOverrideEditAction::CancelMove,
            (false, None) => RecurrenceOverrideEditAction::Keep,
        };
        Self {
            original_text: format_exception_start_value(&occurrence_override.original),
            action,
            replacement_text: occurrence_override
                .replacement
                .as_ref()
                .map_or_else(String::new, format_exception_start_value),
        }
    }

    fn compact_entry(&self) -> String {
        let original = self.original_text.trim();
        match self.action {
            RecurrenceOverrideEditAction::Move => {
                format!("{original}=>{}", self.replacement_text.trim())
            }
            RecurrenceOverrideEditAction::Cancel => format!("{original}=>CANCEL"),
            RecurrenceOverrideEditAction::CancelMove => {
                format!("{original}=>CANCEL@{}", self.replacement_text.trim())
            }
            RecurrenceOverrideEditAction::Keep => format!("{original}=>KEEP"),
        }
    }
}

fn recurrence_exception_edit_rows(values: &[TimeSpec]) -> Vec<String> {
    values.iter().map(format_exception_start_value).collect()
}

fn format_recurrence_exception_edit_rows(rows: &[String]) -> String {
    rows.iter()
        .map(|value| value.trim())
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_recurrence_exception_edit_rows(
    raw: &str,
    base_time: &TimeSpec,
    label: &str,
) -> Result<Vec<String>, String> {
    parse_exception_start_values(raw, base_time, label)
        .map(|values| recurrence_exception_edit_rows(&values))
}

fn validate_recurrence_exception_edit_rows(rows: &[String], label: &str) -> Result<(), String> {
    if rows.iter().any(|value| value.trim().is_empty()) {
        Err(format!(
            "{label} rows must contain an occurrence start or be removed."
        ))
    } else {
        Ok(())
    }
}

fn recurrence_override_edit_rows(values: &[RecurrenceOverride]) -> Vec<RecurrenceOverrideEditRow> {
    values
        .iter()
        .map(RecurrenceOverrideEditRow::from_override)
        .collect()
}

fn format_recurrence_override_edit_rows(rows: &[RecurrenceOverrideEditRow]) -> String {
    rows.iter()
        .map(RecurrenceOverrideEditRow::compact_entry)
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_recurrence_override_edit_rows(
    raw: &str,
    base_time: &TimeSpec,
) -> Result<Vec<RecurrenceOverrideEditRow>, String> {
    parse_recurrence_override_values(raw, base_time)
        .map(|values| recurrence_override_edit_rows(&values))
}

#[derive(Debug, Clone)]
struct RecurrenceEditDraft {
    event_id: Uuid,
    had_recurrence: bool,
    rule: RecurrenceRule,
    interval_text: String,
    count_text: String,
    until_text: String,
    week_no_text: String,
    week_no_rows: Vec<String>,
    year_day_text: String,
    year_day_rows: Vec<String>,
    month_day_text: String,
    month_day_rows: Vec<String>,
    ordinal_byday_text: String,
    ordinal_byday_rows: Vec<RecurrenceOrdinalByDayEditRow>,
    hour_text: String,
    hour_rows: Vec<String>,
    minute_text: String,
    minute_rows: Vec<String>,
    second_text: String,
    second_rows: Vec<String>,
    set_pos_text: String,
    set_pos_rows: Vec<String>,
    rdate_text: String,
    rdate_rows: Vec<String>,
    exdate_text: String,
    exdate_rows: Vec<String>,
    override_text: String,
    override_rows: Vec<RecurrenceOverrideEditRow>,
    base_time: TimeSpec,
    focused_occurrence_original: Option<TimeSpec>,
    focused_occurrence_current: Option<TimeSpec>,
    draft_token: Uuid,
    alternative_slots: Vec<FreeInterval>,
    alternative_note: Option<String>,
    alternative_rule: Option<RecurrenceRule>,
    focused_conflict_confirmed_rule: Option<RecurrenceRule>,
    focused_conflict_warning: Option<String>,
}

impl RecurrenceEditDraft {
    fn from_event(event: &TemporalEvent) -> Self {
        let had_recurrence = event.recurrence.is_some();
        let rule = event
            .recurrence
            .clone()
            .unwrap_or_else(|| RecurrenceRule::new(RecurrenceFrequency::Daily));
        let rdate_text = format_exception_start_values(&rule.rdates);
        let rdate_rows = recurrence_exception_edit_rows(&rule.rdates);
        let exdate_text = format_exception_start_values(&rule.exdates);
        let exdate_rows = recurrence_exception_edit_rows(&rule.exdates);
        let override_text = format_recurrence_override_values(&rule.overrides);
        let override_rows = recurrence_override_edit_rows(&rule.overrides);
        let week_no_text = format_selector_values(&rule.by_week_no);
        let week_no_rows = recurrence_weekno_edit_rows(&rule.by_week_no);
        let year_day_text = format_selector_values(&rule.by_year_day);
        let year_day_rows = recurrence_yearday_edit_rows(&rule.by_year_day);
        let month_day_text = format_selector_values(&rule.by_month_day);
        let month_day_rows = recurrence_monthday_edit_rows(&rule.by_month_day);
        let hour_text = format_selector_values(&rule.by_hour);
        let hour_rows = recurrence_hour_edit_rows(&rule.by_hour);
        let minute_text = format_selector_values(&rule.by_minute);
        let minute_rows = recurrence_minute_edit_rows(&rule.by_minute);
        let second_text = format_selector_values(&rule.by_second);
        let second_rows = recurrence_second_edit_rows(&rule.by_second);
        let set_pos_text = format_selector_values(&rule.by_set_pos);
        let set_pos_rows = recurrence_setpos_edit_rows(&rule.by_set_pos);
        let ordinal_byday_text = format_ordinal_byday_values(&rule.by_month_weekday);
        let ordinal_byday_rows = recurrence_ordinal_byday_edit_rows(&rule.by_month_weekday);
        Self {
            event_id: event.id,
            had_recurrence,
            interval_text: rule.interval.to_string(),
            count_text: rule
                .count
                .map_or_else(String::new, |count| count.to_string()),
            until_text: rule
                .until
                .map_or_else(String::new, |until| until.to_string()),
            week_no_text,
            week_no_rows,
            year_day_text,
            year_day_rows,
            month_day_text,
            month_day_rows,
            ordinal_byday_text,
            ordinal_byday_rows,
            hour_text,
            hour_rows,
            minute_text,
            minute_rows,
            second_text,
            second_rows,
            set_pos_text,
            set_pos_rows,
            rdate_text,
            rdate_rows,
            exdate_text,
            exdate_rows,
            override_text,
            override_rows,
            base_time: event.time.clone(),
            focused_occurrence_original: None,
            focused_occurrence_current: None,
            draft_token: Uuid::new_v4(),
            alternative_slots: Vec::new(),
            alternative_note: None,
            alternative_rule: None,
            focused_conflict_confirmed_rule: None,
            focused_conflict_warning: None,
            rule,
        }
    }

    /// A focused editor is only a view over the existing series definition.
    /// Selecting an occurrence never modifies the recurrence or creates a
    /// phantom override; the user must explicitly choose Move or Cancel.
    fn focus_occurrence(&mut self, original: &TimeSpec) -> Result<(), String> {
        if !self.had_recurrence {
            return Err("this event does not have a recurrence series".to_string());
        }
        if original.kind_name() != self.base_time.kind_name() {
            return Err("occurrence time kind differs from the series".to_string());
        }
        self.focused_occurrence_original = Some(original.clone());
        self.focused_occurrence_current = Some(original.clone());
        self.alternative_slots.clear();
        self.alternative_note = None;
        self.alternative_rule = None;
        self.focused_conflict_confirmed_rule = None;
        self.focused_conflict_warning = None;
        Ok(())
    }

    fn apply_preset(&mut self, preset: RecurrencePreset) {
        self.interval_text = "1".to_string();
        self.rule.week_start = RecurrenceWeekday::Monday;
        self.rule.by_weekday.clear();
        self.rule.by_month.clear();
        self.week_no_text.clear();
        self.week_no_rows.clear();
        self.year_day_text.clear();
        self.year_day_rows.clear();
        self.month_day_text.clear();
        self.month_day_rows.clear();
        self.ordinal_byday_text.clear();
        self.ordinal_byday_rows.clear();
        self.hour_text.clear();
        self.hour_rows.clear();
        self.minute_text.clear();
        self.minute_rows.clear();
        self.second_text.clear();
        self.second_rows.clear();
        self.set_pos_text.clear();
        self.set_pos_rows.clear();

        match preset {
            RecurrencePreset::Daily => {
                self.rule.frequency = RecurrenceFrequency::Daily;
            }
            RecurrencePreset::Weekdays => {
                self.rule.frequency = RecurrenceFrequency::Daily;
                self.rule.by_weekday = vec![
                    RecurrenceWeekday::Monday,
                    RecurrenceWeekday::Tuesday,
                    RecurrenceWeekday::Wednesday,
                    RecurrenceWeekday::Thursday,
                    RecurrenceWeekday::Friday,
                ];
            }
            RecurrencePreset::Weekly => {
                self.rule.frequency = RecurrenceFrequency::Weekly;
            }
            RecurrencePreset::Monthly => {
                self.rule.frequency = RecurrenceFrequency::Monthly;
            }
            RecurrencePreset::LastWeekdayOfMonth => {
                self.rule.frequency = RecurrenceFrequency::Monthly;
                self.rule.by_weekday = vec![
                    RecurrenceWeekday::Monday,
                    RecurrenceWeekday::Tuesday,
                    RecurrenceWeekday::Wednesday,
                    RecurrenceWeekday::Thursday,
                    RecurrenceWeekday::Friday,
                ];
                self.set_pos_text = "-1".to_string();
                self.set_pos_rows = vec!["-1".to_string()];
            }
            RecurrencePreset::Yearly => {
                self.rule.frequency = RecurrenceFrequency::Yearly;
            }
        }
    }

    fn parsed_rule(&self) -> Result<RecurrenceRule, String> {
        let mut rule = self.rule.clone();
        rule.interval = self
            .interval_text
            .trim()
            .parse::<u32>()
            .map_err(|_| "Interval must be a positive integer.".to_string())?;
        rule.count = if self.count_text.trim().is_empty() {
            None
        } else {
            Some(
                self.count_text
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| "Count must be a positive integer or blank.".to_string())?,
            )
        };
        rule.until = if self.until_text.trim().is_empty() {
            None
        } else {
            Some(
                NaiveDate::parse_from_str(self.until_text.trim(), "%Y-%m-%d")
                    .map_err(|_| "Until must be YYYY-MM-DD or blank.".to_string())?,
            )
        };
        let canonical_week_no_text = format_selector_values(&self.rule.by_week_no);
        let structured_week_no_text = format_recurrence_weekno_edit_rows(&self.week_no_rows);
        let week_no_raw_override = self.week_no_text != canonical_week_no_text
            && self.week_no_text != structured_week_no_text;
        if week_no_raw_override {
            rule.by_week_no = parse_i8_selector_values(&self.week_no_text, "BYWEEKNO")?;
        } else {
            validate_recurrence_weekno_edit_rows(&self.week_no_rows)?;
            rule.by_week_no = parse_i8_selector_values(&structured_week_no_text, "BYWEEKNO")?;
        }

        let canonical_year_day_text = format_selector_values(&self.rule.by_year_day);
        let structured_year_day_text = format_recurrence_yearday_edit_rows(&self.year_day_rows);
        let year_day_raw_override = self.year_day_text != canonical_year_day_text
            && self.year_day_text != structured_year_day_text;
        if year_day_raw_override {
            rule.by_year_day = parse_i16_selector_values(&self.year_day_text, "BYYEARDAY")?;
        } else {
            validate_recurrence_yearday_edit_rows(&self.year_day_rows)?;
            rule.by_year_day = parse_i16_selector_values(&structured_year_day_text, "BYYEARDAY")?;
        }

        let canonical_month_day_text = format_selector_values(&self.rule.by_month_day);
        let structured_month_day_text = format_recurrence_monthday_edit_rows(&self.month_day_rows);
        let month_day_raw_override = self.month_day_text != canonical_month_day_text
            && self.month_day_text != structured_month_day_text;
        if month_day_raw_override {
            rule.by_month_day = parse_i8_selector_values(&self.month_day_text, "BYMONTHDAY")?;
        } else {
            validate_recurrence_monthday_edit_rows(&self.month_day_rows)?;
            rule.by_month_day = parse_i8_selector_values(&structured_month_day_text, "BYMONTHDAY")?;
        }

        let canonical_ordinal_byday_text = format_ordinal_byday_values(&self.rule.by_month_weekday);
        let structured_ordinal_byday_text =
            format_recurrence_ordinal_byday_edit_rows(&self.ordinal_byday_rows);
        let ordinal_byday_raw_override = self.ordinal_byday_text != canonical_ordinal_byday_text
            && self.ordinal_byday_text != structured_ordinal_byday_text;
        if ordinal_byday_raw_override {
            rule.by_month_weekday = parse_ordinal_byday_values(&self.ordinal_byday_text)?;
        } else {
            validate_recurrence_ordinal_byday_edit_rows(&self.ordinal_byday_rows)?;
            rule.by_month_weekday = parse_ordinal_byday_values(&structured_ordinal_byday_text)?;
        }

        let canonical_hour_text = format_selector_values(&self.rule.by_hour);
        let structured_hour_text = format_recurrence_hour_edit_rows(&self.hour_rows);
        let hour_raw_override =
            self.hour_text != canonical_hour_text && self.hour_text != structured_hour_text;
        if hour_raw_override {
            rule.by_hour = parse_u8_selector_values(&self.hour_text, "BYHOUR")?;
        } else {
            validate_recurrence_hour_edit_rows(&self.hour_rows)?;
            rule.by_hour = parse_u8_selector_values(&structured_hour_text, "BYHOUR")?;
        }

        let canonical_minute_text = format_selector_values(&self.rule.by_minute);
        let structured_minute_text = format_recurrence_minute_edit_rows(&self.minute_rows);
        let minute_raw_override =
            self.minute_text != canonical_minute_text && self.minute_text != structured_minute_text;
        if minute_raw_override {
            rule.by_minute = parse_u8_selector_values(&self.minute_text, "BYMINUTE")?;
        } else {
            validate_recurrence_minute_edit_rows(&self.minute_rows)?;
            rule.by_minute = parse_u8_selector_values(&structured_minute_text, "BYMINUTE")?;
        }

        let canonical_second_text = format_selector_values(&self.rule.by_second);
        let structured_second_text = format_recurrence_second_edit_rows(&self.second_rows);
        let second_raw_override =
            self.second_text != canonical_second_text && self.second_text != structured_second_text;
        if second_raw_override {
            rule.by_second = parse_u8_selector_values(&self.second_text, "BYSECOND")?;
        } else {
            validate_recurrence_second_edit_rows(&self.second_rows)?;
            rule.by_second = parse_u8_selector_values(&structured_second_text, "BYSECOND")?;
        }

        let canonical_set_pos_text = format_selector_values(&self.rule.by_set_pos);
        let structured_set_pos_text = format_recurrence_setpos_edit_rows(&self.set_pos_rows);
        let set_pos_raw_override = self.set_pos_text != canonical_set_pos_text
            && self.set_pos_text != structured_set_pos_text;
        if set_pos_raw_override {
            rule.by_set_pos = parse_i16_selector_values(&self.set_pos_text, "BYSETPOS")?;
        } else {
            validate_recurrence_setpos_edit_rows(&self.set_pos_rows)?;
            rule.by_set_pos = parse_i16_selector_values(&structured_set_pos_text, "BYSETPOS")?;
        }

        let canonical_rdate_text = format_exception_start_values(&self.rule.rdates);
        let structured_rdate_text = format_recurrence_exception_edit_rows(&self.rdate_rows);
        let rdate_raw_override =
            self.rdate_text != canonical_rdate_text && self.rdate_text != structured_rdate_text;
        if rdate_raw_override {
            rule.rdates = parse_exception_start_values(&self.rdate_text, &self.base_time, "RDATE")?;
        } else {
            validate_recurrence_exception_edit_rows(&self.rdate_rows, "RDATE")?;
            if structured_rdate_text != canonical_rdate_text {
                rule.rdates =
                    parse_exception_start_values(&structured_rdate_text, &self.base_time, "RDATE")?;
            }
        }

        let canonical_exdate_text = format_exception_start_values(&self.rule.exdates);
        let structured_exdate_text = format_recurrence_exception_edit_rows(&self.exdate_rows);
        let exdate_raw_override =
            self.exdate_text != canonical_exdate_text && self.exdate_text != structured_exdate_text;
        if exdate_raw_override {
            rule.exdates =
                parse_exception_start_values(&self.exdate_text, &self.base_time, "EXDATE")?;
        } else {
            validate_recurrence_exception_edit_rows(&self.exdate_rows, "EXDATE")?;
            if structured_exdate_text != canonical_exdate_text {
                rule.exdates = parse_exception_start_values(
                    &structured_exdate_text,
                    &self.base_time,
                    "EXDATE",
                )?;
            }
        }

        let canonical_override_text = format_recurrence_override_values(&self.rule.overrides);
        let structured_override_text = format_recurrence_override_edit_rows(&self.override_rows);
        let edited_override_text = if self.override_text != canonical_override_text
            && self.override_text != structured_override_text
        {
            &self.override_text
        } else {
            &structured_override_text
        };
        if edited_override_text != &canonical_override_text {
            rule.overrides =
                parse_recurrence_override_values(edited_override_text, &self.base_time)?;
        }

        rule.validate().map_err(|error| error.to_string())?;
        let mut validation_event =
            TemporalEvent::new("Recurrence editor validation", self.base_time.clone());
        validation_event.recurrence = Some(rule.clone());
        validation_event
            .validate_recurrence()
            .map_err(|error| error.to_string())?;
        Ok(rule)
    }
}

fn selector_tokens(raw: &str) -> impl Iterator<Item = &str> {
    raw.split(|character: char| character == ',' || character.is_whitespace())
        .filter(|token| !token.is_empty())
}

fn format_selector_values<T: ToString>(values: &[T]) -> String {
    values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn format_exception_start_value(value: &TimeSpec) -> String {
    match value {
        TimeSpec::DateOnly { start, .. } | TimeSpec::AllDay { start, .. } => start.to_string(),
        TimeSpec::Instant { start_utc, .. } => {
            start_utc.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
        }
        TimeSpec::Floating { start, .. } => {
            if start.nanosecond() == 0 {
                start.format("%Y-%m-%dT%H:%M:%S").to_string()
            } else {
                start.format("%Y-%m-%dT%H:%M:%S%.f").to_string()
            }
        }
        TimeSpec::Month { .. } | TimeSpec::Year { .. } | TimeSpec::Unknown { .. } => {
            format!("<unsupported:{}>", value.kind_name())
        }
    }
}

fn format_exception_start_values(values: &[TimeSpec]) -> String {
    values
        .iter()
        .map(format_exception_start_value)
        .collect::<Vec<_>>()
        .join(",")
}

fn format_recurrence_override_values(values: &[RecurrenceOverride]) -> String {
    values
        .iter()
        .map(|occurrence_override| {
            let original = format_exception_start_value(&occurrence_override.original);
            match (
                occurrence_override.cancelled,
                occurrence_override.replacement.as_ref(),
            ) {
                (false, Some(replacement)) => {
                    format!("{original}=>{}", format_exception_start_value(replacement))
                }
                (true, None) => format!("{original}=>CANCEL"),
                (true, Some(replacement)) => format!(
                    "{original}=>CANCEL@{}",
                    format_exception_start_value(replacement)
                ),
                (false, None) => format!("{original}=>KEEP"),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_floating_exception_start(raw: &str, label: &str) -> Result<NaiveDateTime, String> {
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
    ] {
        if let Ok(value) = NaiveDateTime::parse_from_str(raw, format) {
            return Ok(value);
        }
    }
    Err(format!(
        "{label} value '{raw}' must use YYYY-MM-DDTHH:MM[:SS[.fraction]]."
    ))
}

fn parse_exception_start_value(
    token: &str,
    base_time: &TimeSpec,
    label: &str,
) -> Result<TimeSpec, String> {
    match base_time {
        TimeSpec::DateOnly {
            start: base_start,
            end_exclusive,
        } => {
            let start = NaiveDate::parse_from_str(token, "%Y-%m-%d")
                .map_err(|_| format!("{label} value '{token}' must use YYYY-MM-DD."))?;
            let end_exclusive = (*end_exclusive)
                .map(|end| end.signed_duration_since(*base_start))
                .map(|duration| {
                    start
                        .checked_add_signed(duration)
                        .ok_or_else(|| format!("{label} value '{token}' overflows its duration."))
                })
                .transpose()?;
            Ok(TimeSpec::DateOnly {
                start,
                end_exclusive,
            })
        }
        TimeSpec::AllDay {
            start: base_start,
            end_exclusive,
        } => {
            let start = NaiveDate::parse_from_str(token, "%Y-%m-%d")
                .map_err(|_| format!("{label} value '{token}' must use YYYY-MM-DD."))?;
            let end_exclusive = (*end_exclusive)
                .map(|end| end.signed_duration_since(*base_start))
                .map(|duration| {
                    start
                        .checked_add_signed(duration)
                        .ok_or_else(|| format!("{label} value '{token}' overflows its duration."))
                })
                .transpose()?;
            Ok(TimeSpec::AllDay {
                start,
                end_exclusive,
            })
        }
        TimeSpec::Instant {
            start_utc: base_start,
            end_utc,
            source_timezone,
        } => {
            let start_utc = DateTime::parse_from_rfc3339(token)
                .map_err(|_| format!("{label} value '{token}' must be RFC3339."))?
                .with_timezone(&Utc);
            let end_utc = end_utc
                .as_ref()
                .map(|end| end.signed_duration_since(*base_start))
                .map(|duration| {
                    start_utc
                        .checked_add_signed(duration)
                        .ok_or_else(|| format!("{label} value '{token}' overflows its duration."))
                })
                .transpose()?;
            Ok(TimeSpec::Instant {
                start_utc,
                end_utc,
                source_timezone: source_timezone.clone(),
            })
        }
        TimeSpec::Floating {
            start: base_start,
            end,
            source_timezone,
        } => {
            let start = parse_floating_exception_start(token, label)?;
            let end = end
                .as_ref()
                .map(|end| end.signed_duration_since(*base_start))
                .map(|duration| {
                    start
                        .checked_add_signed(duration)
                        .ok_or_else(|| format!("{label} value '{token}' overflows its duration."))
                })
                .transpose()?;
            Ok(TimeSpec::Floating {
                start,
                end,
                source_timezone: source_timezone.clone(),
            })
        }
        TimeSpec::Month { .. } | TimeSpec::Year { .. } | TimeSpec::Unknown { .. } => Err(format!(
            "{label} cannot be authored for {} recurrence.",
            base_time.kind_name()
        )),
    }
}
fn parse_exception_start_values(
    raw: &str,
    base_time: &TimeSpec,
    label: &str,
) -> Result<Vec<TimeSpec>, String> {
    selector_tokens(raw)
        .map(|token| parse_exception_start_value(token, base_time, label))
        .collect()
}

fn override_entries(raw: &str) -> impl Iterator<Item = &str> {
    raw.split(['\n', ';'])
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
}

fn parse_recurrence_override_values(
    raw: &str,
    base_time: &TimeSpec,
) -> Result<Vec<RecurrenceOverride>, String> {
    override_entries(raw)
        .map(|entry| {
            let (original_raw, action_raw) = entry.split_once("=>").ok_or_else(|| {
                format!(
                    "Override '{entry}' must use original=>replacement, original=>CANCEL, or original=>KEEP."
                )
            })?;
            let original_raw = original_raw.trim();
            let action_raw = action_raw.trim();
            if original_raw.is_empty() || action_raw.is_empty() {
                return Err(format!("Override '{entry}' is missing an original or action."));
            }

            let original =
                parse_exception_start_value(original_raw, base_time, "Override original")?;

            if action_raw.eq_ignore_ascii_case("CANCEL") {
                return Ok(RecurrenceOverride {
                    original,
                    replacement: None,
                    cancelled: true,
                });
            }
            if action_raw.eq_ignore_ascii_case("KEEP") {
                return Ok(RecurrenceOverride {
                    original,
                    replacement: None,
                    cancelled: false,
                });
            }
            if let Some(replacement_raw) = action_raw
                .strip_prefix("CANCEL@")
                .or_else(|| action_raw.strip_prefix("cancel@"))
            {
                let replacement = parse_exception_start_value(
                    replacement_raw.trim(),
                    base_time,
                    "Override replacement",
                )?;
                return Ok(RecurrenceOverride {
                    original,
                    replacement: Some(replacement),
                    cancelled: true,
                });
            }

            let replacement =
                parse_exception_start_value(action_raw, base_time, "Override replacement")?;
            Ok(RecurrenceOverride {
                original,
                replacement: Some(replacement),
                cancelled: false,
            })
        })
        .collect()
}

fn recurrence_exception_value_hint(base_time: &TimeSpec) -> &'static str {
    match base_time {
        TimeSpec::DateOnly { .. } | TimeSpec::AllDay { .. } => "2026-03-10",
        TimeSpec::Floating { .. } => "2026-03-10T09:00:00",
        TimeSpec::Instant { .. } => "2026-03-10T15:00:00Z",
        TimeSpec::Month { .. } | TimeSpec::Year { .. } | TimeSpec::Unknown { .. } => "",
    }
}

fn recurrence_override_hint(base_time: &TimeSpec) -> String {
    let occurrence = recurrence_exception_value_hint(base_time);
    if occurrence.is_empty() {
        String::new()
    } else {
        format!("{occurrence}=>CANCEL")
    }
}

fn recurrence_exception_hint(base_time: &TimeSpec) -> &'static str {
    match base_time {
        TimeSpec::DateOnly { .. } | TimeSpec::AllDay { .. } => "2026-03-10,2026-04-15",
        TimeSpec::Floating { .. } => "2026-03-10T09:00:00",
        TimeSpec::Instant { .. } => "2026-03-10T15:00:00Z",
        TimeSpec::Month { .. } | TimeSpec::Year { .. } | TimeSpec::Unknown { .. } => "",
    }
}

fn recurrence_weekday_rrule_code(weekday: RecurrenceWeekday) -> &'static str {
    match weekday {
        RecurrenceWeekday::Monday => "MO",
        RecurrenceWeekday::Tuesday => "TU",
        RecurrenceWeekday::Wednesday => "WE",
        RecurrenceWeekday::Thursday => "TH",
        RecurrenceWeekday::Friday => "FR",
        RecurrenceWeekday::Saturday => "SA",
        RecurrenceWeekday::Sunday => "SU",
    }
}

fn format_ordinal_byday_values(values: &[RecurrenceOrdinalWeekday]) -> String {
    values
        .iter()
        .map(|selector| {
            format!(
                "{}{}",
                selector.ordinal,
                recurrence_weekday_rrule_code(selector.weekday)
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_i8_selector_values(raw: &str, label: &str) -> Result<Vec<i8>, String> {
    selector_tokens(raw)
        .map(|token| {
            token
                .parse::<i8>()
                .map_err(|_| format!("{label} contains invalid integer '{token}'."))
        })
        .collect()
}

fn parse_i16_selector_values(raw: &str, label: &str) -> Result<Vec<i16>, String> {
    selector_tokens(raw)
        .map(|token| {
            token
                .parse::<i16>()
                .map_err(|_| format!("{label} contains invalid integer '{token}'."))
        })
        .collect()
}

fn parse_u8_selector_values(raw: &str, label: &str) -> Result<Vec<u8>, String> {
    selector_tokens(raw)
        .map(|token| {
            token
                .parse::<u8>()
                .map_err(|_| format!("{label} contains invalid non-negative integer '{token}'."))
        })
        .collect()
}

fn parse_ordinal_byday_values(raw: &str) -> Result<Vec<RecurrenceOrdinalWeekday>, String> {
    selector_tokens(raw)
        .map(|token| {
            let weekday_start = token
                .find(|character: char| character.is_ascii_alphabetic())
                .ok_or_else(|| {
                    format!("Ordinal BYDAY token '{token}' must look like 1MO or -1FR.")
                })?;
            let (ordinal_raw, weekday_raw) = token.split_at(weekday_start);
            if ordinal_raw.is_empty() {
                return Err(format!(
                    "Ordinal BYDAY token '{token}' needs a numeric ordinal; use BYDAY checkboxes for plain weekdays."
                ));
            }
            let ordinal = ordinal_raw
                .parse::<i8>()
                .map_err(|_| format!("Ordinal BYDAY token '{token}' has an invalid ordinal."))?;
            let weekday = match weekday_raw.to_ascii_uppercase().as_str() {
                "MO" => RecurrenceWeekday::Monday,
                "TU" => RecurrenceWeekday::Tuesday,
                "WE" => RecurrenceWeekday::Wednesday,
                "TH" => RecurrenceWeekday::Thursday,
                "FR" => RecurrenceWeekday::Friday,
                "SA" => RecurrenceWeekday::Saturday,
                "SU" => RecurrenceWeekday::Sunday,
                _ => {
                    return Err(format!(
                        "Ordinal BYDAY token '{token}' has an invalid weekday."
                    ));
                }
            };
            Ok(RecurrenceOrdinalWeekday::new(ordinal, weekday))
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
struct ConflictConfirmation {
    time: TimeSpec,
    time_uncertainty: Option<TimeUncertainty>,
    availability: AvailabilityBehavior,
    status: EventStatus,
    warning: String,
    alternatives: Vec<FreeInterval>,
    alternative_note: Option<String>,
}

impl ConflictConfirmation {
    fn for_event(event: &TemporalEvent, warning: String) -> Self {
        Self {
            time: event.time.clone(),
            time_uncertainty: event.time_uncertainty.clone(),
            availability: event.availability,
            status: event.status,
            warning,
            alternatives: Vec::new(),
            alternative_note: None,
        }
    }

    fn matches(&self, event: &TemporalEvent) -> bool {
        self.time == event.time
            && self.time_uncertainty == event.time_uncertainty
            && self.availability == event.availability
            && self.status == event.status
    }
}

#[derive(Debug, Clone)]
struct NewLocalEventDraft {
    draft_token: Uuid,
    title: String,
    description: String,
    event_type: String,
    domain: String,
    participant_names: String,
    location_name: String,
    location_address: String,
    location_virtual_url: String,
    status: EventStatus,
    availability: AvailabilityBehavior,
    date: String,
    all_day: bool,
    start_time: String,
    duration_minutes: String,
    end_date: String,
    conflict_confirmation: Option<ConflictConfirmation>,
}

impl NewLocalEventDraft {
    fn for_date(date: NaiveDate) -> Self {
        Self {
            draft_token: Uuid::new_v4(),
            title: String::new(),
            description: String::new(),
            event_type: String::new(),
            domain: String::new(),
            participant_names: String::new(),
            location_name: String::new(),
            location_address: String::new(),
            location_virtual_url: String::new(),
            status: EventStatus::Scheduled,
            availability: AvailabilityBehavior::Busy,
            date: date.to_string(),
            all_day: false,
            start_time: "09:00".to_string(),
            duration_minutes: "60".to_string(),
            end_date: String::new(),
            conflict_confirmation: None,
        }
    }
}

#[derive(Debug, Clone)]
enum EventTimeEditKind {
    Instant {
        edit_timezone: Tz,
        source_timezone: Option<String>,
    },
    Floating {
        source_timezone: Option<String>,
    },
    AllDay,
    DateOnly,
}

impl EventTimeEditKind {
    const fn label(&self) -> &'static str {
        match self {
            Self::Instant { .. } => "Exact instant",
            Self::Floating { .. } => "Floating wall clock",
            Self::AllDay => "All day",
            Self::DateOnly => "Date only",
        }
    }
}

#[derive(Debug, Clone)]
struct EventTimeEditDraft {
    draft_token: Uuid,
    event_id: Uuid,
    kind: EventTimeEditKind,
    date: String,
    start_time: String,
    duration_minutes: String,
    end_date: String,
    conflict_confirmation: Option<ConflictConfirmation>,
    uncertain_origin: Option<(TimeSpec, TimeUncertainty)>,
}

impl EventTimeEditDraft {
    fn from_event(event: &TemporalEvent, display_timezone: Tz) -> anyhow::Result<Self> {
        if event.recurrence.is_some() {
            anyhow::bail!("recurring event master time editing is not supported yet");
        }
        if event.time_uncertainty.is_some() {
            event.validate_time_uncertainty()?;
            match &event.time {
                TimeSpec::Instant {
                    start_utc,
                    end_utc: Some(end_utc),
                    ..
                } if start_utc.second() == 0
                    && start_utc.timestamp_subsec_nanos() == 0
                    && end_utc.second() == 0
                    && end_utc.timestamp_subsec_nanos() == 0
                    && *end_utc > *start_utc => {}
                TimeSpec::Floating {
                    start,
                    end: Some(end),
                    ..
                } if start.second() == 0
                    && start.nanosecond() == 0
                    && end.second() == 0
                    && end.nanosecond() == 0
                    && *end > *start => {}
                TimeSpec::AllDay { .. } | TimeSpec::DateOnly { .. } => {}
                _ => anyhow::bail!(
                    "bounded uncertainty editing requires a whole-minute timed duration or a civil-date interval"
                ),
            }
        }

        match &event.time {
            TimeSpec::Instant {
                start_utc,
                end_utc,
                source_timezone,
            } => {
                let edit_timezone = match source_timezone.as_deref() {
                    Some(raw) => raw
                        .parse::<Tz>()
                        .map_err(|_| anyhow::anyhow!("invalid source timezone {raw:?}"))?,
                    None => display_timezone,
                };
                let local = start_utc.with_timezone(&edit_timezone);
                let duration_minutes = end_utc.map_or_else(String::new, |end| {
                    (end - *start_utc).num_minutes().to_string()
                });
                Ok(Self {
                    draft_token: Uuid::new_v4(),
                    event_id: event.id,
                    kind: EventTimeEditKind::Instant {
                        edit_timezone,
                        source_timezone: source_timezone.clone(),
                    },
                    date: local.date_naive().to_string(),
                    start_time: local.format("%H:%M").to_string(),
                    duration_minutes,
                    end_date: String::new(),
                    conflict_confirmation: None,
                    uncertain_origin: event
                        .time_uncertainty
                        .as_ref()
                        .map(|window| (event.time.clone(), window.clone())),
                })
            }
            TimeSpec::Floating {
                start,
                end,
                source_timezone,
            } => Ok(Self {
                draft_token: Uuid::new_v4(),
                event_id: event.id,
                kind: EventTimeEditKind::Floating {
                    source_timezone: source_timezone.clone(),
                },
                date: start.date().to_string(),
                start_time: start.format("%H:%M").to_string(),
                duration_minutes: end
                    .map_or_else(String::new, |end| (end - *start).num_minutes().to_string()),
                end_date: String::new(),
                conflict_confirmation: None,
                uncertain_origin: event
                    .time_uncertainty
                    .as_ref()
                    .map(|window| (event.time.clone(), window.clone())),
            }),
            TimeSpec::AllDay {
                start,
                end_exclusive,
            } => Ok(Self {
                draft_token: Uuid::new_v4(),
                event_id: event.id,
                kind: EventTimeEditKind::AllDay,
                date: start.to_string(),
                start_time: String::new(),
                duration_minutes: String::new(),
                end_date: end_exclusive.map_or_else(String::new, |value| value.to_string()),
                conflict_confirmation: None,
                uncertain_origin: event
                    .time_uncertainty
                    .as_ref()
                    .map(|window| (event.time.clone(), window.clone())),
            }),
            TimeSpec::DateOnly {
                start,
                end_exclusive,
            } => Ok(Self {
                draft_token: Uuid::new_v4(),
                event_id: event.id,
                kind: EventTimeEditKind::DateOnly,
                date: start.to_string(),
                start_time: String::new(),
                duration_minutes: String::new(),
                end_date: end_exclusive.map_or_else(String::new, |value| value.to_string()),
                conflict_confirmation: None,
                uncertain_origin: event
                    .time_uncertainty
                    .as_ref()
                    .map(|window| (event.time.clone(), window.clone())),
            }),
            TimeSpec::Month { .. } | TimeSpec::Year { .. } | TimeSpec::Unknown { .. } => {
                anyhow::bail!(
                    "{} precision is not editable through the scheduling time editor",
                    event.time.kind_name()
                )
            }
        }
    }

    fn parsed_time(&self) -> anyhow::Result<TimeSpec> {
        let date = NaiveDate::parse_from_str(self.date.trim(), "%Y-%m-%d")
            .map_err(|_| anyhow::anyhow!("date must use YYYY-MM-DD"))?;

        match &self.kind {
            EventTimeEditKind::Instant {
                edit_timezone,
                source_timezone,
            } => {
                let start_time = NaiveTime::parse_from_str(self.start_time.trim(), "%H:%M")
                    .map_err(|_| anyhow::anyhow!("start time must use 24-hour HH:MM"))?;
                let local = date.and_time(start_time);
                let start_utc = match edit_timezone.from_local_datetime(&local) {
                    chrono::LocalResult::Single(value) => value.with_timezone(&Utc),
                    chrono::LocalResult::Ambiguous(_, _) => {
                        anyhow::bail!(
                            "start time is ambiguous in {edit_timezone}; choose an unambiguous local time"
                        )
                    }
                    chrono::LocalResult::None => {
                        anyhow::bail!(
                            "start time does not exist in {edit_timezone} because of a timezone transition"
                        )
                    }
                };
                let end_utc = parse_optional_positive_duration(
                    &self.duration_minutes,
                    start_utc,
                    "duration",
                )?;
                Ok(TimeSpec::Instant {
                    start_utc,
                    end_utc,
                    source_timezone: source_timezone.clone(),
                })
            }
            EventTimeEditKind::Floating { source_timezone } => {
                let start_time = NaiveTime::parse_from_str(self.start_time.trim(), "%H:%M")
                    .map_err(|_| anyhow::anyhow!("start time must use 24-hour HH:MM"))?;
                let start = date.and_time(start_time);
                let end =
                    parse_optional_positive_duration(&self.duration_minutes, start, "duration")?;
                Ok(TimeSpec::Floating {
                    start,
                    end,
                    source_timezone: source_timezone.clone(),
                })
            }
            EventTimeEditKind::AllDay | EventTimeEditKind::DateOnly => {
                let end_exclusive = match self.uncertain_origin.as_ref() {
                    Some((
                        TimeSpec::AllDay {
                            start: original_start,
                            end_exclusive: original_end,
                        }
                        | TimeSpec::DateOnly {
                            start: original_start,
                            end_exclusive: original_end,
                        },
                        _,
                    )) => {
                        let displacement = date - *original_start;
                        original_end
                            .map(|value| {
                                value.checked_add_signed(displacement).ok_or_else(|| {
                                    anyhow::anyhow!(
                                        "shifted exclusive end date exceeds supported range"
                                    )
                                })
                            })
                            .transpose()?
                    }
                    _ => parse_optional_end_date(&self.end_date, date)?,
                };
                Ok(match self.kind {
                    EventTimeEditKind::AllDay => TimeSpec::AllDay {
                        start: date,
                        end_exclusive,
                    },
                    EventTimeEditKind::DateOnly => TimeSpec::DateOnly {
                        start: date,
                        end_exclusive,
                    },
                    _ => unreachable!("matched date-like time edit kind"),
                })
            }
        }
    }

    fn timezone_label(&self) -> Option<&str> {
        match &self.kind {
            EventTimeEditKind::Instant { edit_timezone, .. } => Some(edit_timezone.name()),
            EventTimeEditKind::Floating {
                source_timezone: Some(value),
            } => Some(value.as_str()),
            EventTimeEditKind::Floating {
                source_timezone: None,
            }
            | EventTimeEditKind::AllDay
            | EventTimeEditKind::DateOnly => None,
        }
    }

    const fn is_timed(&self) -> bool {
        matches!(
            self.kind,
            EventTimeEditKind::Instant { .. } | EventTimeEditKind::Floating { .. }
        )
    }
}

fn parse_optional_positive_duration<T>(
    raw: &str,
    start: T,
    label: &str,
) -> anyhow::Result<Option<T>>
where
    T: Copy + std::ops::Add<ChronoDuration, Output = T>,
{
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let minutes = raw
        .parse::<i64>()
        .map_err(|_| anyhow::anyhow!("{label} must be a whole number of minutes"))?;
    if minutes <= 0 {
        anyhow::bail!("{label} must be greater than zero minutes when present");
    }
    Ok(Some(start + ChronoDuration::minutes(minutes)))
}

fn parse_optional_end_date(raw: &str, start: NaiveDate) -> anyhow::Result<Option<NaiveDate>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let end = NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map_err(|_| anyhow::anyhow!("end date must use YYYY-MM-DD"))?;
    if end <= start {
        anyhow::bail!("end date must be later than the start date");
    }
    Ok(Some(end))
}

#[derive(Debug, Clone, Default)]
struct EventLocationEditDraft {
    event_id: Uuid,
    name: String,
    address: String,
    locality: String,
    region: String,
    postal_code: String,
    country: String,
    latitude: String,
    longitude: String,
    virtual_url: String,
}

impl EventLocationEditDraft {
    fn from_event(event: &TemporalEvent) -> Self {
        let location = event.location.as_ref();
        Self {
            event_id: event.id,
            name: location
                .and_then(|value| value.name.clone())
                .unwrap_or_default(),
            address: location
                .and_then(|value| value.address.clone())
                .unwrap_or_default(),
            locality: location
                .and_then(|value| value.locality.clone())
                .unwrap_or_default(),
            region: location
                .and_then(|value| value.region.clone())
                .unwrap_or_default(),
            postal_code: location
                .and_then(|value| value.postal_code.clone())
                .unwrap_or_default(),
            country: location
                .and_then(|value| value.country.clone())
                .unwrap_or_default(),
            latitude: location
                .and_then(|value| value.latitude)
                .map_or_else(String::new, |value| value.to_string()),
            longitude: location
                .and_then(|value| value.longitude)
                .map_or_else(String::new, |value| value.to_string()),
            virtual_url: location
                .and_then(|value| value.virtual_url.clone())
                .unwrap_or_default(),
        }
    }

    fn parsed_location(&self) -> anyhow::Result<Option<EventLocation>> {
        let latitude = parse_optional_f64(&self.latitude, "latitude")?;
        let longitude = parse_optional_f64(&self.longitude, "longitude")?;
        let location = EventLocation {
            name: optional_trimmed(&self.name),
            address: optional_trimmed(&self.address),
            locality: optional_trimmed(&self.locality),
            region: optional_trimmed(&self.region),
            postal_code: optional_trimmed(&self.postal_code),
            country: optional_trimmed(&self.country),
            latitude,
            longitude,
            virtual_url: optional_trimmed(&self.virtual_url),
        };

        let empty = location.text_values().next().is_none() && location.latitude.is_none();
        if empty {
            return Ok(None);
        }

        location.validate()?;
        Ok(Some(location))
    }
}

fn parse_optional_f64(raw: &str, label: &str) -> anyhow::Result<Option<f64>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let value = raw
        .parse::<f64>()
        .map_err(|_| anyhow::anyhow!("{label} must be a number"))?;
    if !value.is_finite() {
        anyhow::bail!("{label} must be finite");
    }
    Ok(Some(value))
}

#[derive(Debug, Clone)]
struct EventDetailsEditDraft {
    event_id: Uuid,
    title: String,
    description: String,
    event_type: String,
    domain: String,
    jurisdiction: String,
    institution: String,
    status: EventStatus,
    availability: AvailabilityBehavior,
    confidence: String,
    importance: String,
    personal_relevance: String,
    conflict_confirmation: Option<ConflictConfirmation>,
}

impl EventDetailsEditDraft {
    fn from_event(event: &TemporalEvent) -> Self {
        Self {
            event_id: event.id,
            title: event.normalized_title.clone(),
            description: event.description.clone().unwrap_or_default(),
            event_type: event.event_type.clone().unwrap_or_default(),
            domain: event.domain.clone().unwrap_or_default(),
            jurisdiction: event.jurisdiction.clone().unwrap_or_default(),
            institution: event.institution.clone().unwrap_or_default(),
            status: event.status,
            availability: event.availability,
            confidence: event
                .confidence
                .map_or_else(String::new, |value| value.to_string()),
            importance: event
                .importance
                .map_or_else(String::new, |value| value.to_string()),
            personal_relevance: event
                .personal_relevance
                .map_or_else(String::new, |value| value.to_string()),
            conflict_confirmation: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventDetailsEditorAction {
    Save,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventTimeEditorAction {
    Save,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventLocationEditorAction {
    Save,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RecurrenceEditorAction {
    Save,
    Cancel,
    Remove,
    FindAlternatives,
    UseAlternative(FreeInterval),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParticipantInspectorAction {
    Add,
    Remove(usize),
    BindEntity(usize, Uuid),
    UnbindEntity(usize),
    CreateEntity(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TopologyInspectorAction {
    AddToCollection(Uuid),
    RemoveFromCollection(Uuid),
    MoveCollectionMemberUp(Uuid),
    MoveCollectionMemberDown(Uuid),
    CreateCollection,
    SaveCollection(Uuid),
    DeleteCollection(Uuid),
    AddRelation,
    DeleteRelation(Uuid),
    SetIdentityState(Uuid, EventIdentityState),
    DeleteIdentityAssessment(Uuid),
    AddIdentityAssessment,
    AddAnnotation,
    DeleteAnnotation(Uuid),
    AddProvenance,
    DeleteProvenance(Uuid),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConflictAlternativeTarget {
    NewEvent(Uuid),
    TimeEdit(Uuid, Uuid),
}

type ConflictAlternativeWorker = (
    ConflictAlternativeTarget,
    TemporalEvent,
    Receiver<Result<AlternativeSlots, String>>,
);

struct RecurrenceAlternativeWorker {
    draft_token: Uuid,
    event_id: Uuid,
    original: TimeSpec,
    current: TimeSpec,
    rule: RecurrenceRule,
    receiver: Receiver<Result<AlternativeSlots, String>>,
}

pub struct EphemerisApp {
    store: TemporalStore,
    state: PersistedUiState,
    events: Vec<TemporalEvent>,
    occurrence_contexts: HashMap<Uuid, OccurrenceContext>,
    unplaced_events: Vec<TemporalEvent>,
    sources: Vec<TemporalSource>,
    source_event_counts: HashMap<Uuid, u64>,
    selected_source_id: Option<Uuid>,
    ics_export_source_id: Option<Uuid>,
    ics_export_path: String,
    csv_export_source_id: Option<Uuid>,
    csv_export_path: String,
    remote_ics_url: String,
    remote_ics_import_receiver: Option<Receiver<Result<IcsImportReport, String>>>,
    conflict_alternative_receiver: Option<ConflictAlternativeWorker>,
    recurrence_alternative_receiver: Option<RecurrenceAlternativeWorker>,
    taria_current_source_ids: BTreeSet<Uuid>,
    event_memberships: HashMap<Uuid, EventMembership>,
    canonical_entities: Vec<CanonicalEntity>,
    canonical_entity_usage: HashMap<Uuid, Vec<(Uuid, String)>>,
    event_collections: Vec<EventCollection>,
    event_collection_members: Vec<EventCollectionMember>,
    event_relations: Vec<EventRelation>,
    event_relation_titles: HashMap<Uuid, String>,
    event_relation_types: Vec<String>,
    event_identity_assessments: Vec<EventIdentityAssessment>,
    event_identity_titles: HashMap<Uuid, String>,
    event_annotations: Vec<EventAnnotation>,
    event_annotation_kinds: Vec<String>,
    annotation_new_kind: String,
    annotation_new_value: String,
    participant_new_name: String,
    participant_new_role: String,
    participant_new_type: String,
    participant_new_entity_ref: String,
    participant_entity_edit: Option<(Uuid, usize)>,
    participant_entity_search: String,
    participant_entity_candidates: Vec<(Uuid, String, Option<String>)>,
    participant_entity_target_id: Option<Uuid>,
    entity_registry_search: String,
    entity_manage_id: Option<Uuid>,
    entity_manage_name: String,
    entity_manage_type: String,
    entity_manage_aliases: String,
    entity_manage_external_refs: String,
    entity_merge_target_id: Option<Uuid>,
    event_provenance_records: Vec<EventProvenanceRecord>,
    event_revision_event_id: Option<Uuid>,
    event_revisions: Vec<EventRevision>,
    provenance_new_role: EventProvenanceRole,
    provenance_new_reference: String,
    provenance_new_note: String,
    identity_search: String,
    identity_candidates: Vec<(Uuid, String)>,
    identity_target_id: Option<Uuid>,
    identity_new_state: EventIdentityState,
    identity_confidence_text: String,
    identity_rationale: String,
    topology_edit_event_id: Option<Uuid>,
    topology_collection_choice: Option<Uuid>,
    topology_new_collection_name: String,
    topology_new_collection_ordered: bool,
    topology_manage_collection_id: Option<Uuid>,
    topology_manage_collection_name: String,
    topology_manage_collection_description: String,
    topology_manage_collection_ordered: bool,
    topology_relation_type: String,
    topology_relation_outgoing: bool,
    topology_relation_search: String,
    topology_relation_candidates: Vec<(Uuid, String)>,
    topology_relation_target_id: Option<Uuid>,
    taria_release_status: Option<TariaReleaseStatusRecord>,
    taria_release_history: Vec<TariaReleaseHistoryEntry>,
    taria_previous_release_diff: Option<TariaReleaseDiff>,
    taria_bundle_refs: Vec<String>,
    taria_calendar_choices: Vec<TariaProjectedCalendarChoice>,
    source_refresh_attempts: Vec<SourceRefreshAttempt>,
    taria_update_receiver: Option<(Uuid, Receiver<Result<TariaWorkspaceUpdateReport, String>>)>,
    last_message: Option<String>,
    last_error: Option<String>,
    new_local_event: Option<NewLocalEventDraft>,
    event_details_editor: Option<EventDetailsEditDraft>,
    event_time_editor: Option<EventTimeEditDraft>,
    event_location_editor: Option<EventLocationEditDraft>,
    recurrence_editor: Option<RecurrenceEditDraft>,
    notification_rules: Vec<NotificationRule>,
    notification_deliveries: Vec<NotificationDelivery>,
    notification_snoozed_deliveries: Vec<NotificationDelivery>,
    notification_occurrences: Vec<NotificationOccurrence>,
    notification_skipped: Vec<NotificationSkip>,
    notification_eval_minute: Option<i64>,
    notification_event_lead_minutes: String,
    notification_saved_view_id: Option<Uuid>,
    notification_saved_view_lead_minutes: String,
    notification_snooze_minutes: String,
    dirty_state: bool,
    saved_view_name: String,
    saved_views: Vec<SavedView>,
}

fn focused_recurrence_conflict_warning(
    store: &TemporalStore,
    timezone: Tz,
    event: &TemporalEvent,
    draft: &RecurrenceEditDraft,
    proposed_rule: &RecurrenceRule,
) -> anyhow::Result<Option<String>> {
    let Some(original) = draft.focused_occurrence_original.as_ref() else {
        return Ok(None);
    };
    let stored_rule = event
        .recurrence
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("focused event is no longer recurring"))?;
    if draft.rule != *stored_rule || draft.base_time != event.time {
        anyhow::bail!("recurrence series changed since the occurrence editor opened; reopen it");
    }
    let mut stored_rest = stored_rule.clone();
    stored_rest
        .overrides
        .retain(|override_value| override_value.original != *original);
    let mut proposed_rest = proposed_rule.clone();
    proposed_rest
        .overrides
        .retain(|override_value| override_value.original != *original);
    if stored_rest != proposed_rest {
        anyhow::bail!(
            "focused occurrence editing cannot change other recurrence slots or series selectors; use Edit series"
        );
    }

    let existing = stored_rule
        .overrides
        .iter()
        .find(|value| value.original == *original);
    let previous_active = !existing.is_some_and(|value| value.cancelled);
    let previous_time = existing
        .and_then(|value| value.replacement.as_ref())
        .unwrap_or(original);
    let next = proposed_rule
        .overrides
        .iter()
        .find(|value| value.original == *original);
    if next.is_some_and(|value| value.cancelled) || !event.availability.blocks_time() {
        return Ok(None);
    }
    let proposed_time = next
        .and_then(|value| value.replacement.as_ref())
        .unwrap_or(original);
    if previous_active && proposed_time == previous_time {
        return Ok(None);
    }
    let corpus = store.list_events()?;
    let check = if previous_active {
        conflicts_for_recurring_occurrence(
            &corpus,
            event.id,
            original,
            previous_time,
            proposed_time,
            timezone,
        )?
    } else {
        conflicts_for_canceled_recurring_occurrence(
            &corpus,
            event.id,
            original,
            proposed_time,
            timezone,
        )?
    };
    if check.conflicts.is_empty() && check.skipped.is_empty() {
        return Ok(None);
    }
    let mut names = check
        .conflicts
        .iter()
        .map(|value| value.event_title.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let extra = names.len().saturating_sub(4);
    names.truncate(4);
    let mut warning = if names.is_empty() {
        "The proposed recurring occurrence has provisional availability.".to_string()
    } else {
        format!("Recurring occurrence conflicts with {}.", names.join(", "))
    };
    if extra != 0 {
        warning.push_str(&format!(" +{extra} more."));
    }
    if !check.skipped.is_empty() {
        warning.push_str(&format!(
            " {} additional events could not be checked.",
            check.skipped.len()
        ));
    }
    warning.push_str(" Save again without changing the override to confirm.");
    Ok(Some(warning))
}

fn details_change_requires_provisional_uncertainty_confirmation(
    before: &TemporalEvent,
    after: &TemporalEvent,
) -> bool {
    let blocks = |event: &TemporalEvent| {
        event.availability.blocks_time()
            && !matches!(
                event.status,
                EventStatus::Cancelled | EventStatus::Postponed | EventStatus::Superseded
            )
    };
    !blocks(before)
        && blocks(after)
        && after.recurrence.is_none()
        && after.time_uncertainty.is_some()
}

fn details_change_requires_conflict_check(before: &TemporalEvent, after: &TemporalEvent) -> bool {
    let blocks = |event: &TemporalEvent| {
        event.availability.blocks_time()
            && !matches!(
                event.status,
                EventStatus::Cancelled | EventStatus::Postponed | EventStatus::Superseded
            )
    };
    !blocks(before)
        && blocks(after)
        && after.recurrence.is_none()
        && after.time_uncertainty.is_none()
        && matches!(
            after.time,
            TimeSpec::Instant { .. } | TimeSpec::Floating { .. } | TimeSpec::AllDay { .. }
        )
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
            occurrence_contexts: HashMap::new(),
            unplaced_events: Vec::new(),
            sources: Vec::new(),
            source_event_counts: HashMap::new(),
            selected_source_id: None,
            ics_export_source_id: None,
            ics_export_path: String::new(),
            csv_export_source_id: None,
            csv_export_path: String::new(),
            remote_ics_url: String::new(),
            remote_ics_import_receiver: None,
            conflict_alternative_receiver: None,
            recurrence_alternative_receiver: None,
            taria_current_source_ids: BTreeSet::new(),
            event_memberships: HashMap::new(),
            canonical_entities: Vec::new(),
            canonical_entity_usage: HashMap::new(),
            event_collections: Vec::new(),
            event_collection_members: Vec::new(),
            event_relations: Vec::new(),
            event_relation_titles: HashMap::new(),
            event_relation_types: Vec::new(),
            event_identity_assessments: Vec::new(),
            event_identity_titles: HashMap::new(),
            event_annotations: Vec::new(),
            event_annotation_kinds: Vec::new(),
            annotation_new_kind: "note".to_string(),
            annotation_new_value: String::new(),
            participant_new_name: String::new(),
            participant_new_role: String::new(),
            participant_new_type: String::new(),
            participant_new_entity_ref: String::new(),
            participant_entity_edit: None,
            participant_entity_search: String::new(),
            participant_entity_candidates: Vec::new(),
            participant_entity_target_id: None,
            entity_registry_search: String::new(),
            entity_manage_id: None,
            entity_manage_name: String::new(),
            entity_manage_type: String::new(),
            entity_manage_aliases: String::new(),
            entity_manage_external_refs: String::new(),
            entity_merge_target_id: None,
            event_provenance_records: Vec::new(),
            event_revision_event_id: None,
            event_revisions: Vec::new(),
            provenance_new_role: EventProvenanceRole::Provenance,
            provenance_new_reference: String::new(),
            provenance_new_note: String::new(),
            identity_search: String::new(),
            identity_candidates: Vec::new(),
            identity_target_id: None,
            identity_new_state: EventIdentityState::Candidate,
            identity_confidence_text: String::new(),
            identity_rationale: String::new(),
            topology_edit_event_id: None,
            topology_collection_choice: None,
            topology_new_collection_name: String::new(),
            topology_new_collection_ordered: false,
            topology_manage_collection_id: None,
            topology_manage_collection_name: String::new(),
            topology_manage_collection_description: String::new(),
            topology_manage_collection_ordered: false,
            topology_relation_type: String::new(),
            topology_relation_outgoing: true,
            topology_relation_search: String::new(),
            topology_relation_candidates: Vec::new(),
            topology_relation_target_id: None,
            taria_release_status: None,
            taria_release_history: Vec::new(),
            taria_previous_release_diff: None,
            taria_bundle_refs: Vec::new(),
            taria_calendar_choices: Vec::new(),
            source_refresh_attempts: Vec::new(),
            taria_update_receiver: None,
            last_message: None,
            last_error: None,
            new_local_event: None,
            event_details_editor: None,
            event_time_editor: None,
            event_location_editor: None,
            recurrence_editor: None,
            notification_rules: Vec::new(),
            notification_deliveries: Vec::new(),
            notification_snoozed_deliveries: Vec::new(),
            notification_occurrences: Vec::new(),
            notification_skipped: Vec::new(),
            notification_eval_minute: None,
            notification_event_lead_minutes: "15".to_string(),
            notification_saved_view_id: None,
            notification_saved_view_lead_minutes: "15".to_string(),
            notification_snooze_minutes: "15".to_string(),
            dirty_state: false,
            saved_view_name: String::new(),
            saved_views,
        };
        app.reload()?;
        Ok(app)
    }

    fn import_dropped_path(&mut self, path: &std::path::Path) {
        if is_canonical_json_path(path) {
            self.import_canonical_json_path(path);
        } else if is_csv_path(path) {
            self.import_csv_path(path);
        } else if is_ics_path(path) {
            self.import_ics_path(path);
        } else {
            self.import_taria_path(path);
        }
    }

    fn import_canonical_json_path(&mut self, path: &std::path::Path) {
        match import_canonical_json_file(&self.store, path) {
            Ok(report) => {
                self.last_message = Some(format!(
                    "Merged canonical snapshot: sources {} created/{} updated/{} unchanged; entities {} created/{} updated/{} unchanged; events {} created/{} updated/{} unchanged; relations {} created/{} updated/{} unchanged; collections {} created/{} updated/{} unchanged; memberships {} replaced/{} unchanged; identity {} created/{} updated/{} unchanged; annotations {} created/{} updated/{} unchanged; provenance {} created/{} updated/{} unchanged; entity bindings {} created/{} updated/{} unchanged",
                    report.sources_created,
                    report.sources_updated,
                    report.sources_unchanged,
                    report.entities_created,
                    report.entities_updated,
                    report.entities_unchanged,
                    report.events_created,
                    report.events_updated,
                    report.events_unchanged,
                    report.relations_created,
                    report.relations_updated,
                    report.relations_unchanged,
                    report.collections_created,
                    report.collections_updated,
                    report.collections_unchanged,
                    report.collection_memberships_replaced,
                    report.collection_memberships_unchanged,
                    report.identity_assessments_created,
                    report.identity_assessments_updated,
                    report.identity_assessments_unchanged,
                    report.annotations_created,
                    report.annotations_updated,
                    report.annotations_unchanged,
                    report.provenance_records_created,
                    report.provenance_records_updated,
                    report.provenance_records_unchanged,
                    report.participant_entity_bindings_created,
                    report.participant_entity_bindings_updated,
                    report.participant_entity_bindings_unchanged
                ));
                self.last_error = None;
                self.reload_or_report();
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!(
                    "Failed to import canonical snapshot {}: {error:#}",
                    path.display()
                ));
            }
        }
    }

    fn import_csv_path(&mut self, path: &std::path::Path) {
        match import_csv_file(&self.store, path) {
            Ok(report) => {
                self.selected_source_id = Some(report.source_id);
                self.state.show_sources = true;
                self.last_message = Some(format!(
                    "Imported {}: {} created, {} updated, {} unchanged, {} retained missing",
                    report.source_name,
                    report.created,
                    report.updated,
                    report.unchanged,
                    report.retained_missing
                ));
                self.last_error = None;
                self.mark_state_dirty();
                self.reload_or_report();
            }
            Err(error) => {
                self.source_refresh_attempts =
                    self.store.source_refresh_attempts(20).unwrap_or_default();
                self.last_message = None;
                self.last_error = Some(format!(
                    "Failed to import CSV {}: {error:#}",
                    path.display()
                ));
            }
        }
    }

    fn import_ics_path(&mut self, path: &std::path::Path) {
        match import_ics_file(&self.store, path) {
            Ok(report) => {
                self.selected_source_id = Some(report.source_id);
                self.state.show_sources = true;
                self.last_message = Some(format!(
                    "Imported {}: {} created, {} updated, {} unchanged, {} retained missing",
                    report.source_name,
                    report.created,
                    report.updated,
                    report.unchanged,
                    report.retained_missing
                ));
                self.last_error = None;
                self.mark_state_dirty();
                self.reload_or_report();
            }
            Err(error) => {
                self.source_refresh_attempts =
                    self.store.source_refresh_attempts(20).unwrap_or_default();
                self.last_message = None;
                self.last_error = Some(format!(
                    "Failed to import iCalendar {}: {error:#}",
                    path.display()
                ));
            }
        }
    }

    fn export_ics_source_to_path(&mut self, source_id: Uuid) {
        let output = self.ics_export_path.trim();
        if output.is_empty() {
            self.last_message = None;
            self.last_error = Some("Choose an output path before exporting iCalendar.".to_string());
            return;
        }

        match export_ics_source_by_id(&self.store, source_id, output) {
            Ok(report) => {
                self.last_message = Some(format!(
                    "Exported {} events to {}",
                    report.total_events,
                    report.output_path.display()
                ));
                self.last_error = None;
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to export iCalendar: {error:#}"));
            }
        }
    }

    fn export_csv_source_to_path(&mut self, source_id: Uuid) {
        let output = self.csv_export_path.trim();
        if output.is_empty() {
            self.last_message = None;
            self.last_error = Some("Choose an output path before exporting CSV.".to_string());
            return;
        }

        match export_source_csv_by_id(&self.store, source_id, output) {
            Ok(report) => {
                self.last_message = Some(format!(
                    "Exported {} CSV events to {}",
                    report.total_events,
                    report.output_path.display()
                ));
                self.last_error = None;
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to export CSV: {error:#}"));
            }
        }
    }

    fn start_remote_ics_import(&mut self, url: String) {
        if self.remote_ics_import_receiver.is_some() {
            return;
        }
        let url = url.trim().to_string();
        if url.is_empty() {
            self.last_message = None;
            self.last_error = Some("Enter a remote iCalendar URL first.".to_string());
            return;
        }

        let Some(database_path) = self.store.path().map(std::path::Path::to_path_buf) else {
            self.last_message = None;
            self.last_error = Some(
                "Remote iCalendar import requires a file-backed Ephemeris database.".to_string(),
            );
            return;
        };

        let (sender, receiver) = mpsc::channel();
        self.remote_ics_import_receiver = Some(receiver);
        self.last_message = Some("Fetching remote iCalendar source...".to_string());
        self.last_error = None;

        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<IcsImportReport> {
                let worker_store = TemporalStore::open(database_path)?;
                import_remote_ics(&worker_store, &url)
            })()
            .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
        });
    }

    fn poll_remote_ics_import(&mut self) {
        let result = match self.remote_ics_import_receiver.as_ref() {
            Some(receiver) => match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(
                    "Remote iCalendar worker exited without returning a result.".to_string(),
                )),
            },
            None => None,
        };

        let Some(result) = result else {
            return;
        };
        self.remote_ics_import_receiver = None;

        match result {
            Ok(report) => {
                self.selected_source_id = Some(report.source_id);
                self.state.show_sources = true;
                self.last_message = Some(if report.not_modified {
                    format!(
                        "{} is unchanged · {} events · HTTP 304",
                        report.source_name, report.total_events
                    )
                } else {
                    format!(
                        "Imported {}: {} created, {} updated, {} unchanged, {} retained missing",
                        report.source_name,
                        report.created,
                        report.updated,
                        report.unchanged,
                        report.retained_missing
                    )
                });
                self.last_error = None;
                self.reload_or_report();
            }
            Err(error) => {
                self.source_refresh_attempts =
                    self.store.source_refresh_attempts(20).unwrap_or_default();
                self.last_message = None;
                self.last_error = Some(format!("Failed to import remote iCalendar: {error}"));
            }
        }
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

        let taria_attempts = self
            .source_refresh_attempts
            .iter()
            .filter(|attempt| attempt.refresh_kind == "taria_workspace")
            .collect::<Vec<_>>();
        ui.collapsing(
            format!("Taria refresh history ({})", taria_attempts.len()),
            |ui| {
                if taria_attempts.is_empty() {
                    ui.small("No persisted Taria refresh attempts yet.");
                }

                for attempt in taria_attempts {
                    let status = match attempt.success {
                        Some(true) => "success",
                        Some(false) => "failed",
                        None if active_attempt_id == Some(attempt.id) => "running",
                        None => "incomplete",
                    };
                    ui.strong(format!("{status} · {}", attempt.target));
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

    fn canonical_event_id(&self, event_id: Uuid) -> Uuid {
        self.occurrence_contexts
            .get(&event_id)
            .map_or(event_id, |occurrence| occurrence.event_id)
    }

    fn event_is_editable(&self, event: &TemporalEvent) -> bool {
        event.source_id.is_none_or(|source_id| {
            self.sources
                .iter()
                .find(|source| source.id == source_id)
                .is_none_or(|source| !source.read_only)
        })
    }

    fn scheduling_conflict_warning(
        &self,
        candidate: &TemporalEvent,
        exclude_event_id: Option<Uuid>,
    ) -> anyhow::Result<Option<String>> {
        if matches!(candidate.time, TimeSpec::DateOnly { .. }) {
            return Ok(None);
        }

        let events = self.store.list_events()?;
        let check =
            conflicts_for_candidate_event(&events, candidate, self.timezone(), exclude_event_id)?;
        if check.conflicts.is_empty() {
            return Ok(None);
        }

        let mut titles = check
            .conflicts
            .iter()
            .map(|interval| interval.event_title.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let extra = titles.len().saturating_sub(4);
        titles.truncate(4);
        let mut message = format!("Scheduling conflict with {}.", titles.join(", "));
        if extra != 0 {
            message.push_str(&format!(" +{extra} more."));
        }
        if !check.skipped.is_empty() {
            message.push_str(&format!(
                " {} other events could not be conflict-checked.",
                check.skipped.len()
            ));
        }
        message.push_str(" Save again without changing the time/status/availability to confirm.");
        Ok(Some(message))
    }

    fn confirmation_with_alternatives(
        &mut self,
        event: &TemporalEvent,
        warning: String,
        exclude_event_id: Option<Uuid>,
        target: ConflictAlternativeTarget,
    ) -> ConflictConfirmation {
        let mut confirmation = ConflictConfirmation::for_event(event, warning);
        if !matches!(
            &event.time,
            TimeSpec::Instant { .. } | TimeSpec::Floating { .. } | TimeSpec::AllDay { .. }
        ) {
            return confirmation;
        }

        let prerequisites = (|| -> anyhow::Result<_> {
            let database_path = self
                .store
                .path()
                .map(std::path::Path::to_path_buf)
                .ok_or_else(|| {
                    anyhow::anyhow!("background suggestions require a file-backed database")
                })?;
            let preferences = if matches!(&event.time, TimeSpec::AllDay { .. }) {
                None
            } else {
                Some(parse_slot_search(
                    &self.state.availability_duration_minutes,
                    &self.state.availability_step_minutes,
                    &self.state.availability_day_start,
                    &self.state.availability_day_end,
                    self.state.availability_workdays,
                )?)
            };
            Ok((database_path, preferences))
        })();

        match prerequisites {
            Ok((database_path, preferences)) => {
                confirmation.alternative_note = Some("Finding later alternatives…".to_string());
                let candidate = event.clone();
                let display_timezone = self.timezone();
                let workdays = self.state.availability_workdays;
                let (sender, receiver) = mpsc::channel();
                self.conflict_alternative_receiver = Some((target, candidate.clone(), receiver));
                std::thread::spawn(move || {
                    let result = (|| -> anyhow::Result<AlternativeSlots> {
                        let store = TemporalStore::open(database_path)?;
                        let events = store.list_events()?;
                        if matches!(&candidate.time, TimeSpec::AllDay { .. }) {
                            alternative_days_for_candidate(
                                &events,
                                &candidate,
                                display_timezone,
                                exclude_event_id,
                                workdays,
                                4,
                            )
                        } else {
                            let preferences = preferences.ok_or_else(|| {
                                anyhow::anyhow!("timed alternative search preferences are missing")
                            })?;
                            alternative_slots_for_candidate(
                                &events,
                                &candidate,
                                display_timezone,
                                exclude_event_id,
                                preferences,
                                4,
                            )
                        }
                    })()
                    .map_err(|error| format!("{error:#}"));
                    let _ = sender.send(result);
                });
            }
            Err(error) => {
                confirmation.alternative_note =
                    Some(format!("Alternatives unavailable: {error:#}"));
            }
        }
        confirmation
    }

    fn poll_conflict_alternatives(&mut self) {
        let completed = match self.conflict_alternative_receiver.as_ref() {
            Some((_, _, receiver)) => match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(
                    "Alternative-slot worker exited without a result".to_string(),
                )),
            },
            None => None,
        };
        let Some(result) = completed else {
            return;
        };
        let Some((target, candidate, _)) = self.conflict_alternative_receiver.take() else {
            return;
        };
        let display_timezone = self.timezone();

        let confirmation = match target {
            ConflictAlternativeTarget::NewEvent(draft_token) => {
                self.new_local_event.as_mut().and_then(|draft| {
                    let is_current = draft.draft_token == draft_token
                        && new_conflict_confirmation_is_current(
                            draft,
                            &candidate,
                            display_timezone,
                        );
                    is_current
                        .then_some(draft)
                        .and_then(|draft| draft.conflict_confirmation.as_mut())
                })
            }
            ConflictAlternativeTarget::TimeEdit(event_id, draft_token) => {
                self.event_time_editor.as_mut().and_then(|draft| {
                    let is_current = draft.draft_token == draft_token
                        && time_conflict_confirmation_is_current(draft, &candidate, event_id);
                    is_current
                        .then_some(draft)
                        .and_then(|draft| draft.conflict_confirmation.as_mut())
                })
            }
        };

        if let Some(confirmation) = confirmation {
            match result {
                Ok(alternatives) => {
                    confirmation.alternatives = alternatives.slots;
                    confirmation.alternative_note = if !alternatives.skipped.is_empty() {
                        Some(format!(
                            "Advisory: {} stored event(s) could not be checked.",
                            alternatives.skipped.len()
                        ))
                    } else if confirmation.alternatives.is_empty() {
                        Some("No later matching slot in the next 15 days.".to_string())
                    } else {
                        None
                    };
                }
                Err(error) => {
                    confirmation.alternative_note =
                        Some(format!("Alternatives unavailable: {error}"));
                }
            }
        }
    }

    fn begin_new_local_event(&mut self) {
        self.new_local_event = Some(NewLocalEventDraft::for_date(self.state.focus_date()));
        self.last_error = None;
    }

    fn save_new_local_event(&mut self) {
        let Some(draft) = self.new_local_event.clone() else {
            return;
        };

        let result = (|| -> anyhow::Result<(Uuid, NaiveDate)> {
            let (event, focus_date) = new_local_event_from_draft(&draft, self.timezone())?;

            if let Some(warning) = self.scheduling_conflict_warning(&event, None)?
                && draft
                    .conflict_confirmation
                    .as_ref()
                    .is_none_or(|confirmation| !confirmation.matches(&event))
            {
                let confirmation = self.confirmation_with_alternatives(
                    &event,
                    warning.clone(),
                    None,
                    ConflictAlternativeTarget::NewEvent(draft.draft_token),
                );
                if let Some(current) = self.new_local_event.as_mut() {
                    current.conflict_confirmation = Some(confirmation);
                }
                anyhow::bail!("{warning}");
            }

            let event_id = event.id;
            self.store.upsert_event(&event)?;
            Ok((event_id, focus_date))
        })();

        match result {
            Ok((event_id, focus_date)) => {
                self.new_local_event = None;
                self.state.set_focus_date(focus_date);
                self.state.selected_event_id = Some(event_id);
                self.state.show_inspector = true;
                self.last_message = Some("Created local event.".to_string());
                self.last_error = None;
                self.mark_state_dirty();
                self.reload_or_report();
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to create local event: {error:#}"));
            }
        }
    }

    fn begin_event_time_edit(&mut self, event_id: Uuid) {
        let canonical_id = self.canonical_event_id(event_id);
        let result = (|| -> anyhow::Result<EventTimeEditDraft> {
            let event = self
                .store
                .event_by_id(canonical_id)?
                .ok_or_else(|| anyhow::anyhow!("canonical event no longer exists"))?;
            if !self.event_is_editable(&event) {
                anyhow::bail!("event source is read-only");
            }
            EventTimeEditDraft::from_event(&event, self.timezone())
        })();

        match result {
            Ok(draft) => {
                self.event_time_editor = Some(draft);
                self.last_error = None;
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Cannot edit event time: {error:#}"));
            }
        }
    }

    fn save_event_time_edit(&mut self) {
        let Some(draft) = self.event_time_editor.clone() else {
            return;
        };

        let result = (|| -> anyhow::Result<()> {
            let mut event = self
                .store
                .event_by_id(draft.event_id)?
                .ok_or_else(|| anyhow::anyhow!("canonical event no longer exists"))?;
            if !self.event_is_editable(&event) {
                anyhow::bail!("event source is read-only");
            }
            if event.recurrence.is_some() {
                anyhow::bail!("recurring event master time editing is not supported yet");
            }
            let proposed_time = draft.parsed_time()?;
            if let Some((original_time, original_uncertainty)) = &draft.uncertain_origin {
                if event.time != *original_time
                    || event.time_uncertainty.as_ref() != Some(original_uncertainty)
                {
                    anyhow::bail!(
                        "uncertain event changed since the editor opened; reopen its time editor"
                    );
                }
                if proposed_time != event.time {
                    event = move_uncertain_placement(&event, &proposed_time)?;
                    let warning = "Moving an uncertain event shifts its entire possible-start window. Its availability cannot be certified from the representative time. Save again without changing the proposed time to acknowledge this provisional placement.";
                    if draft
                        .conflict_confirmation
                        .as_ref()
                        .is_none_or(|confirmation| !confirmation.matches(&event))
                    {
                        if let Some(current) = self.event_time_editor.as_mut() {
                            current.conflict_confirmation =
                                Some(ConflictConfirmation::for_event(&event, warning.to_string()));
                        }
                        anyhow::bail!("{warning}");
                    }
                }
            } else {
                if event.time_uncertainty.is_some() {
                    anyhow::bail!(
                        "event acquired bounded uncertainty since the editor opened; reopen it"
                    );
                }
                event.time = proposed_time;
            }

            if draft.uncertain_origin.is_none()
                && let Some(warning) = self.scheduling_conflict_warning(&event, Some(event.id))?
                && draft
                    .conflict_confirmation
                    .as_ref()
                    .is_none_or(|confirmation| !confirmation.matches(&event))
            {
                let confirmation = self.confirmation_with_alternatives(
                    &event,
                    warning.clone(),
                    Some(event.id),
                    ConflictAlternativeTarget::TimeEdit(event.id, draft.draft_token),
                );
                if let Some(current) = self.event_time_editor.as_mut() {
                    current.conflict_confirmation = Some(confirmation);
                }
                anyhow::bail!("{warning}");
            }

            event.updated_at = Utc::now();
            self.store.upsert_event(&event)?;
            Ok(())
        })();

        match result {
            Ok(()) => {
                self.event_time_editor = None;
                self.last_message = Some("Saved canonical event time.".to_string());
                self.last_error = None;
                self.reload_or_report();
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to save event time: {error:#}"));
            }
        }
    }

    fn begin_event_location_edit(&mut self, event_id: Uuid) {
        let canonical_id = self.canonical_event_id(event_id);
        match self.store.event_by_id(canonical_id) {
            Ok(Some(event)) if self.event_is_editable(&event) => {
                self.event_location_editor = Some(EventLocationEditDraft::from_event(&event));
                self.last_error = None;
            }
            Ok(Some(_)) => {
                self.last_message = None;
                self.last_error = Some(
                    "This event comes from a read-only source and cannot be edited.".to_string(),
                );
            }
            Ok(None) => {
                self.last_message = None;
                self.last_error = Some("The canonical event could not be found.".to_string());
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to load location editor: {error:#}"));
            }
        }
    }

    fn save_event_location_edit(&mut self) {
        let Some(draft) = self.event_location_editor.clone() else {
            return;
        };

        let result = (|| -> anyhow::Result<()> {
            let mut event = self
                .store
                .event_by_id(draft.event_id)?
                .ok_or_else(|| anyhow::anyhow!("canonical event no longer exists"))?;
            if !self.event_is_editable(&event) {
                anyhow::bail!("event source is read-only");
            }
            event.location = draft.parsed_location()?;
            event.updated_at = Utc::now();
            self.store.upsert_event(&event)?;
            Ok(())
        })();

        match result {
            Ok(()) => {
                self.event_location_editor = None;
                self.last_message = Some("Saved structured event location.".to_string());
                self.last_error = None;
                self.reload_or_report();
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to save event location: {error:#}"));
            }
        }
    }

    fn begin_event_details_edit(&mut self, event_id: Uuid) {
        let canonical_id = self.canonical_event_id(event_id);
        match self.store.event_by_id(canonical_id) {
            Ok(Some(event)) if self.event_is_editable(&event) => {
                self.event_details_editor = Some(EventDetailsEditDraft::from_event(&event));
                self.last_error = None;
            }
            Ok(Some(_)) => {
                self.last_message = None;
                self.last_error = Some(
                    "This event comes from a read-only source and cannot be edited.".to_string(),
                );
            }
            Ok(None) => {
                self.last_message = None;
                self.last_error = Some("The canonical event could not be found.".to_string());
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to load event editor: {error:#}"));
            }
        }
    }

    fn save_event_details_edit(&mut self) {
        let Some(draft) = self.event_details_editor.clone() else {
            return;
        };

        let result = (|| -> anyhow::Result<()> {
            let title = draft.title.trim();
            if title.is_empty() {
                anyhow::bail!("event title cannot be empty");
            }

            let confidence = parse_optional_confidence(&draft.confidence)?;
            let importance = parse_optional_i32(&draft.importance, "importance")?;
            let personal_relevance =
                parse_optional_i32(&draft.personal_relevance, "personal relevance")?;

            let mut event = self
                .store
                .event_by_id(draft.event_id)?
                .ok_or_else(|| anyhow::anyhow!("canonical event no longer exists"))?;
            if !self.event_is_editable(&event) {
                anyhow::bail!("event source is read-only");
            }
            let original = event.clone();

            event.normalized_title = title.to_string();
            event.description = optional_trimmed(&draft.description);
            event.event_type = optional_trimmed(&draft.event_type);
            event.domain = optional_trimmed(&draft.domain);
            event.jurisdiction = optional_trimmed(&draft.jurisdiction);
            event.institution = optional_trimmed(&draft.institution);
            event.status = draft.status;
            event.availability = draft.availability;
            event.confidence = confidence;
            event.importance = importance;
            event.personal_relevance = personal_relevance;

            let warning = if details_change_requires_provisional_uncertainty_confirmation(
                &original, &event,
            ) {
                Some("Activating a Busy event with bounded start uncertainty does not establish a definite occupied interval. Its possible-start window remains provisional; Save again without changing status or availability to acknowledge this.".to_string())
            } else if details_change_requires_conflict_check(&original, &event) {
                self.scheduling_conflict_warning(&event, Some(event.id))?
            } else {
                None
            };
            if let Some(warning) = warning
                && draft
                    .conflict_confirmation
                    .as_ref()
                    .is_none_or(|confirmation| !confirmation.matches(&event))
            {
                if let Some(current) = self.event_details_editor.as_mut() {
                    current.conflict_confirmation =
                        Some(ConflictConfirmation::for_event(&event, warning.clone()));
                }
                anyhow::bail!("{warning}");
            }

            event.updated_at = Utc::now();
            self.store.upsert_event(&event)?;
            Ok(())
        })();

        match result {
            Ok(()) => {
                self.event_details_editor = None;
                self.last_message = Some("Saved canonical event details.".to_string());
                self.last_error = None;
                self.reload_or_report();
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to save event details: {error:#}"));
            }
        }
    }

    fn begin_recurrence_edit(&mut self, event_id: Uuid) {
        let canonical_id = self.canonical_event_id(event_id);
        match self.store.event_by_id(canonical_id) {
            Ok(Some(event)) if self.event_is_editable(&event) => {
                self.recurrence_editor = Some(RecurrenceEditDraft::from_event(&event));
                self.last_error = None;
            }
            Ok(Some(_)) => {
                self.last_message = None;
                self.last_error = Some(
                    "This event comes from a read-only source and cannot be edited.".to_string(),
                );
            }
            Ok(None) => {
                self.last_message = None;
                self.last_error = Some("The canonical event could not be found.".to_string());
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to load recurrence editor: {error:#}"));
            }
        }
    }

    fn begin_occurrence_override_edit(&mut self, occurrence_id: Uuid) {
        let Some(context) = self.occurrence_contexts.get(&occurrence_id).cloned() else {
            self.last_error =
                Some("Selected event has no recurrence occurrence identity.".to_string());
            return;
        };
        self.begin_recurrence_edit(context.event_id);
        let current_time = self
            .events
            .iter()
            .find(|event| event.id == occurrence_id)
            .map(|event| event.time.clone());
        if let Some(draft) = self.recurrence_editor.as_mut()
            && draft.event_id == context.event_id
        {
            match draft.focus_occurrence(&context.original_time) {
                Ok(()) => draft.focused_occurrence_current = current_time,
                Err(error) => {
                    self.last_message = None;
                    self.last_error = Some(format!("Cannot focus recurring occurrence: {error}"));
                }
            }
        }
    }

    fn find_recurring_occurrence_alternatives(&mut self) {
        let result = (|| -> anyhow::Result<_> {
            let draft = self
                .recurrence_editor
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("recurrence editor is closed"))?;
            let original = draft
                .focused_occurrence_original
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("no recurring occurrence is focused"))?
                .clone();
            let current = draft
                .focused_occurrence_current
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("selected occurrence is no longer materialized"))?
                .clone();
            if !matches!(
                current,
                TimeSpec::Instant { .. } | TimeSpec::Floating { .. } | TimeSpec::AllDay { .. }
            ) {
                anyhow::bail!("this occurrence has no definite schedulable interval");
            }
            let event = self
                .store
                .event_by_id(draft.event_id)?
                .ok_or_else(|| anyhow::anyhow!("recurrence series no longer exists"))?;
            if !self.event_is_editable(&event) {
                anyhow::bail!("recurrence series is read-only");
            }
            let rule = event
                .recurrence
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("event no longer recurs"))?
                .clone();
            if draft.parsed_rule().map_err(anyhow::Error::msg)? != rule {
                anyhow::bail!(
                    "save or revert unsaved recurrence edits before finding alternatives"
                );
            }
            let current_stored = rule
                .overrides
                .iter()
                .find(|value| value.original == original)
                .and_then(|value| value.replacement.as_ref())
                .unwrap_or(&original);
            if *current_stored != current {
                anyhow::bail!("the selected occurrence has changed; reopen its editor");
            }
            let database_path = self
                .store
                .path()
                .map(std::path::Path::to_path_buf)
                .ok_or_else(|| {
                    anyhow::anyhow!("background suggestions require a file-backed database")
                })?;
            let search = if matches!(current, TimeSpec::AllDay { .. }) {
                SlotSearch {
                    duration_minutes: 60,
                    step_minutes: 30,
                    day_start: NaiveTime::from_hms_opt(9, 0, 0).expect("valid hour"),
                    day_end: NaiveTime::from_hms_opt(17, 0, 0).expect("valid hour"),
                    workdays: self.state.availability_workdays,
                }
            } else {
                parse_slot_search(
                    &self.state.availability_duration_minutes,
                    &self.state.availability_step_minutes,
                    &self.state.availability_day_start,
                    &self.state.availability_day_end,
                    self.state.availability_workdays,
                )?
            };
            Ok((
                draft.draft_token,
                event.id,
                original,
                current,
                rule,
                database_path,
                search,
                self.timezone(),
            ))
        })();

        match result {
            Ok((
                draft_token,
                event_id,
                original,
                current,
                rule,
                database_path,
                search,
                timezone,
            )) => {
                let was_cancelled = rule
                    .overrides
                    .iter()
                    .any(|value| value.original == original && value.cancelled);
                let (sender, receiver) = mpsc::channel();
                self.recurrence_alternative_receiver = Some(RecurrenceAlternativeWorker {
                    draft_token,
                    event_id,
                    original: original.clone(),
                    current: current.clone(),
                    rule,
                    receiver,
                });
                if let Some(draft) = self.recurrence_editor.as_mut() {
                    draft.alternative_slots.clear();
                    draft.alternative_rule = None;
                    draft.alternative_note = Some(if was_cancelled {
                        "Finding restoration openings…".to_string()
                    } else {
                        "Finding later openings…".to_string()
                    });
                }
                std::thread::spawn(move || {
                    let result = (|| -> anyhow::Result<AlternativeSlots> {
                        let store = TemporalStore::open(database_path)?;
                        let events = store.list_events()?;
                        if was_cancelled {
                            alternative_slots_for_canceled_recurring_occurrence(
                                &events, event_id, &original, timezone, search, 4,
                            )
                        } else {
                            crate::availability::alternative_slots_for_recurring_occurrence(
                                &events, event_id, &original, &current, timezone, search, 4,
                            )
                        }
                    })()
                    .map_err(|error| format!("{error:#}"));
                    let _ = sender.send(result);
                });
            }
            Err(error) => {
                if let Some(draft) = self.recurrence_editor.as_mut() {
                    draft.alternative_slots.clear();
                    draft.alternative_rule = None;
                    draft.alternative_note = Some(format!("Alternatives unavailable: {error:#}"));
                }
            }
        }
    }

    fn poll_recurring_occurrence_alternatives(&mut self) {
        let completed = match self.recurrence_alternative_receiver.as_ref() {
            Some(worker) => match worker.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(
                    "Recurring alternatives worker exited unexpectedly".to_string(),
                )),
            },
            None => None,
        };
        let Some(result) = completed else { return };
        let Some(worker) = self.recurrence_alternative_receiver.take() else {
            return;
        };
        let Some(draft) = self.recurrence_editor.as_mut() else {
            return;
        };
        if draft.draft_token != worker.draft_token
            || draft.event_id != worker.event_id
            || draft.focused_occurrence_original.as_ref() != Some(&worker.original)
            || draft.focused_occurrence_current.as_ref() != Some(&worker.current)
            || draft.parsed_rule().ok().as_ref() != Some(&worker.rule)
        {
            return;
        }
        let was_cancelled = worker
            .rule
            .overrides
            .iter()
            .any(|value| value.original == worker.original && value.cancelled);
        match result {
            Ok(alternatives) => {
                draft.alternative_note = Some(if alternatives.slots.is_empty() {
                    if was_cancelled {
                        "No restoration openings found in the next 15 days.".to_string()
                    } else {
                        "No later openings found in the next 15 days.".to_string()
                    }
                } else if alternatives.skipped.is_empty() {
                    "Suggested openings are advisory; Save validates the recurrence.".to_string()
                } else {
                    format!(
                        "Provisional openings: {} event(s) could not be checked. Save validates the recurrence.",
                        alternatives.skipped.len()
                    )
                });
                draft.alternative_slots = alternatives.slots;
                draft.alternative_rule = Some(worker.rule);
            }
            Err(error) => {
                draft.alternative_slots.clear();
                draft.alternative_rule = None;
                draft.alternative_note = Some(format!("Alternatives unavailable: {error}"));
            }
        }
    }

    fn use_recurring_occurrence_alternative(&mut self, slot: &FreeInterval) {
        let timezone = self.timezone();
        let result = self
            .recurrence_editor
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("recurrence editor is closed"))
            .and_then(|draft| apply_recurring_alternative_to_draft(draft, slot, timezone));
        match result {
            Ok(()) => self.last_error = None,
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Cannot apply occurrence alternative: {error:#}"));
            }
        }
    }

    fn save_recurrence_edit(&mut self, remove: bool) {
        let Some(draft) = self.recurrence_editor.clone() else {
            return;
        };

        let result = (|| -> anyhow::Result<()> {
            let mut event = self
                .store
                .event_by_id(draft.event_id)?
                .ok_or_else(|| anyhow::anyhow!("canonical event no longer exists"))?;
            if !self.event_is_editable(&event) {
                anyhow::bail!("event source is read-only");
            }

            let proposed_rule = if remove {
                None
            } else {
                Some(
                    draft
                        .parsed_rule()
                        .map_err(|error| anyhow::anyhow!("{error}"))?,
                )
            };
            if let Some(rule) = proposed_rule.as_ref()
                && draft.focused_occurrence_original.is_some()
                && let Some(warning) = focused_recurrence_conflict_warning(
                    &self.store,
                    self.timezone(),
                    &event,
                    &draft,
                    rule,
                )?
                && draft.focused_conflict_confirmed_rule.as_ref() != Some(rule)
            {
                if let Some(current) = self.recurrence_editor.as_mut() {
                    current.focused_conflict_confirmed_rule = Some(rule.clone());
                    current.focused_conflict_warning = Some(warning.clone());
                }
                anyhow::bail!("{warning}");
            }
            event.recurrence = proposed_rule;
            event.updated_at = Utc::now();
            self.store.upsert_event(&event)?;
            Ok(())
        })();

        match result {
            Ok(()) => {
                self.recurrence_editor = None;
                self.last_message = Some(if remove {
                    "Removed recurrence from canonical event.".to_string()
                } else {
                    "Saved recurrence on canonical event.".to_string()
                });
                self.last_error = None;
                self.state.selected_event_id = None;
                self.mark_state_dirty();
                self.reload_or_report();
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to save recurrence: {error:#}"));
            }
        }
    }

    fn create_event_notification_rule(&mut self, event_id: Uuid, event_title: &str) {
        let result = (|| -> anyhow::Result<()> {
            let lead_minutes =
                parse_notification_lead_minutes(&self.notification_event_lead_minutes)?;
            let rule = NotificationRule::for_event(
                event_id,
                format!("{event_title} · {lead_minutes}m before"),
                lead_minutes,
            );
            self.store.upsert_notification_rule(&rule)?;
            Ok(())
        })();

        match result {
            Ok(()) => {
                self.last_message = Some("Added event reminder.".to_string());
                self.last_error = None;
                self.reload_or_report();
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to add event reminder: {error:#}"));
            }
        }
    }

    fn create_saved_view_notification_rule(&mut self) {
        let result = (|| -> anyhow::Result<()> {
            let saved_view_id = self
                .notification_saved_view_id
                .ok_or_else(|| anyhow::anyhow!("select a saved view first"))?;
            let view = self
                .saved_views
                .iter()
                .find(|view| view.id == saved_view_id)
                .ok_or_else(|| anyhow::anyhow!("saved view no longer exists"))?;
            let lead_minutes =
                parse_notification_lead_minutes(&self.notification_saved_view_lead_minutes)?;
            let rule = NotificationRule::for_saved_view(
                saved_view_id,
                format!("{} · {lead_minutes}m before", view.name),
                lead_minutes,
            );
            self.store.upsert_notification_rule(&rule)?;
            Ok(())
        })();

        match result {
            Ok(()) => {
                self.last_message = Some("Added saved-view reminder rule.".to_string());
                self.last_error = None;
                self.reload_or_report();
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to add saved-view reminder: {error:#}"));
            }
        }
    }

    fn set_notification_rule_enabled(&mut self, rule_id: Uuid, enabled: bool) {
        let result = (|| -> anyhow::Result<()> {
            let mut rule = self
                .store
                .notification_rule_by_id(rule_id)?
                .ok_or_else(|| anyhow::anyhow!("notification rule no longer exists"))?;
            rule.enabled = enabled;
            rule.updated_at = Utc::now();
            self.store.upsert_notification_rule(&rule)
        })();

        match result {
            Ok(()) => {
                self.last_error = None;
                self.reload_or_report();
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to update reminder: {error:#}"));
            }
        }
    }

    fn refresh_reminder_delivery_lists(&mut self, now: DateTime<Utc>) -> anyhow::Result<()> {
        let due = self.store.due_notification_deliveries(now)?;
        let snoozed = self.store.snoozed_notification_deliveries(now)?;
        self.notification_deliveries = due;
        self.notification_snoozed_deliveries = snoozed;
        Ok(())
    }

    fn wake_notification_delivery(&mut self, delivery_id: Uuid) {
        match self.store.clear_notification_snooze(delivery_id) {
            Ok(true) => {
                self.last_message = Some("Reminder is due again.".to_string());
                self.last_error = None;
                if let Err(error) = self.refresh_reminder_delivery_lists(Utc::now()) {
                    self.last_error = Some(format!("Failed to refresh reminders: {error:#}"));
                }
            }
            Ok(false) => {
                self.last_message = None;
                self.last_error = Some("Reminder is no longer snoozed.".to_string());
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to wake reminder: {error:#}"));
            }
        }
    }

    fn snooze_notification_delivery(&mut self, delivery_id: Uuid, minutes: i64) {
        let until = Utc::now() + ChronoDuration::minutes(minutes);
        match self.store.snooze_notification_delivery(delivery_id, until) {
            Ok(true) => {
                self.last_message = Some(format!("Snoozed reminder for {minutes} minutes."));
                self.last_error = None;
                if let Err(error) = self.refresh_reminder_delivery_lists(Utc::now()) {
                    self.last_error = Some(format!("Failed to refresh reminders: {error:#}"));
                }
            }
            Ok(false) => {
                self.notification_deliveries
                    .retain(|delivery| delivery.id != delivery_id);
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to snooze reminder: {error:#}"));
            }
        }
    }

    fn dismiss_notification_delivery(&mut self, delivery_id: Uuid) {
        match self
            .store
            .dismiss_notification_delivery(delivery_id, Utc::now())
        {
            Ok(true) => {
                self.last_message = Some("Dismissed reminder.".to_string());
                self.last_error = None;
                if let Err(error) = self.refresh_reminder_delivery_lists(Utc::now()) {
                    self.last_error = Some(format!("Failed to refresh reminders: {error:#}"));
                }
            }
            Ok(false) => {
                self.notification_deliveries
                    .retain(|delivery| delivery.id != delivery_id);
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to dismiss reminder: {error:#}"));
            }
        }
    }

    fn delete_notification_rule(&mut self, rule_id: Uuid) {
        match self.store.delete_notification_rule(rule_id) {
            Ok(true) => {
                self.last_message = Some("Deleted reminder rule.".to_string());
                self.last_error = None;
                self.reload_or_report();
            }
            Ok(false) => {
                self.last_message = None;
                self.last_error = Some("Reminder rule no longer exists.".to_string());
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Failed to delete reminder: {error:#}"));
            }
        }
    }

    fn refresh_notification_evaluation(&mut self) -> anyhow::Result<()> {
        let now = Utc::now();
        let events = self.store.list_events()?;
        let evaluation = evaluate_notification_rules(
            &self.notification_rules,
            &events,
            &self.saved_views,
            &self.event_memberships,
            &QueryContext::for_timezone(self.timezone()),
            now - ChronoDuration::hours(24),
            now + ChronoDuration::days(7),
        )?;

        for occurrence in evaluation
            .occurrences
            .iter()
            .filter(|occurrence| occurrence.trigger_at_utc <= now)
        {
            let delivery = NotificationDelivery::from_occurrence(occurrence, now);
            self.store.record_notification_delivery(&delivery)?;
        }

        self.refresh_reminder_delivery_lists(now)?;
        self.notification_occurrences = evaluation.occurrences;
        self.notification_skipped = evaluation.skipped;
        self.notification_eval_minute = Some(now.timestamp() / 60);
        Ok(())
    }

    fn refresh_notification_evaluation_if_needed(&mut self) {
        if self.notification_rules.is_empty() {
            self.notification_occurrences.clear();
            self.notification_skipped.clear();
            self.notification_eval_minute = None;
            return;
        }

        let minute = Utc::now().timestamp() / 60;
        if self.notification_eval_minute == Some(minute) {
            return;
        }
        if let Err(error) = self.refresh_notification_evaluation() {
            self.last_error = Some(format!("Failed to evaluate reminders: {error:#}"));
        }
    }

    fn reload(&mut self) -> anyhow::Result<()> {
        self.event_revision_event_id = None;
        self.event_revisions.clear();

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
        let mut candidates = self
            .store
            .events_in_window(
                window.start,
                window.end_exclusive,
                timezone,
                include_imprecise,
            )?
            .into_iter()
            .map(|event| (event.id, event))
            .collect::<HashMap<_, _>>();
        for event in self.store.recurring_events()? {
            candidates.entry(event.id).or_insert(event);
        }

        self.events.clear();
        self.occurrence_contexts.clear();
        for event in candidates.into_values() {
            if event.recurrence.is_some() {
                for occurrence in
                    event.occurrences_in_window(window.start, window.end_exclusive, timezone)?
                {
                    let mut materialized = event.clone();
                    materialized.id = occurrence.id;
                    materialized.time = occurrence.time.clone();
                    materialized.status = occurrence.status;
                    self.occurrence_contexts.insert(
                        materialized.id,
                        OccurrenceContext {
                            event_id: event.id,
                            recurrence_index: occurrence.recurrence_index,
                            origin: occurrence.origin,
                            original_time: occurrence.original_time,
                            override_applied: occurrence.override_applied,
                            cancelled_by_override: occurrence.cancelled_by_override,
                        },
                    );
                    self.events.push(materialized);
                }
            } else {
                self.events.push(event);
            }
        }

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
        self.event_memberships = self
            .store
            .taria_event_memberships_for_release(self.state.taria_last_release_id.as_deref())?;
        for (event_id, topology) in self.store.event_relation_collection_memberships()? {
            let membership = self.event_memberships.entry(event_id).or_default();
            membership.collection_ids.extend(topology.collection_ids);
            membership
                .outgoing_relation_types
                .extend(topology.outgoing_relation_types);
            membership
                .incoming_relation_types
                .extend(topology.incoming_relation_types);
        }
        for assessment in self.store.list_event_identity_assessments()? {
            let state = assessment.state.as_str().to_string();
            self.event_memberships
                .entry(assessment.left_event_id)
                .or_default()
                .identity_states
                .insert(state.clone());
            self.event_memberships
                .entry(assessment.right_event_id)
                .or_default()
                .identity_states
                .insert(state);
        }
        self.event_annotations = self.store.list_event_annotations()?;
        self.event_annotation_kinds = self
            .event_annotations
            .iter()
            .map(|annotation| annotation.kind.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for annotation in &self.event_annotations {
            self.event_memberships
                .entry(annotation.event_id)
                .or_default()
                .annotation_kinds
                .insert(annotation.kind.clone());
        }
        self.event_provenance_records = self.store.list_event_provenance_records()?;
        for record in &self.event_provenance_records {
            let membership = self.event_memberships.entry(record.event_id).or_default();
            membership
                .provenance_roles
                .insert(record.role.as_str().to_string());
            membership
                .provenance_references
                .insert(record.reference.clone());
        }
        self.canonical_entities = self.store.list_canonical_entities()?;
        self.canonical_entity_usage = self.store.canonical_entity_event_usage()?;
        for (event_id, entities) in self.store.resolved_canonical_entities_by_event()? {
            let membership = self.event_memberships.entry(event_id).or_default();
            for entity in entities {
                membership.canonical_entity_ids.insert(entity.id);
                membership
                    .canonical_entity_names
                    .insert(entity.canonical_name.clone());
                if let Some(entity_type) = entity.entity_type {
                    membership.canonical_entity_types.insert(entity_type);
                }
            }
        }

        self.event_collections = self.store.list_event_collections()?;
        self.event_collection_members = self.store.list_event_collection_members()?;
        self.event_relations = self.store.list_event_relations()?;
        self.event_relation_types = self
            .event_relations
            .iter()
            .map(|relation| relation.relation_type.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self.event_relation_titles.clear();
        let relation_event_ids = self
            .event_relations
            .iter()
            .flat_map(|relation| [relation.from_event_id, relation.to_event_id])
            .collect::<BTreeSet<_>>();
        for event_id in relation_event_ids {
            if let Some(related) = self.store.event_by_id(event_id)? {
                self.event_relation_titles
                    .insert(event_id, related.normalized_title);
            }
        }
        self.event_identity_assessments = self.store.list_event_identity_assessments()?;
        self.event_identity_titles.clear();
        let identity_event_ids = self
            .event_identity_assessments
            .iter()
            .flat_map(|assessment| [assessment.left_event_id, assessment.right_event_id])
            .collect::<BTreeSet<_>>();
        for event_id in identity_event_ids {
            if let Some(related) = self.store.event_by_id(event_id)? {
                self.event_identity_titles
                    .insert(event_id, related.normalized_title);
            }
        }
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
        self.notification_rules = self.store.list_notification_rules()?;
        self.notification_eval_minute = None;
        self.refresh_notification_evaluation()?;
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
                if self.notification_saved_view_id == Some(id) {
                    self.notification_saved_view_id = self.saved_views.first().map(|view| view.id);
                }
                self.notification_eval_minute = None;
                self.mark_state_dirty();
                self.reload_or_report();
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
                        self.event_memberships
                            .get(&self.canonical_event_id(event.id)),
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
                        self.event_memberships
                            .get(&self.canonical_event_id(event.id)),
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
            if input.key_pressed(egui::Key::P) {
                target_layout = Some(CalendarLayout::Summary);
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

    fn render_availability_panel(&mut self, ui: &mut egui::Ui) {
        let timezone = self.timezone();
        let window = window_for_view(
            self.state.calendar_view,
            self.state.focus_date(),
            self.state.week_start_monday,
        );
        let visible_events = self.visible_events();
        let mut selected_free_interval = None;
        let mut selected_suggested_slot = None;
        let mut selected_uncheckable_event = None;
        let mut availability_preferences_changed = false;

        ui.collapsing(
            format!("Availability · {}", self.state.calendar_view.label()),
            |ui| {
                ui.small(format!(
                    "{} through {} · {} · current query/source visibility",
                    window.start,
                    window.end_exclusive.pred_opt().unwrap_or(window.start),
                    timezone
                ));

                match availability_for_materialized_date_window(
                    &visible_events,
                    timezone,
                    window.start,
                    window.end_exclusive,
                ) {
                    Ok(result) => {
                        let tentative = result
                            .busy
                            .iter()
                            .filter(|interval| interval.kind == BusyKind::Tentative)
                            .count();
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(format!("{} free interval(s)", result.free.len()));
                            ui.small(format!(
                                "· {} busy/tentative block(s) · {} tentative · {} skipped",
                                result.busy.len(),
                                tentative,
                                result.skipped.len()
                            ));
                        });

                        if !result.skipped.is_empty() {
                            ui.colored_label(
                                Color32::YELLOW,
                                "Availability is provisional: some events have no definite checkable interval. Inspect the skipped events below.",
                            );
                        }

                        ui.add_space(4.0);
                        ui.strong("Find a slot");
                        ui.horizontal_wrapped(|ui| {
                            ui.small("duration");
                            availability_preferences_changed |= ui
                                .add(
                                    egui::TextEdit::singleline(
                                        &mut self.state.availability_duration_minutes,
                                    )
                                    .desired_width(55.0),
                                )
                                .changed();
                            ui.small("min · step");
                            availability_preferences_changed |= ui
                                .add(
                                    egui::TextEdit::singleline(
                                        &mut self.state.availability_step_minutes,
                                    )
                                    .desired_width(55.0),
                                )
                                .changed();
                            ui.small("min · hours");
                            availability_preferences_changed |= ui
                                .add(
                                    egui::TextEdit::singleline(
                                        &mut self.state.availability_day_start,
                                    )
                                    .desired_width(60.0),
                                )
                                .changed();
                            ui.small("to");
                            availability_preferences_changed |= ui
                                .add(
                                    egui::TextEdit::singleline(
                                        &mut self.state.availability_day_end,
                                    )
                                    .desired_width(60.0),
                                )
                                .changed();
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.small("days");
                            for (index, label) in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
                                .into_iter()
                                .enumerate()
                            {
                                availability_preferences_changed |= ui
                                    .checkbox(&mut self.state.availability_workdays[index], label)
                                    .changed();
                            }
                        });

                        match parse_slot_search(
                            &self.state.availability_duration_minutes,
                            &self.state.availability_step_minutes,
                            &self.state.availability_day_start,
                            &self.state.availability_day_end,
                            self.state.availability_workdays,
                        )
                        .and_then(|search| {
                            suggest_slots(
                                &result.free,
                                timezone,
                                window.start,
                                window.end_exclusive,
                                search,
                            )
                        }) {
                            Ok(slots) => {
                                if slots.is_empty() {
                                    ui.small("No candidate slots match those constraints.");
                                }
                                for slot in slots.iter().take(20) {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.monospace(format_availability_interval(
                                            slot.start_utc,
                                            slot.end_utc,
                                            timezone,
                                        ));
                                        if ui.small_button("Use slot").clicked() {
                                            selected_suggested_slot = Some(slot.clone());
                                        }
                                    });
                                }
                                if slots.len() > 20 {
                                    ui.small(format!(
                                        "{} more candidate slots hidden",
                                        slots.len() - 20
                                    ));
                                }
                            }
                            Err(error) => {
                                ui.small(format!("Slot search unavailable: {error}"));
                            }
                        }

                        ui.add_space(4.0);
                        ui.collapsing("Raw free intervals", |ui| {
                            if result.free.is_empty() {
                                ui.small("No free time in the current visible window.");
                            }
                            for interval in result.free.iter().take(24) {
                                ui.horizontal_wrapped(|ui| {
                                    ui.monospace(format_availability_interval(
                                        interval.start_utc,
                                        interval.end_utc,
                                        timezone,
                                    ));
                                    let minutes =
                                        (interval.end_utc - interval.start_utc).num_minutes();
                                    ui.small(format!("{minutes} min"));
                                    if ui.small_button("New event here").clicked() {
                                        selected_free_interval = Some(interval.clone());
                                    }
                                });
                            }
                            if result.free.len() > 24 {
                                ui.small(format!(
                                    "{} more free intervals hidden in this long view",
                                    result.free.len() - 24
                                ));
                            }
                        });

                        ui.add_space(4.0);
                        ui.collapsing("Busy intervals", |ui| {
                            if result.busy.is_empty() {
                                ui.small("No blocking commitments in the current visible window.");
                            }
                            for interval in result.busy.iter().take(24) {
                                let kind = match interval.kind {
                                    BusyKind::Busy => "busy",
                                    BusyKind::Tentative => "tentative",
                                };
                                ui.horizontal_wrapped(|ui| {
                                    ui.monospace(format_availability_interval(
                                        interval.start_utc,
                                        interval.end_utc,
                                        timezone,
                                    ));
                                    ui.label(&interval.event_title);
                                    ui.small(kind);
                                });
                            }
                            if result.busy.len() > 24 {
                                ui.small(format!(
                                    "{} more busy intervals hidden in this long view",
                                    result.busy.len() - 24
                                ));
                            }
                        });

                        if !result.skipped.is_empty() {
                            ui.add_space(4.0);
                            ui.collapsing("Not used for free/busy", |ui| {
                                for skip in result.skipped.iter().take(12) {
                                    let title = visible_events
                                        .iter()
                                        .find(|event| event.id == skip.event_id)
                                        .map_or_else(
                                            || skip.event_id.to_string(),
                                            |event| event.normalized_title.clone(),
                                        );
                                    ui.horizontal_wrapped(|ui| {
                                        if ui.small_button(&title).clicked() {
                                            selected_uncheckable_event = Some(skip.event_id);
                                        }
                                        ui.small(&skip.reason);
                                    });
                                }
                                if result.skipped.len() > 12 {
                                    ui.small(format!(
                                        "{} more skipped values hidden",
                                        result.skipped.len() - 12
                                    ));
                                }
                            });
                        }
                    }
                    Err(error) => {
                        ui.colored_label(
                            Color32::LIGHT_RED,
                            format!("Availability calculation failed: {error:#}"),
                        );
                    }
                }
            },
        );

        if availability_preferences_changed {
            self.mark_state_dirty();
        }
        if let Some(id) = selected_uncheckable_event {
            self.state.selected_event_id = Some(self.canonical_event_id(id));
            self.state.show_inspector = true;
            self.mark_state_dirty();
        }

        let draft_result = if let Some(interval) = selected_suggested_slot {
            new_event_draft_for_suggested_slot(&interval, timezone)
        } else if let Some(interval) = selected_free_interval {
            new_event_draft_for_free_interval(&interval, timezone)
        } else {
            return;
        };

        match draft_result {
            Ok(draft) => {
                self.new_local_event = Some(draft);
                self.last_message =
                    Some("Prepared a new local event in the selected free interval.".to_string());
                self.last_error = None;
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Could not use free interval: {error:#}"));
            }
        }
    }

    fn render_notification_center(&mut self, ui: &mut egui::Ui) {
        if self.notification_rules.is_empty()
            && self.notification_deliveries.is_empty()
            && self.notification_snoozed_deliveries.is_empty()
        {
            return;
        }

        let now = Utc::now();
        let timezone = self.timezone();
        let deliveries = self.notification_deliveries.clone();
        let snoozed_rows = self.notification_snoozed_deliveries.clone();
        let upcoming_rows = self
            .notification_occurrences
            .iter()
            .filter(|occurrence| occurrence.trigger_at_utc > now)
            .cloned()
            .collect::<Vec<_>>();
        let skipped_count = self.notification_skipped.len();
        let mut dismiss = None;
        let mut snooze = None;
        let mut wake = None;

        ui.collapsing(
            format!(
                "Reminders · {} due · {} snoozed · {} upcoming",
                deliveries.len(),
                snoozed_rows.len(),
                upcoming_rows.len()
            ),
            |ui| {
                if deliveries.is_empty() && snoozed_rows.is_empty() && upcoming_rows.is_empty() {
                    ui.small("No timed reminders fall within the next seven days.");
                }

                ui.horizontal_wrapped(|ui| {
                    ui.small("Custom snooze");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.notification_snooze_minutes)
                            .desired_width(64.0)
                            .hint_text("minutes"),
                    );
                    match parse_snooze_minutes(&self.notification_snooze_minutes) {
                        Ok(minutes) => {
                            ui.small(format!("{minutes}m"));
                        }
                        Err(_) => {
                            ui.colored_label(Color32::LIGHT_RED, "1–10080 minutes");
                        }
                    }
                });

                for delivery in deliveries.iter().take(12) {
                    let trigger = delivery.trigger_at_utc.with_timezone(&timezone);
                    let start = delivery.starts_at_utc.with_timezone(&timezone);
                    ui.group(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new("DUE").color(Color32::LIGHT_RED).strong());
                            ui.strong(&delivery.event_title);
                            if ui.small_button("10m").on_hover_text("Snooze 10 minutes").clicked() {
                                snooze = Some((delivery.id, 10));
                            }
                            if ui.small_button("30m").on_hover_text("Snooze 30 minutes").clicked() {
                                snooze = Some((delivery.id, 30));
                            }
                            if ui.small_button("1h").on_hover_text("Snooze 1 hour").clicked() {
                                snooze = Some((delivery.id, 60));
                            }
                            let custom_minutes =
                                parse_snooze_minutes(&self.notification_snooze_minutes).ok();
                            if ui
                                .add_enabled(
                                    custom_minutes.is_some(),
                                    egui::Button::new("Custom").small(),
                                )
                                .on_hover_text("Snooze by the custom interval above")
                                .clicked()
                                && let Some(minutes) = custom_minutes
                            {
                                snooze = Some((delivery.id, minutes));
                            }
                            if ui.small_button("Dismiss").clicked() {
                                dismiss = Some(delivery.id);
                            }
                        });
                        ui.small(format!(
                            "{} · trigger {} · starts {} · {}m lead",
                            delivery.rule_name,
                            trigger.format("%Y-%m-%d %H:%M"),
                            start.format("%Y-%m-%d %H:%M"),
                            delivery.lead_minutes
                        ));
                    });
                }
                if deliveries.len() > 12 {
                    ui.small(format!(
                        "{} more undismissed reminders",
                        deliveries.len() - 12
                    ));
                }

                for delivery in snoozed_rows.iter().take(12) {
                    let resume_at = delivery
                        .snoozed_until
                        .expect("snoozed delivery has wake deadline")
                        .with_timezone(&timezone);
                    ui.group(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new("SNOOZED").strong());
                            ui.strong(&delivery.event_title);
                            if ui.small_button("Wake now").clicked() {
                                wake = Some(delivery.id);
                            }
                            if ui.small_button("Dismiss").clicked() {
                                dismiss = Some(delivery.id);
                            }
                        });
                        ui.small(format!(
                            "{} · due again {}",
                            delivery.rule_name,
                            resume_at.format("%Y-%m-%d %H:%M")
                        ));
                    });
                }
                if snoozed_rows.len() > 12 {
                    ui.small(format!(
                        "{} more snoozed reminders",
                        snoozed_rows.len() - 12
                    ));
                }

                for occurrence in upcoming_rows.iter().take(12) {
                    let trigger = occurrence.trigger_at_utc.with_timezone(&timezone);
                    let start = occurrence.starts_at_utc.with_timezone(&timezone);
                    ui.group(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new("UPCOMING").strong());
                            ui.strong(&occurrence.event_title);
                        });
                        ui.small(format!(
                            "{} · trigger {} · starts {} · {}m lead",
                            occurrence.rule_name,
                            trigger.format("%Y-%m-%d %H:%M"),
                            start.format("%Y-%m-%d %H:%M"),
                            occurrence.lead_minutes
                        ));
                    });
                }
                if upcoming_rows.len() > 12 {
                    ui.small(format!(
                        "{} more upcoming reminders in the seven-day evaluation window",
                        upcoming_rows.len() - 12
                    ));
                }
                if skipped_count != 0 {
                    ui.small(format!(
                        "{skipped_count} rule/event matches are not schedulable yet because their temporal kind has no explicit notification clock."
                    ));
                }
            },
        );

        if let Some((delivery_id, minutes)) = snooze {
            self.snooze_notification_delivery(delivery_id, minutes);
        } else if let Some(delivery_id) = wake {
            self.wake_notification_delivery(delivery_id);
        } else if let Some(delivery_id) = dismiss {
            self.dismiss_notification_delivery(delivery_id);
        }
    }

    fn render_event_notification_rules(
        &mut self,
        ui: &mut egui::Ui,
        event_id: Uuid,
        event_title: &str,
        time: &TimeSpec,
    ) {
        let rules = self
            .notification_rules
            .iter()
            .filter(|rule| {
                matches!(
                    rule.target,
                    NotificationTarget::Event {
                        event_id: target
                    } if target == event_id
                )
            })
            .cloned()
            .collect::<Vec<_>>();

        ui.separator();
        ui.strong("Reminders");
        ui.small(
            "Reminders are Ephemeris-local and can attach to imported/read-only events without modifying their source.",
        );

        let mut toggle = None;
        let mut delete = None;
        for rule in rules {
            ui.horizontal_wrapped(|ui| {
                let mut enabled = rule.enabled;
                if ui.checkbox(&mut enabled, "").changed() {
                    toggle = Some((rule.id, enabled));
                }
                ui.label(format!(
                    "{} · {}m before start",
                    rule.name,
                    rule.trigger.lead_minutes()
                ));
                if ui.small_button("Delete").clicked() {
                    delete = Some(rule.id);
                }
            });
        }

        let timed = matches!(time, TimeSpec::Instant { .. } | TimeSpec::Floating { .. });
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled(
                timed,
                egui::TextEdit::singleline(&mut self.notification_event_lead_minutes)
                    .desired_width(70.0)
                    .hint_text("minutes"),
            );
            if ui
                .add_enabled(timed, egui::Button::new("Add reminder"))
                .clicked()
            {
                self.create_event_notification_rule(event_id, event_title);
            }
            ui.small("minutes before");
        });
        if !timed {
            ui.small(
                "Before-start reminders currently require an exact or floating DATE-TIME. All-day/date-only events need an explicit notification clock before they can be scheduled.",
            );
        }

        if let Some((rule_id, enabled)) = toggle {
            self.set_notification_rule_enabled(rule_id, enabled);
        }
        if let Some(rule_id) = delete {
            self.delete_notification_rule(rule_id);
        }
    }

    fn render_saved_view_notification_rules(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.strong("Saved-view reminders");
        ui.small(
            "A saved-view rule applies the same query, composition, source visibility, and membership semantics as the calendar view.",
        );

        if self.notification_saved_view_id.is_none() {
            self.notification_saved_view_id = self.saved_views.first().map(|view| view.id);
        }

        let selected_name = self
            .notification_saved_view_id
            .and_then(|id| self.saved_views.iter().find(|view| view.id == id))
            .map_or("Select saved view…", |view| view.name.as_str());
        egui::ComboBox::from_id_salt("notification-saved-view-target")
            .selected_text(selected_name)
            .show_ui(ui, |ui| {
                for view in &self.saved_views {
                    ui.selectable_value(
                        &mut self.notification_saved_view_id,
                        Some(view.id),
                        &view.name,
                    );
                }
            });

        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.notification_saved_view_lead_minutes)
                    .desired_width(70.0)
                    .hint_text("minutes"),
            );
            if ui
                .add_enabled(
                    self.notification_saved_view_id.is_some(),
                    egui::Button::new("Add saved-view reminder"),
                )
                .clicked()
            {
                self.create_saved_view_notification_rule();
            }
            ui.small("minutes before each matching timed occurrence");
        });

        let rows = self
            .notification_rules
            .iter()
            .filter_map(|rule| match rule.target {
                NotificationTarget::SavedView { saved_view_id } => {
                    Some((rule.clone(), saved_view_id))
                }
                NotificationTarget::Event { .. } => None,
            })
            .collect::<Vec<_>>();

        let mut toggle = None;
        let mut delete = None;
        for (rule, saved_view_id) in rows {
            let view_name = self
                .saved_views
                .iter()
                .find(|view| view.id == saved_view_id)
                .map_or_else(|| saved_view_id.to_string(), |view| view.name.clone());
            ui.horizontal_wrapped(|ui| {
                let mut enabled = rule.enabled;
                if ui.checkbox(&mut enabled, "").changed() {
                    toggle = Some((rule.id, enabled));
                }
                ui.label(format!(
                    "{view_name} · {}m before",
                    rule.trigger.lead_minutes()
                ));
                if ui.small_button("Delete").clicked() {
                    delete = Some(rule.id);
                }
            });
        }

        if let Some((rule_id, enabled)) = toggle {
            self.set_notification_rule_enabled(rule_id, enabled);
        }
        if let Some(rule_id) = delete {
            self.delete_notification_rule(rule_id);
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
            if ui.button("New event").clicked() {
                self.begin_new_local_event();
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

        if self.new_local_event.is_some() {
            let timezone = self.timezone();
            let timezone_name = self.state.display_timezone.clone();
            let mut save = false;
            let mut cancel = false;
            let mut selected_alternative = None;
            ui.group(|ui| {
                ui.strong("New local event");
                ui.small(format!(
                    "Timed events use display timezone {timezone_name}. Recurrence and richer metadata can be added after creation."
                ));
                let draft = self.new_local_event.as_mut().expect("checked above");
                ui.add(
                    egui::TextEdit::singleline(&mut draft.title)
                        .hint_text("Title")
                        .desired_width(260.0),
                );
                ui.add(
                    egui::TextEdit::multiline(&mut draft.description)
                        .desired_rows(2)
                        .hint_text("Description (optional)")
                        .desired_width(320.0),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.event_type)
                            .hint_text("Event type (optional)")
                            .desired_width(150.0),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.domain)
                            .hint_text("Domain (optional)")
                            .desired_width(150.0),
                    );
                });
                ui.collapsing("Participants and location (optional)", |ui| {
                    ui.small("Participant names are local event metadata, not invitations. One name per line.");
                    ui.add(
                        egui::TextEdit::multiline(&mut draft.participant_names)
                            .desired_rows(2)
                            .hint_text("Participant names")
                            .desired_width(320.0),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.location_name)
                            .hint_text("Venue or location name")
                            .desired_width(260.0),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.location_address)
                            .hint_text("Address")
                            .desired_width(320.0),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.location_virtual_url)
                            .hint_text("Virtual meeting URL")
                            .desired_width(320.0),
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    egui::ComboBox::from_id_salt("new-local-event-status")
                        .selected_text(draft.status.as_str())
                        .show_ui(ui, |ui| {
                            for status in EventStatus::ALL {
                                ui.selectable_value(&mut draft.status, status, status.as_str());
                            }
                        });
                    egui::ComboBox::from_id_salt("new-local-event-availability")
                        .selected_text(match draft.availability {
                            AvailabilityBehavior::Busy => "Busy",
                            AvailabilityBehavior::Free => "Free",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut draft.availability,
                                AvailabilityBehavior::Busy,
                                "Busy · blocks availability",
                            );
                            ui.selectable_value(
                                &mut draft.availability,
                                AvailabilityBehavior::Free,
                                "Free · does not block availability",
                            );
                        });
                });
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.date)
                            .hint_text("YYYY-MM-DD")
                            .desired_width(110.0),
                    );
                    ui.checkbox(&mut draft.all_day, "All day");
                    if !draft.all_day {
                        ui.add(
                            egui::TextEdit::singleline(&mut draft.start_time)
                                .hint_text("HH:MM")
                                .desired_width(75.0),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut draft.duration_minutes)
                                .hint_text("minutes")
                                .desired_width(80.0),
                        );
                        ui.small("min");
                    }
                });
                if draft.all_day {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("End (exclusive)");
                        ui.add(
                            egui::TextEdit::singleline(&mut draft.end_date)
                                .hint_text("YYYY-MM-DD · blank = one day")
                                .desired_width(210.0),
                        );
                    });
                }
                if let Some(confirmation) = &draft.conflict_confirmation {
                    ui.colored_label(Color32::YELLOW, &confirmation.warning);
                    selected_alternative =
                        render_conflict_alternatives(ui, confirmation, timezone);
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !draft.title.trim().is_empty(),
                            egui::Button::new("Create event"),
                        )
                        .clicked()
                    {
                        save = true;
                    }
                    if ui.small_button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
            if let Some(slot) = selected_alternative {
                let timezone = self.timezone();
                if let Some(draft) = self.new_local_event.as_mut() {
                    match apply_alternative_to_new_draft(draft, &slot, timezone) {
                        Ok(()) => self.last_error = None,
                        Err(error) => {
                            self.last_error = Some(format!("Cannot use alternate slot: {error:#}"));
                        }
                    }
                }
            } else if save {
                self.save_new_local_event();
            } else if cancel {
                self.new_local_event = None;
                self.last_error = None;
            }
        }

        self.render_notification_center(ui);
        self.render_availability_panel(ui);

        if let Some(message) = self.last_message.as_deref() {
            ui.colored_label(Color32::LIGHT_GREEN, message);
        }
        if let Some(error) = self.last_error.as_deref() {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
    }

    fn render_calendar_refresh_history(&self, ui: &mut egui::Ui) {
        let attempts = self
            .source_refresh_attempts
            .iter()
            .filter(|attempt| attempt.refresh_kind != "taria_workspace")
            .collect::<Vec<_>>();

        ui.collapsing(
            format!("External refresh history ({})", attempts.len()),
            |ui| {
                if attempts.is_empty() {
                    ui.small("No persisted ICS/Webcal/CSV refresh attempts yet.");
                    return;
                }

                for attempt in attempts {
                    let status = match attempt.success {
                        Some(true) => "success",
                        Some(false) => "failed",
                        None => "incomplete",
                    };
                    let kind = match attempt.refresh_kind.as_str() {
                        "ics_file" => "local ICS",
                        "webcal" => "remote Webcal",
                        "csv_file" => "local CSV",
                        other => other,
                    };
                    ui.strong(format!("{status} · {kind}"));
                    ui.small(format!("Target {}", attempt.target));
                    ui.small(format!("Started {}", attempt.started_at));
                    if let Some(completed_at) = attempt.completed_at.as_deref() {
                        ui.small(format!("Completed {completed_at}"));
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
    }

    fn begin_canonical_entity_edit(&mut self, entity_id: Uuid) {
        if let Some(entity) = self
            .canonical_entities
            .iter()
            .find(|entity| entity.id == entity_id)
            .cloned()
        {
            self.entity_manage_id = Some(entity.id);
            self.entity_manage_name = entity.canonical_name;
            self.entity_manage_type = entity.entity_type.unwrap_or_default();
            self.entity_manage_aliases = entity.aliases.join("\n");
            self.entity_manage_external_refs = entity.external_refs.join("\n");
            self.entity_merge_target_id = None;
        }
    }

    fn begin_new_canonical_entity(&mut self) {
        self.entity_manage_id = None;
        self.entity_manage_name.clear();
        self.entity_manage_type.clear();
        self.entity_manage_aliases.clear();
        self.entity_manage_external_refs.clear();
        self.entity_merge_target_id = None;
    }

    fn save_managed_canonical_entity(&mut self) {
        let result = (|| -> anyhow::Result<CanonicalEntity> {
            let mut entity = match self.entity_manage_id {
                Some(entity_id) => {
                    self.store
                        .canonical_entity_by_id(entity_id)?
                        .ok_or_else(|| {
                            anyhow::anyhow!("canonical entity {entity_id} no longer exists")
                        })?
                }
                None => CanonicalEntity::new(self.entity_manage_name.trim()),
            };

            entity.canonical_name = self.entity_manage_name.trim().to_string();
            entity.entity_type = optional_trimmed(&self.entity_manage_type);
            entity.aliases = multiline_values(&self.entity_manage_aliases);
            entity.external_refs = multiline_values(&self.entity_manage_external_refs);
            entity.updated_at = Utc::now();
            entity.validate()?;
            self.store.upsert_canonical_entity(&entity)?;
            Ok(entity)
        })();

        match result {
            Ok(entity) => {
                self.last_message = Some(format!(
                    "Saved canonical entity {:?}.",
                    entity.canonical_name
                ));
                self.last_error = None;
                self.entity_manage_id = Some(entity.id);
                self.reload_or_report();
                self.begin_canonical_entity_edit(entity.id);
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Canonical entity save failed: {error:#}"));
            }
        }
    }

    fn merge_managed_canonical_entity(&mut self) {
        let (Some(source_id), Some(target_id)) =
            (self.entity_manage_id, self.entity_merge_target_id)
        else {
            return;
        };

        match self.store.merge_canonical_entities(target_id, source_id) {
            Ok(merged) => {
                self.last_message = Some(format!(
                    "Merged canonical entity into {:?}. Participant bindings and references were preserved.",
                    merged.canonical_name
                ));
                self.last_error = None;
                self.entity_manage_id = Some(merged.id);
                self.entity_merge_target_id = None;
                self.reload_or_report();
                self.begin_canonical_entity_edit(merged.id);
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Canonical entity merge failed: {error:#}"));
            }
        }
    }

    fn delete_managed_canonical_entity(&mut self) {
        let Some(entity_id) = self.entity_manage_id else {
            return;
        };
        match self.store.delete_canonical_entity(entity_id) {
            Ok(true) => {
                self.last_message = Some(
                    "Deleted canonical entity. Manual participant bindings were removed; source participant refs were left untouched."
                        .to_string(),
                );
                self.last_error = None;
                self.begin_new_canonical_entity();
                self.reload_or_report();
            }
            Ok(false) => {
                self.last_message = None;
                self.last_error = Some("Canonical entity no longer exists.".to_string());
                self.begin_new_canonical_entity();
                self.reload_or_report();
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Canonical entity delete failed: {error:#}"));
            }
        }
    }

    fn render_canonical_entity_registry(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        egui::CollapsingHeader::new(format!(
            "Canonical Entities ({})",
            self.canonical_entities.len()
        ))
        .default_open(false)
        .show(ui, |ui| {
            ui.small(
                "Durable people, organizations, teams, places, and other entities used by participant resolution.",
            );
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.entity_registry_search)
                        .hint_text("Search entities")
                        .desired_width(170.0),
                );
                if ui.button("New").clicked() {
                    self.begin_new_canonical_entity();
                }
            });

            let query = self.entity_registry_search.trim().to_lowercase();
            let visible = self
                .canonical_entities
                .iter()
                .filter(|entity| {
                    query.is_empty()
                        || entity
                            .text_values()
                            .any(|value| value.to_lowercase().contains(&query))
                })
                .take(24)
                .cloned()
                .collect::<Vec<_>>();
            if visible.is_empty() {
                ui.small("No canonical entities match.");
            } else {
                egui::ScrollArea::vertical()
                    .max_height(150.0)
                    .show(ui, |ui| {
                        for entity in visible {
                            let label = entity.entity_type.as_deref().map_or_else(
                                || entity.canonical_name.clone(),
                                |kind| format!("{} · {kind}", entity.canonical_name),
                            );
                            if ui
                                .selectable_label(
                                    self.entity_manage_id == Some(entity.id),
                                    label,
                                )
                                .clicked()
                            {
                                self.begin_canonical_entity_edit(entity.id);
                            }
                        }
                    });
            }

            if self.entity_manage_id.is_some() || !self.entity_manage_name.is_empty() {
                ui.separator();
                ui.strong(if self.entity_manage_id.is_some() {
                    "Edit canonical entity"
                } else {
                    "New canonical entity"
                });
                if let Some(entity_id) = self.entity_manage_id
                    && let Some(entity) = self
                        .canonical_entities
                        .iter()
                        .find(|entity| entity.id == entity_id)
                {
                    ui.small(format!("Local ref: {}", entity.local_reference()));
                    let usage = self
                        .canonical_entity_usage
                        .get(&entity_id)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]);
                    ui.small(format!(
                        "Resolved participant usage: {} canonical event{}",
                        usage.len(),
                        if usage.len() == 1 { "" } else { "s" }
                    ));
                    for (_, title) in usage.iter().take(8) {
                        ui.small(format!("• {title}"));
                    }
                    if usage.len() > 8 {
                        ui.small(format!("… and {} more", usage.len() - 8));
                    }
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.entity_manage_name)
                        .hint_text("Canonical name"),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.entity_manage_type)
                        .hint_text("Type: person, organization, team, place, ..."),
                );
                ui.label("Aliases");
                ui.add(
                    egui::TextEdit::multiline(&mut self.entity_manage_aliases)
                        .desired_rows(3)
                        .hint_text("One alias per line"),
                );
                ui.label("External refs");
                ui.add(
                    egui::TextEdit::multiline(&mut self.entity_manage_external_refs)
                        .desired_rows(3)
                        .hint_text("wikidata:Q…\nsource:stable-id"),
                );

                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !self.entity_manage_name.trim().is_empty(),
                            egui::Button::new("Save entity"),
                        )
                        .clicked()
                    {
                        self.save_managed_canonical_entity();
                    }
                    if self.entity_manage_id.is_some()
                        && ui
                            .button("Delete")
                            .on_hover_text(
                                "Deletes the registry entity and local manual bindings. Source participant refs are preserved.",
                            )
                            .clicked()
                    {
                        self.delete_managed_canonical_entity();
                    }
                    if ui.small_button("Clear editor").clicked() {
                        self.begin_new_canonical_entity();
                    }
                });

                if let Some(source_id) = self.entity_manage_id {
                    ui.separator();
                    ui.strong("Merge entity");
                    ui.small(
                        "Merge this entity into another canonical entity. Aliases, external refs, and local participant bindings transfer; conflicting types or opaque properties are rejected.",
                    );

                    let targets = self
                        .canonical_entities
                        .iter()
                        .filter(|entity| entity.id != source_id)
                        .cloned()
                        .collect::<Vec<_>>();
                    let selected = self
                        .entity_merge_target_id
                        .and_then(|target_id| {
                            targets
                                .iter()
                                .find(|entity| entity.id == target_id)
                                .map(|entity| {
                                    entity.entity_type.as_deref().map_or_else(
                                        || entity.canonical_name.clone(),
                                        |kind| format!("{} · {kind}", entity.canonical_name),
                                    )
                                })
                        })
                        .unwrap_or_else(|| "Select merge target…".to_string());

                    egui::ComboBox::from_id_salt(("canonical-entity-merge-target", source_id))
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            for entity in &targets {
                                let label = entity.entity_type.as_deref().map_or_else(
                                    || entity.canonical_name.clone(),
                                    |kind| format!("{} · {kind}", entity.canonical_name),
                                );
                                ui.selectable_value(
                                    &mut self.entity_merge_target_id,
                                    Some(entity.id),
                                    label,
                                );
                            }
                        });

                    if ui
                        .add_enabled(
                            self.entity_merge_target_id.is_some(),
                            egui::Button::new("Merge current into target"),
                        )
                        .on_hover_text(
                            "The current entity is deleted after its aliases, refs, and local bindings are transferred. Source participant text is never rewritten.",
                        )
                        .clicked()
                    {
                        self.merge_managed_canonical_entity();
                    }
                }
            }
        });
    }

    fn render_sources(&mut self, ui: &mut egui::Ui) {
        ui.set_width(280.0);

        self.render_taria_workspace(ui);
        ui.separator();
        ui.heading("Remote iCalendar");
        ui.small("Subscribe to an HTTP, HTTPS, webcal, or webcals calendar feed.");
        let importing_remote = self.remote_ics_import_receiver.is_some();
        ui.horizontal(|ui| {
            let response = ui.add_enabled(
                !importing_remote,
                egui::TextEdit::singleline(&mut self.remote_ics_url)
                    .hint_text("https://…/calendar.ics")
                    .desired_width(190.0),
            );
            let submit =
                response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            let ready = !self.remote_ics_url.trim().is_empty() && !importing_remote;
            if ui
                .add_enabled(
                    ready,
                    egui::Button::new(if importing_remote {
                        "Importing..."
                    } else {
                        "Add feed"
                    }),
                )
                .clicked()
                || (submit && ready)
            {
                self.start_remote_ics_import(self.remote_ics_url.clone());
            }
        });
        if importing_remote {
            ui.small("Fetching and importing in the background...");
        }
        self.render_calendar_refresh_history(ui);
        self.render_canonical_entity_registry(ui);

        let membership_options = MembershipPredicateOptions {
            bundles: self.taria_bundle_refs.clone(),
            calendars: self.taria_calendar_choices.clone(),
            collections: self.event_collections.clone(),
            relation_types: self.event_relation_types.clone(),
            annotation_kinds: self.event_annotation_kinds.clone(),
            canonical_entities: self.canonical_entities.clone(),
            canonical_entity_types: self
                .canonical_entities
                .iter()
                .filter_map(|entity| entity.entity_type.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
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

        self.render_saved_view_notification_rules(ui);

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

        let grouping_locked = matches!(
            self.state.calendar_layout,
            CalendarLayout::Stream | CalendarLayout::Timeline | CalendarLayout::Density
        );
        let sorting_locked = matches!(
            self.state.calendar_layout,
            CalendarLayout::Stream
                | CalendarLayout::Timeline
                | CalendarLayout::Density
                | CalendarLayout::Summary
        );
        let aggregate_layout = matches!(
            self.state.calendar_layout,
            CalendarLayout::Density | CalendarLayout::Summary
        );

        match self.state.calendar_layout {
            CalendarLayout::Density => {
                ui.small(
                    "Density aggregates by day. Saved grouping, sort, and color settings are preserved but do not alter this aggregate layout.",
                );
            }
            CalendarLayout::Summary => {
                ui.small(
                    "Summary uses Group by as its pivot dimension. Saved sort and color settings are preserved but do not alter aggregate rows.",
                );
            }
            CalendarLayout::Stream | CalendarLayout::Timeline => {
                ui.small(
                    "Stream/Timeline own temporal ordering. Saved grouping and sort rules are preserved for Agenda/Compact/Table but do not alter these chronological layouts.",
                );
            }
            _ => {}
        }

        ui.add_enabled_ui(!grouping_locked, |ui| {
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

        ui.add_enabled_ui(!aggregate_layout, |ui| {
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
        if aggregate_layout {
            ui.small(
                "Aggregate layouts do not apply per-event color rules. Saved color settings are preserved for other layouts.",
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
        ui.add_enabled_ui(!sorting_locked, |ui| {
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
                    if source.kind == crate::domain::SourceKind::Webcal {
                        inspector_row(ui, "Locator", &remote_locator_display(locator));
                    } else {
                        inspector_row(ui, "Locator", locator);
                    }
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
                if matches!(
                    source.kind,
                    crate::domain::SourceKind::Ics | crate::domain::SourceKind::Webcal
                ) {
                    if self.ics_export_source_id != Some(source.id) {
                        self.ics_export_source_id = Some(source.id);
                        self.ics_export_path = default_ics_export_path(source);
                    }

                    ui.separator();
                    ui.strong("iCalendar source");
                    if let Some(locator) = source.locator.as_deref() {
                        if source.kind == crate::domain::SourceKind::Webcal {
                            ui.small("Refresh fetches the remote feed in the background using stable source and UID identity.");
                            if ui
                                .add_enabled(
                                    self.remote_ics_import_receiver.is_none(),
                                    egui::Button::new("Refresh remote feed"),
                                )
                                .clicked()
                            {
                                self.start_remote_ics_import(locator.to_string());
                            }
                        } else {
                            ui.small("Refresh re-reads the original local file using stable source and UID identity.");
                            if ui.button("Refresh ICS").clicked() {
                                self.import_ics_path(std::path::Path::new(locator));
                            }
                        }
                    } else {
                        ui.small("This iCalendar source has no refresh locator.");
                    }

                    ui.add_space(4.0);
                    ui.small("Export writes the current canonical source state to a new .ics file.");
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.ics_export_path)
                            .hint_text("/path/to/export.ics")
                            .desired_width(250.0),
                    );
                    let submit = response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    let export_enabled = !self.ics_export_path.trim().is_empty();
                    if ui
                        .add_enabled(export_enabled, egui::Button::new("Export ICS"))
                        .clicked()
                        || (submit && export_enabled)
                    {
                        self.export_ics_source_to_path(source.id);
                    }
                }

                if source.kind == crate::domain::SourceKind::Csv {
                    if self.csv_export_source_id != Some(source.id) {
                        self.csv_export_source_id = Some(source.id);
                        self.csv_export_path = default_csv_export_path(source);
                    }

                    ui.separator();
                    ui.strong("CSV source");
                    if let Some(locator) = source.locator.as_deref() {
                        ui.small("Refresh re-reads the original CSV using stable source and record_key identity.");
                        if ui.button("Refresh CSV").clicked() {
                            self.import_csv_path(std::path::Path::new(locator));
                        }
                    } else {
                        ui.small("This CSV source has no local file locator.");
                    }

                    ui.add_space(4.0);
                    ui.small("CSV v1 is strict: unsupported recurrence or richer metadata makes export fail rather than flattening it.");
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.csv_export_path)
                            .hint_text("/path/to/export.csv")
                            .desired_width(250.0),
                    );
                    let submit = response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    let export_enabled = !self.csv_export_path.trim().is_empty();
                    if ui
                        .add_enabled(export_enabled, egui::Button::new("Export CSV"))
                        .clicked()
                        || (submit && export_enabled)
                    {
                        self.export_csv_source_to_path(source.id);
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
                                    memberships: &self.event_memberships,
                                    occurrence_contexts: &self.occurrence_contexts,
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

    fn reset_topology_editor(&mut self, event_id: Uuid) {
        self.topology_edit_event_id = Some(event_id);
        self.topology_collection_choice = None;
        self.topology_new_collection_name.clear();
        self.topology_new_collection_ordered = false;
        self.topology_manage_collection_id = None;
        self.topology_manage_collection_name.clear();
        self.topology_manage_collection_description.clear();
        self.topology_manage_collection_ordered = false;
        self.topology_relation_type.clear();
        self.topology_relation_outgoing = true;
        self.topology_relation_search.clear();
        self.topology_relation_candidates.clear();
        self.topology_relation_target_id = None;
        self.identity_search.clear();
        self.identity_candidates.clear();
        self.identity_target_id = None;
        self.identity_new_state = EventIdentityState::Candidate;
        self.identity_confidence_text.clear();
        self.identity_rationale.clear();
        self.annotation_new_kind = "note".to_string();
        self.annotation_new_value.clear();
        self.provenance_new_role = EventProvenanceRole::Provenance;
        self.provenance_new_reference.clear();
        self.provenance_new_note.clear();
    }

    fn refresh_topology_relation_candidates(&mut self, event_id: Uuid) {
        match self
            .store
            .search_event_titles(&self.topology_relation_search, Some(event_id), 20)
        {
            Ok(candidates) => {
                if self
                    .topology_relation_target_id
                    .is_some_and(|selected| !candidates.iter().any(|(id, _)| *id == selected))
                {
                    self.topology_relation_target_id = None;
                }
                self.topology_relation_candidates = candidates;
            }
            Err(error) => {
                self.topology_relation_candidates.clear();
                self.topology_relation_target_id = None;
                self.last_error = Some(format!("Failed to search relation targets: {error:#}"));
            }
        }
    }

    fn refresh_identity_candidates(&mut self, event_id: Uuid) {
        match self
            .store
            .search_event_titles(&self.identity_search, Some(event_id), 20)
        {
            Ok(candidates) => {
                if self
                    .identity_target_id
                    .is_some_and(|selected| !candidates.iter().any(|(id, _)| *id == selected))
                {
                    self.identity_target_id = None;
                }
                self.identity_candidates = candidates;
            }
            Err(error) => {
                self.identity_candidates.clear();
                self.identity_target_id = None;
                self.last_error = Some(format!("Failed to search identity candidates: {error:#}"));
            }
        }
    }

    fn apply_topology_action(&mut self, event_id: Uuid, action: TopologyInspectorAction) {
        let result = match action {
            TopologyInspectorAction::AddToCollection(collection_id) => {
                self.add_event_to_collection(event_id, collection_id)
            }
            TopologyInspectorAction::RemoveFromCollection(collection_id) => {
                self.remove_event_from_collection(event_id, collection_id)
            }
            TopologyInspectorAction::MoveCollectionMemberUp(collection_id) => {
                self.move_event_in_collection(event_id, collection_id, -1)
            }
            TopologyInspectorAction::MoveCollectionMemberDown(collection_id) => {
                self.move_event_in_collection(event_id, collection_id, 1)
            }
            TopologyInspectorAction::CreateCollection => self.create_collection_for_event(event_id),
            TopologyInspectorAction::SaveCollection(collection_id) => {
                self.save_managed_collection(collection_id)
            }
            TopologyInspectorAction::DeleteCollection(collection_id) => {
                self.delete_managed_collection(collection_id)
            }
            TopologyInspectorAction::AddRelation => self.create_relation_for_event(event_id),
            TopologyInspectorAction::DeleteRelation(relation_id) => {
                self.store.delete_event_relation(relation_id).map(|_| ())
            }
            TopologyInspectorAction::SetIdentityState(assessment_id, state) => {
                self.set_identity_assessment_state(assessment_id, state)
            }
            TopologyInspectorAction::DeleteIdentityAssessment(assessment_id) => self
                .store
                .delete_event_identity_assessment(assessment_id)
                .map(|_| ()),
            TopologyInspectorAction::AddIdentityAssessment => {
                self.create_identity_assessment_for_event(event_id)
            }
            TopologyInspectorAction::AddAnnotation => self.create_annotation_for_event(event_id),
            TopologyInspectorAction::DeleteAnnotation(annotation_id) => self
                .store
                .delete_event_annotation(annotation_id)
                .map(|_| ()),
            TopologyInspectorAction::AddProvenance => self.create_provenance_for_event(event_id),
            TopologyInspectorAction::DeleteProvenance(record_id) => self
                .store
                .delete_event_provenance_record(record_id)
                .map(|_| ()),
        };

        match result {
            Ok(()) => {
                self.last_error = None;
                self.reload_or_report();
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Inspector edit failed: {error:#}"));
            }
        }
    }

    fn begin_participant_entity_resolution(
        &mut self,
        event_id: Uuid,
        participant_index: usize,
        participant: &EventParticipant,
    ) {
        self.participant_entity_edit = Some((event_id, participant_index));
        self.participant_entity_search = participant.name.clone();
        self.participant_entity_target_id = None;
        self.refresh_participant_entity_candidates();
    }

    fn refresh_participant_entity_candidates(&mut self) {
        let query = self.participant_entity_search.trim();
        if query.is_empty() {
            self.participant_entity_candidates.clear();
            self.participant_entity_target_id = None;
            return;
        }

        match self.store.search_canonical_entities(query, 12) {
            Ok(entities) => {
                self.participant_entity_candidates = entities
                    .into_iter()
                    .map(|entity| (entity.id, entity.canonical_name, entity.entity_type))
                    .collect();
                if self.participant_entity_target_id.is_some_and(|target_id| {
                    !self
                        .participant_entity_candidates
                        .iter()
                        .any(|(id, _, _)| *id == target_id)
                }) {
                    self.participant_entity_target_id = None;
                }
                self.last_error = None;
            }
            Err(error) => {
                self.participant_entity_candidates.clear();
                self.participant_entity_target_id = None;
                self.last_error = Some(format!("Entity search failed: {error:#}"));
            }
        }
    }

    fn reset_participant_entity_editor(&mut self) {
        self.participant_entity_edit = None;
        self.participant_entity_search.clear();
        self.participant_entity_candidates.clear();
        self.participant_entity_target_id = None;
    }

    fn apply_participant_action(&mut self, event_id: Uuid, action: ParticipantInspectorAction) {
        let result = (|| -> anyhow::Result<()> {
            match action {
                ParticipantInspectorAction::BindEntity(index, entity_id) => {
                    let binding = self
                        .store
                        .bind_event_participant_entity(event_id, index, entity_id)?;
                    let entity = self
                        .store
                        .canonical_entity_by_id(binding.entity_id)?
                        .ok_or_else(|| anyhow::anyhow!("bound canonical entity disappeared"))?;
                    self.last_message =
                        Some(format!("Bound participant to {:?}.", entity.canonical_name));
                    self.reset_participant_entity_editor();
                    return Ok(());
                }
                ParticipantInspectorAction::UnbindEntity(index) => {
                    self.store
                        .unbind_event_participant_entity(event_id, index)?;
                    self.last_message =
                        Some("Removed local participant entity binding.".to_string());
                    self.reset_participant_entity_editor();
                    return Ok(());
                }
                ParticipantInspectorAction::CreateEntity(index) => {
                    let event = self
                        .store
                        .event_by_id(event_id)?
                        .ok_or_else(|| anyhow::anyhow!("event {event_id} does not exist"))?;
                    let participant = event
                        .participants
                        .get(index)
                        .ok_or_else(|| anyhow::anyhow!("participant row {index} does not exist"))?;

                    let mut entity = CanonicalEntity::new(participant.name.trim());
                    entity.entity_type = participant.participant_type.clone();
                    if let Some(reference) = participant.entity_ref.as_deref()
                        && self
                            .store
                            .canonical_entity_by_reference(reference)?
                            .is_none()
                    {
                        entity.external_refs.push(reference.trim().to_string());
                    }
                    entity.validate()?;
                    self.store.upsert_canonical_entity(&entity)?;
                    self.store
                        .bind_event_participant_entity(event_id, index, entity.id)?;
                    self.last_message = Some(format!(
                        "Created and bound canonical entity {:?}.",
                        entity.canonical_name
                    ));
                    self.reset_participant_entity_editor();
                    return Ok(());
                }
                ParticipantInspectorAction::Add | ParticipantInspectorAction::Remove(_) => {}
            }

            let mut event = self
                .store
                .event_by_id(event_id)?
                .ok_or_else(|| anyhow::anyhow!("event {event_id} does not exist"))?;
            if !self.event_is_editable(&event) {
                anyhow::bail!("this event comes from a read-only source and cannot be edited");
            }

            match action {
                ParticipantInspectorAction::Add => {
                    let mut participant = EventParticipant::new(self.participant_new_name.trim());
                    participant.role = optional_trimmed(&self.participant_new_role);
                    participant.participant_type = optional_trimmed(&self.participant_new_type);
                    participant.entity_ref = optional_trimmed(&self.participant_new_entity_ref);
                    participant.validate()?;
                    event.participants.push(participant);
                    self.participant_new_name.clear();
                    self.participant_new_role.clear();
                    self.participant_new_type.clear();
                    self.participant_new_entity_ref.clear();
                    self.last_message = Some("Added event participant.".to_string());
                }
                ParticipantInspectorAction::Remove(index) => {
                    if index >= event.participants.len() {
                        anyhow::bail!("participant row no longer exists");
                    }
                    event.participants.remove(index);
                    self.last_message = Some("Removed event participant.".to_string());
                }
                ParticipantInspectorAction::BindEntity(_, _)
                | ParticipantInspectorAction::UnbindEntity(_)
                | ParticipantInspectorAction::CreateEntity(_) => unreachable!(),
            }

            event.updated_at = Utc::now();
            self.store.upsert_event(&event)
        })();

        match result {
            Ok(()) => {
                self.last_error = None;
                self.reload_or_report();
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Participant edit failed: {error:#}"));
            }
        }
    }

    fn create_provenance_for_event(&mut self, event_id: Uuid) -> anyhow::Result<()> {
        let reference = self.provenance_new_reference.trim();
        if reference.is_empty() {
            anyhow::bail!("provenance reference is empty");
        }

        let mut record = EventProvenanceRecord::new(event_id, self.provenance_new_role, reference);
        record.note = if self.provenance_new_note.trim().is_empty() {
            None
        } else {
            Some(self.provenance_new_note.trim().to_string())
        };
        self.store.upsert_event_provenance_record(&record)?;

        self.provenance_new_reference.clear();
        self.provenance_new_note.clear();
        self.last_message = Some("Created structured provenance record.".to_string());
        Ok(())
    }

    fn create_annotation_for_event(&mut self, event_id: Uuid) -> anyhow::Result<()> {
        let kind = self.annotation_new_kind.trim();
        if kind.is_empty() {
            anyhow::bail!("annotation kind is empty");
        }
        let raw_value = self.annotation_new_value.trim();
        if raw_value.is_empty() {
            anyhow::bail!(
                "annotation value is empty; enter JSON such as \"note text\", true, or 5"
            );
        }
        let value = serde_json::from_str(raw_value)
            .map_err(|error| anyhow::anyhow!("annotation value must be valid JSON: {error}"))?;
        let annotation = EventAnnotation::new(event_id, kind, value);
        self.store.upsert_event_annotation(&annotation)?;
        self.annotation_new_kind = "note".to_string();
        self.annotation_new_value.clear();
        self.last_message = Some("Created user annotation.".to_string());
        Ok(())
    }

    fn create_identity_assessment_for_event(&mut self, event_id: Uuid) -> anyhow::Result<()> {
        let target_id = self
            .identity_target_id
            .ok_or_else(|| anyhow::anyhow!("choose an identity comparison target"))?;
        if self
            .store
            .event_identity_assessment_between(event_id, target_id)?
            .is_some()
        {
            anyhow::bail!("an identity assessment for this event pair already exists");
        }

        let mut assessment = EventIdentityAssessment::new(event_id, target_id);
        assessment.state = self.identity_new_state;
        assessment.confidence = if self.identity_confidence_text.trim().is_empty() {
            None
        } else {
            Some(
                self.identity_confidence_text
                    .trim()
                    .parse::<f32>()
                    .map_err(|_| {
                        anyhow::anyhow!("identity confidence must be a number in 0..=1")
                    })?,
            )
        };
        assessment.rationale = if self.identity_rationale.trim().is_empty() {
            None
        } else {
            Some(self.identity_rationale.trim().to_string())
        };
        assessment.updated_at = Utc::now();
        self.store.upsert_event_identity_assessment(&assessment)?;

        self.identity_search.clear();
        self.identity_candidates.clear();
        self.identity_target_id = None;
        self.identity_new_state = EventIdentityState::Candidate;
        self.identity_confidence_text.clear();
        self.identity_rationale.clear();
        self.last_message = Some("Created duplicate/entity assessment.".to_string());
        Ok(())
    }

    fn set_identity_assessment_state(
        &mut self,
        assessment_id: Uuid,
        state: EventIdentityState,
    ) -> anyhow::Result<()> {
        let mut assessment = self
            .store
            .event_identity_assessment_by_id(assessment_id)?
            .ok_or_else(|| anyhow::anyhow!("identity assessment {assessment_id} does not exist"))?;
        assessment.state = state;
        assessment.updated_at = Utc::now();
        self.store.upsert_event_identity_assessment(&assessment)?;
        self.last_message = Some(format!(
            "Updated duplicate/entity assessment to {}.",
            state.as_str()
        ));
        Ok(())
    }

    fn add_event_to_collection(
        &mut self,
        event_id: Uuid,
        collection_id: Uuid,
    ) -> anyhow::Result<()> {
        let members = self.store.event_collection_members(collection_id)?;
        if members.iter().any(|member| member.event_id == event_id) {
            return Ok(());
        }
        let mut event_ids = members
            .into_iter()
            .map(|member| member.event_id)
            .collect::<Vec<_>>();
        event_ids.push(event_id);
        self.store
            .replace_event_collection_members(collection_id, &event_ids)?;
        self.last_message = Some("Added event to collection.".to_string());
        Ok(())
    }

    fn remove_event_from_collection(
        &mut self,
        event_id: Uuid,
        collection_id: Uuid,
    ) -> anyhow::Result<()> {
        let event_ids = self
            .store
            .event_collection_members(collection_id)?
            .into_iter()
            .map(|member| member.event_id)
            .filter(|member_event_id| *member_event_id != event_id)
            .collect::<Vec<_>>();
        self.store
            .replace_event_collection_members(collection_id, &event_ids)?;
        self.last_message = Some("Removed event from collection.".to_string());
        Ok(())
    }

    fn move_event_in_collection(
        &mut self,
        event_id: Uuid,
        collection_id: Uuid,
        offset: isize,
    ) -> anyhow::Result<()> {
        let collection = self
            .store
            .event_collection_by_id(collection_id)?
            .ok_or_else(|| anyhow::anyhow!("event collection {collection_id} does not exist"))?;
        if !collection.ordered {
            anyhow::bail!(
                "collection {:?} is not an ordered sequence",
                collection.name
            );
        }

        let members = self.store.event_collection_members(collection_id)?;
        let Some(index) = members
            .iter()
            .position(|member| member.event_id == event_id)
        else {
            anyhow::bail!("event {event_id} is not in collection {collection_id}");
        };
        let target = index.saturating_add_signed(offset);
        if target >= members.len() || target == index {
            return Ok(());
        }

        let mut event_ids = members
            .into_iter()
            .map(|member| member.event_id)
            .collect::<Vec<_>>();
        event_ids.swap(index, target);
        self.store
            .replace_event_collection_members(collection_id, &event_ids)?;
        self.last_message = Some(format!(
            "Moved event within sequence {:?}.",
            collection.name
        ));
        Ok(())
    }

    fn create_collection_for_event(&mut self, event_id: Uuid) -> anyhow::Result<()> {
        let name = self.topology_new_collection_name.trim();
        if name.is_empty() {
            anyhow::bail!("collection name is empty");
        }
        let collection = EventCollection::new(name, self.topology_new_collection_ordered);
        self.store.upsert_event_collection(&collection)?;
        self.store
            .replace_event_collection_members(collection.id, &[event_id])?;
        self.topology_new_collection_name.clear();
        self.topology_new_collection_ordered = false;
        self.topology_collection_choice = Some(collection.id);
        self.last_message = Some(if collection.ordered {
            "Created sequence and added event.".to_string()
        } else {
            "Created collection and added event.".to_string()
        });
        Ok(())
    }

    fn begin_manage_collection(&mut self, collection_id: Uuid) -> anyhow::Result<()> {
        let collection = self
            .store
            .event_collection_by_id(collection_id)?
            .ok_or_else(|| anyhow::anyhow!("event collection {collection_id} does not exist"))?;
        self.topology_manage_collection_id = Some(collection.id);
        self.topology_manage_collection_name = collection.name;
        self.topology_manage_collection_description = collection.description.unwrap_or_default();
        self.topology_manage_collection_ordered = collection.ordered;
        Ok(())
    }

    fn save_managed_collection(&mut self, collection_id: Uuid) -> anyhow::Result<()> {
        let name = self.topology_manage_collection_name.trim();
        if name.is_empty() {
            anyhow::bail!("collection name is empty");
        }

        let mut collection = self
            .store
            .event_collection_by_id(collection_id)?
            .ok_or_else(|| anyhow::anyhow!("event collection {collection_id} does not exist"))?;
        let ordered_changed = collection.ordered != self.topology_manage_collection_ordered;
        let member_ids = if ordered_changed {
            self.store
                .events_for_collection(collection_id)?
                .into_iter()
                .map(|event| event.id)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        collection.name = name.to_string();
        collection.description = if self
            .topology_manage_collection_description
            .trim()
            .is_empty()
        {
            None
        } else {
            Some(
                self.topology_manage_collection_description
                    .trim()
                    .to_string(),
            )
        };
        collection.ordered = self.topology_manage_collection_ordered;
        collection.updated_at = Utc::now();
        self.store.upsert_event_collection(&collection)?;

        if ordered_changed {
            self.store
                .replace_event_collection_members(collection_id, &member_ids)?;
        }

        self.last_message = Some(if collection.ordered {
            format!("Updated sequence {:?}.", collection.name)
        } else {
            format!("Updated collection {:?}.", collection.name)
        });
        Ok(())
    }

    fn delete_managed_collection(&mut self, collection_id: Uuid) -> anyhow::Result<()> {
        let collection = self
            .store
            .event_collection_by_id(collection_id)?
            .ok_or_else(|| anyhow::anyhow!("event collection {collection_id} does not exist"))?;
        self.store.delete_event_collection(collection_id)?;
        self.topology_manage_collection_id = None;
        self.topology_manage_collection_name.clear();
        self.topology_manage_collection_description.clear();
        self.topology_manage_collection_ordered = false;
        self.topology_collection_choice = None;
        self.last_message = Some(format!(
            "Deleted {} {:?}; events were not deleted.",
            if collection.ordered {
                "sequence"
            } else {
                "collection"
            },
            collection.name
        ));
        Ok(())
    }

    fn create_relation_for_event(&mut self, event_id: Uuid) -> anyhow::Result<()> {
        let relation_type = self.topology_relation_type.trim();
        if relation_type.is_empty() {
            anyhow::bail!("relation type is empty");
        }
        let target_id = self
            .topology_relation_target_id
            .ok_or_else(|| anyhow::anyhow!("choose a relation target event"))?;
        let (from_event_id, to_event_id) = if self.topology_relation_outgoing {
            (event_id, target_id)
        } else {
            (target_id, event_id)
        };
        let relation = EventRelation::new(from_event_id, to_event_id, relation_type);
        self.store.upsert_event_relation(&relation)?;
        self.topology_relation_search.clear();
        self.topology_relation_candidates.clear();
        self.topology_relation_target_id = None;
        self.last_message = Some("Created event relation.".to_string());
        Ok(())
    }

    fn render_inspector(&mut self, ui: &mut egui::Ui, events: &[TemporalEvent]) {
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
            })
            .cloned();
        let Some(event) = event else {
            ui.label("The selected event is not in the current view.");
            return;
        };
        let canonical_id = self.canonical_event_id(event.id);
        if self
            .event_details_editor
            .as_ref()
            .is_some_and(|draft| draft.event_id != canonical_id)
        {
            self.event_details_editor = None;
        }
        if self
            .participant_entity_edit
            .is_some_and(|(event_id, _)| event_id != canonical_id)
        {
            self.reset_participant_entity_editor();
        }
        let participant_resolutions = event
            .participants
            .iter()
            .enumerate()
            .map(|(index, _)| {
                self.store
                    .resolve_event_participant_entity(canonical_id, index)
            })
            .collect::<Vec<_>>();
        if self.event_revision_event_id != Some(canonical_id) {
            match self.store.event_revisions(canonical_id) {
                Ok(revisions) => {
                    self.event_revision_event_id = Some(canonical_id);
                    self.event_revisions = revisions;
                }
                Err(error) => {
                    self.event_revision_event_id = Some(canonical_id);
                    self.event_revisions.clear();
                    self.last_error = Some(format!("Failed to load event history: {error:#}"));
                }
            }
        }
        let revision_rows = self.event_revisions.clone();

        if self.topology_edit_event_id != Some(canonical_id) {
            self.reset_topology_editor(canonical_id);
        }

        let collection_ids = self
            .event_memberships
            .get(&canonical_id)
            .map(|membership| membership.collection_ids.clone())
            .unwrap_or_default();
        let collection_rows = self
            .event_collections
            .iter()
            .filter(|collection| collection_ids.contains(&collection.id))
            .map(|collection| {
                let ordered_members = self
                    .event_collection_members
                    .iter()
                    .filter(|member| collection.ordered && member.collection_id == collection.id)
                    .collect::<Vec<_>>();
                let position = ordered_members
                    .iter()
                    .position(|member| member.event_id == canonical_id);
                (
                    collection.id,
                    collection.name.clone(),
                    collection.ordered,
                    position,
                    ordered_members.len(),
                )
            })
            .collect::<Vec<_>>();
        let available_collections = self
            .event_collections
            .iter()
            .filter(|collection| !collection_ids.contains(&collection.id))
            .map(|collection| (collection.id, collection.name.clone(), collection.ordered))
            .collect::<Vec<_>>();
        let relation_rows = self
            .event_relations
            .iter()
            .filter(|relation| {
                relation.from_event_id == canonical_id || relation.to_event_id == canonical_id
            })
            .cloned()
            .collect::<Vec<_>>();
        let identity_rows = self
            .event_identity_assessments
            .iter()
            .filter(|assessment| {
                assessment.left_event_id == canonical_id
                    || assessment.right_event_id == canonical_id
            })
            .cloned()
            .collect::<Vec<_>>();
        let annotation_rows = self
            .event_annotations
            .iter()
            .filter(|annotation| annotation.event_id == canonical_id)
            .cloned()
            .collect::<Vec<_>>();
        let provenance_rows = self
            .event_provenance_records
            .iter()
            .filter(|record| record.event_id == canonical_id)
            .cloned()
            .collect::<Vec<_>>();
        let mut topology_action = None;
        let mut participant_action = None;
        let mut event_details_action = None;
        let mut event_location_action = None;
        let mut refresh_participant_entity_search = false;
        let mut refresh_relation_search = false;
        let mut refresh_identity_search = false;

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading(&event.normalized_title);
                if self.event_is_editable(&event)
                    && self
                        .event_details_editor
                        .as_ref()
                        .is_none_or(|draft| draft.event_id != canonical_id)
                    && ui.small_button("Edit details").clicked()
                {
                    self.begin_event_details_edit(canonical_id);
                }
            });
            ui.label(
                RichText::new(event.status.as_str())
                    .color(status_color(event.status))
                    .strong(),
            );

            if self
                .event_details_editor
                .as_ref()
                .is_some_and(|draft| draft.event_id == canonical_id)
            {
                ui.group(|ui| {
                    ui.strong("Edit canonical details");
                    let draft = self.event_details_editor.as_mut().expect("checked above");
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.title)
                            .hint_text("Title"),
                    );
                    ui.add(
                        egui::TextEdit::multiline(&mut draft.description)
                            .desired_rows(3)
                            .hint_text("Description"),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.event_type)
                            .hint_text("Event type"),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.domain)
                            .hint_text("Domain"),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.jurisdiction)
                            .hint_text("Jurisdiction"),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.institution)
                            .hint_text("Institution"),
                    );

                    ui.horizontal_wrapped(|ui| {
                        egui::ComboBox::from_id_salt(("event-details-status", canonical_id))
                            .selected_text(draft.status.as_str())
                            .show_ui(ui, |ui| {
                                for status in EventStatus::ALL {
                                    ui.selectable_value(&mut draft.status, status, status.as_str());
                                }
                            });
                        egui::ComboBox::from_id_salt(("event-details-availability", canonical_id))
                            .selected_text(match draft.availability {
                                AvailabilityBehavior::Busy => "Busy",
                                AvailabilityBehavior::Free => "Free",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut draft.availability,
                                    AvailabilityBehavior::Busy,
                                    "Busy · blocks availability",
                                );
                                ui.selectable_value(
                                    &mut draft.availability,
                                    AvailabilityBehavior::Free,
                                    "Free · does not block availability",
                                );
                            });
                    });

                    ui.horizontal_wrapped(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut draft.confidence)
                                .desired_width(90.0)
                                .hint_text("Confidence 0..1"),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut draft.importance)
                                .desired_width(80.0)
                                .hint_text("Importance"),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut draft.personal_relevance)
                                .desired_width(100.0)
                                .hint_text("Relevance"),
                        );
                    });
                    ui.small(
                        "Blank optional fields clear the canonical value. Time, recurrence, participants, topology, and provenance are edited in their dedicated sections.",
                    );
                    if let Some(confirmation) = &draft.conflict_confirmation {
                        ui.colored_label(Color32::YELLOW, &confirmation.warning);
                    }

                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                !draft.title.trim().is_empty(),
                                egui::Button::new("Save details"),
                            )
                            .clicked()
                        {
                            event_details_action = Some(EventDetailsEditorAction::Save);
                        }
                        if ui.small_button("Cancel").clicked() {
                            event_details_action = Some(EventDetailsEditorAction::Cancel);
                        }
                    });
                });
            }

            ui.add_space(8.0);

            ui.horizontal_wrapped(|ui| {
                inspector_row(ui, "Time kind", event.time.kind_name());
                let time_edit_supported = self.event_is_editable(&event)
                    && event.recurrence.is_none()
                    && matches!(
                        event.time,
                        TimeSpec::Instant { .. }
                            | TimeSpec::Floating { .. }
                            | TimeSpec::AllDay { .. }
                            | TimeSpec::DateOnly { .. }
                    )
                    && (event.time_uncertainty.is_none()
                        || EventTimeEditDraft::from_event(&event, self.timezone()).is_ok());
                if ui
                    .add_enabled(
                        time_edit_supported,
                        egui::Button::new(if event.time_uncertainty.is_some() {
                            "Move uncertain placement"
                        } else {
                            "Edit time"
                        }),
                    )
                    .clicked()
                {
                    self.begin_event_time_edit(canonical_id);
                }
            });
            inspector_row(ui, "Display", &event.display_time_label(self.timezone()));

            if self
                .event_time_editor
                .as_ref()
                .is_some_and(|draft| draft.event_id == canonical_id)
            {
                let display_timezone = self.timezone();
                let mut selected_alternative = None;
                ui.group(|ui| {
                    ui.strong("Edit canonical time");
                    let draft = self.event_time_editor.as_mut().expect("checked above");
                    let uncertain = draft.uncertain_origin.is_some();
                    if uncertain {
                        ui.small(
                            "Moving the representative start also moves its entire bounded possible-start window. Duration, precision, and source clock stay unchanged. Save twice to acknowledge provisional availability.",
                        );
                    }
                    ui.small(format!("Kind: {}", draft.kind.label()));
                    if let Some(timezone) = draft.timezone_label() {
                        ui.small(format!("Clock context: {timezone}"));
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut draft.date)
                                .hint_text("YYYY-MM-DD")
                                .desired_width(110.0),
                        );
                        if draft.is_timed() {
                            ui.add(
                                egui::TextEdit::singleline(&mut draft.start_time)
                                    .hint_text("HH:MM")
                                    .desired_width(75.0),
                            );
                            ui.add_enabled(
                                !uncertain,
                                egui::TextEdit::singleline(&mut draft.duration_minutes)
                                    .hint_text("minutes")
                                    .desired_width(90.0),
                            );
                            ui.small(if uncertain {
                                "fixed duration (minutes)"
                            } else {
                                "duration min · blank keeps no explicit end"
                            });
                        } else if uncertain {
                            let derived_end = draft.parsed_time().ok().and_then(|time| match time {
                                TimeSpec::AllDay { end_exclusive, .. }
                                | TimeSpec::DateOnly { end_exclusive, .. } => end_exclusive,
                                _ => None,
                            });
                            if let Some(end) = derived_end {
                                ui.small(format!("derived exclusive end: {end}"));
                            } else {
                                ui.small("implicit one-day span; exclusive end remains absent");
                            }
                        } else {
                            ui.add(
                                egui::TextEdit::singleline(&mut draft.end_date)
                                    .hint_text("end exclusive YYYY-MM-DD")
                                    .desired_width(185.0),
                            );
                            ui.small("optional end-exclusive date");
                        }
                    });

                    if let Some(confirmation) = &draft.conflict_confirmation {
                        ui.colored_label(Color32::YELLOW, &confirmation.warning);
                        let timezone = match &draft.kind {
                            EventTimeEditKind::Instant { edit_timezone, .. } => *edit_timezone,
                            EventTimeEditKind::Floating {
                                source_timezone: Some(raw),
                            } => raw.parse::<Tz>().unwrap_or(display_timezone),
                            _ => display_timezone,
                        };
                        selected_alternative =
                            render_conflict_alternatives(ui, confirmation, timezone);
                    }

                    let mut action = None;
                    ui.horizontal(|ui| {
                        if ui.button("Save time").clicked() {
                            action = Some(EventTimeEditorAction::Save);
                        }
                        if ui.small_button("Cancel").clicked() {
                            action = Some(EventTimeEditorAction::Cancel);
                        }
                    });
                    match action {
                        Some(EventTimeEditorAction::Save) => self.save_event_time_edit(),
                        Some(EventTimeEditorAction::Cancel) => {
                            self.event_time_editor = None;
                            self.last_error = None;
                        }
                        None => {}
                    }
                });
                if let Some(slot) = selected_alternative {
                    let timezone = self.timezone();
                    if let Some(draft) = self.event_time_editor.as_mut() {
                        match apply_alternative_to_time_draft(draft, &slot, timezone) {
                            Ok(()) => self.last_error = None,
                            Err(error) => {
                                self.last_error =
                                    Some(format!("Cannot use alternate slot: {error:#}"));
                            }
                        }
                    }
                }
            }
            inspector_row(
                ui,
                "Availability",
                match event.availability {
                    AvailabilityBehavior::Busy => "Busy · blocks time",
                    AvailabilityBehavior::Free => "Free · does not block time",
                },
            );
            if let Some(uncertainty) = event.time_uncertainty.as_ref() {
                let (earliest, latest) =
                    temporal_uncertainty_labels(uncertainty, self.timezone());
                ui.separator();
                ui.strong("Temporal uncertainty");
                ui.small(
                    "Display is the representative placement; the actual start is bounded by this window.",
                );
                inspector_row(ui, "Window kind", uncertainty.kind_name());
                inspector_row(ui, "Earliest", &earliest);
                inspector_row(ui, "Latest", &latest);
            }
            if let Some(raw) = event.raw_title.as_deref() {
                inspector_row(ui, "Raw title", raw);
            }

            self.render_event_notification_rules(
                ui,
                canonical_id,
                &event.normalized_title,
                &event.time,
            );

            if let Some(rule) = event.recurrence.as_ref() {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.strong("Recurrence");
                    if self.event_is_editable(&event) {
                        if ui.small_button("Edit series").clicked() {
                            self.begin_recurrence_edit(canonical_id);
                        }
                        if self.occurrence_contexts.contains_key(&event.id)
                            && ui.small_button("Edit this occurrence").clicked()
                        {
                            self.begin_occurrence_override_edit(event.id);
                        }
                    }
                });
                inspector_row(ui, "Frequency", rule.frequency.as_str());
                inspector_row(ui, "Interval", &rule.interval.to_string());
                inspector_row(
                    ui,
                    "Count",
                    &rule
                        .count
                        .map_or_else(|| "unbounded".to_string(), |count| count.to_string()),
                );
                inspector_row(
                    ui,
                    "Until",
                    &rule
                        .until
                        .map_or_else(|| "unbounded".to_string(), |until| until.to_string()),
                );
                if !rule.by_weekday.is_empty() {
                    let mut weekdays = rule.by_weekday.clone();
                    weekdays.sort_by_key(|weekday| weekday.offset_from(rule.week_start));
                    let weekdays = weekdays
                        .into_iter()
                        .map(|weekday| weekday.short_label())
                        .collect::<Vec<_>>()
                        .join(", ");
                    inspector_row(ui, "Weekdays", &weekdays);
                    if rule.frequency == RecurrenceFrequency::Weekly || !rule.by_week_no.is_empty()
                    {
                        inspector_row(ui, "Week start", rule.week_start.short_label());
                    }
                }
                if !rule.by_month.is_empty() {
                    let mut months = rule.by_month.clone();
                    months.sort_unstable();
                    let months = months
                        .into_iter()
                        .map(|month| month.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    inspector_row(ui, "Months", &months);
                }
                if !rule.by_week_no.is_empty() {
                    let mut week_numbers = rule.by_week_no.clone();
                    week_numbers.sort_unstable();
                    let week_numbers = week_numbers
                        .into_iter()
                        .map(|week_no| week_no.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    inspector_row(ui, "Week numbers", &week_numbers);
                }
                if !rule.by_year_day.is_empty() {
                    let mut year_days = rule.by_year_day.clone();
                    year_days.sort_unstable();
                    let year_days = year_days
                        .into_iter()
                        .map(|day| day.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    inspector_row(ui, "Year days", &year_days);
                }
                if !rule.by_month_day.is_empty() {
                    let mut month_days = rule.by_month_day.clone();
                    month_days.sort_unstable();
                    let month_days = month_days
                        .into_iter()
                        .map(|day| day.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    inspector_row(ui, "Month days", &month_days);
                }
                if !rule.by_month_weekday.is_empty() {
                    let mut selectors = rule.by_month_weekday.clone();
                    selectors.sort_by_key(|selector| {
                        (selector.ordinal, selector.weekday.offset_from_monday())
                    });
                    let selectors = selectors
                        .into_iter()
                        .map(|selector| {
                            format!("{} {}", selector.ordinal, selector.weekday.short_label())
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let label = if rule.frequency == RecurrenceFrequency::Yearly
                        && rule.by_month.is_empty()
                    {
                        "Year weekdays"
                    } else {
                        "Month weekdays"
                    };
                    inspector_row(ui, label, &selectors);
                }
                if !rule.by_hour.is_empty() {
                    let mut hours = rule.by_hour.clone();
                    hours.sort_unstable();
                    let hours = hours
                        .into_iter()
                        .map(|hour| format!("{hour:02}:00"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    inspector_row(ui, "Hours", &hours);
                }
                if !rule.by_minute.is_empty() {
                    let mut minutes = rule.by_minute.clone();
                    minutes.sort_unstable();
                    let minutes = minutes
                        .into_iter()
                        .map(|minute| format!("{minute:02}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    inspector_row(ui, "Minutes", &minutes);
                }
                if !rule.by_second.is_empty() {
                    let mut seconds = rule.by_second.clone();
                    seconds.sort_unstable();
                    let seconds = seconds
                        .into_iter()
                        .map(|second| format!("{second:02}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    inspector_row(ui, "Seconds", &seconds);
                }
                if !rule.by_set_pos.is_empty() {
                    let mut positions = rule.by_set_pos.clone();
                    positions.sort_unstable();
                    let positions = positions
                        .into_iter()
                        .map(|position| position.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    inspector_row(ui, "Set positions", &positions);
                }
                inspector_row(ui, "RDATE additions", &rule.rdates.len().to_string());
                inspector_row(ui, "EXDATE exclusions", &rule.exdates.len().to_string());
                inspector_row(
                    ui,
                    "Occurrence overrides",
                    &rule.overrides.len().to_string(),
                );
            } else if self.event_is_editable(&event)
                && !matches!(
                    event.time,
                    TimeSpec::Month { .. } | TimeSpec::Year { .. } | TimeSpec::Unknown { .. }
                )
            {
                ui.separator();
                if ui.button("Add recurrence").clicked() {
                    self.begin_recurrence_edit(event.id);
                }
            }

            if self
                .recurrence_editor
                .as_ref()
                .is_some_and(|draft| draft.event_id == self.canonical_event_id(event.id))
            {
                ui.separator();
                let display_timezone = self.timezone();
                let action = {
                    let draft = self.recurrence_editor.as_mut().expect("checked above");
                    render_recurrence_editor(ui, draft, display_timezone)
                };
                match action {
                    Some(RecurrenceEditorAction::Save) => self.save_recurrence_edit(false),
                    Some(RecurrenceEditorAction::Remove) => self.save_recurrence_edit(true),
                    Some(RecurrenceEditorAction::FindAlternatives) => {
                        self.find_recurring_occurrence_alternatives();
                    }
                    Some(RecurrenceEditorAction::UseAlternative(slot)) => {
                        self.use_recurring_occurrence_alternative(&slot);
                    }
                    Some(RecurrenceEditorAction::Cancel) => {
                        self.recurrence_editor = None;
                        self.recurrence_alternative_receiver = None;
                        self.last_error = None;
                    }
                    None => {}
                }
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
            if event.location.is_some() || self.event_is_editable(&event) {
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    ui.strong("Location");
                    if self.event_is_editable(&event)
                        && self
                            .event_location_editor
                            .as_ref()
                            .is_none_or(|draft| draft.event_id != canonical_id)
                        && ui.small_button("Edit location").clicked()
                    {
                        self.begin_event_location_edit(canonical_id);
                    }
                });

                if let Some(location) = event.location.as_ref() {
                    if let Some(value) = location.name.as_deref() {
                        inspector_row(ui, "Venue", value);
                    }
                    if let Some(value) = location.address.as_deref() {
                        inspector_row(ui, "Address", value);
                    }
                    if let Some(value) = location.locality.as_deref() {
                        inspector_row(ui, "Locality", value);
                    }
                    if let Some(value) = location.region.as_deref() {
                        inspector_row(ui, "Region", value);
                    }
                    if let Some(value) = location.postal_code.as_deref() {
                        inspector_row(ui, "Postal code", value);
                    }
                    if let Some(value) = location.country.as_deref() {
                        inspector_row(ui, "Country", value);
                    }
                    if let (Some(latitude), Some(longitude)) =
                        (location.latitude, location.longitude)
                    {
                        inspector_row(
                            ui,
                            "Coordinates",
                            &format!("{latitude:.6}, {longitude:.6}"),
                        );
                    }
                    if let Some(value) = location.virtual_url.as_deref() {
                        inspector_row(ui, "Virtual URL", value);
                    }
                } else {
                    ui.small("No structured location.");
                }

                if self
                    .event_location_editor
                    .as_ref()
                    .is_some_and(|draft| draft.event_id == canonical_id)
                {
                    ui.group(|ui| {
                        ui.small(
                            "Blank every field to remove the location. Latitude and longitude must be supplied together.",
                        );
                        let draft = self.event_location_editor.as_mut().expect("checked above");
                        ui.add(
                            egui::TextEdit::singleline(&mut draft.name)
                                .hint_text("Venue / location name"),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut draft.address)
                                .hint_text("Street address"),
                        );
                        ui.horizontal_wrapped(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut draft.locality)
                                    .hint_text("Locality")
                                    .desired_width(120.0),
                            );
                            ui.add(
                                egui::TextEdit::singleline(&mut draft.region)
                                    .hint_text("Region")
                                    .desired_width(100.0),
                            );
                            ui.add(
                                egui::TextEdit::singleline(&mut draft.postal_code)
                                    .hint_text("Postal")
                                    .desired_width(80.0),
                            );
                            ui.add(
                                egui::TextEdit::singleline(&mut draft.country)
                                    .hint_text("Country")
                                    .desired_width(100.0),
                            );
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut draft.latitude)
                                    .hint_text("Latitude")
                                    .desired_width(100.0),
                            );
                            ui.add(
                                egui::TextEdit::singleline(&mut draft.longitude)
                                    .hint_text("Longitude")
                                    .desired_width(100.0),
                            );
                        });
                        ui.add(
                            egui::TextEdit::singleline(&mut draft.virtual_url)
                                .hint_text("Virtual meeting URL"),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Save location").clicked() {
                                event_location_action = Some(EventLocationEditorAction::Save);
                            }
                            if ui.small_button("Cancel").clicked() {
                                event_location_action = Some(EventLocationEditorAction::Cancel);
                            }
                        });
                    });
                }
            }
            if !event.participants.is_empty() || self.event_is_editable(&event) {
                ui.separator();
                ui.strong("Participants");
                ui.small(
                    "Participant labels remain source-owned; canonical entity resolution is a separate local layer.",
                );
                for (index, participant) in event.participants.iter().enumerate() {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            let label = participant.role.as_deref().unwrap_or("Participant");
                            ui.label(RichText::new(&participant.name).strong());
                            ui.small(format!("· {label}"));
                            if self.event_is_editable(&event)
                                && ui.small_button("×").on_hover_text("Remove participant").clicked()
                            {
                                participant_action =
                                    Some(ParticipantInspectorAction::Remove(index));
                            }
                        });
                        if let Some(participant_type) = participant.participant_type.as_deref() {
                            inspector_row(ui, "Type", participant_type);
                        }
                        if let Some(entity_ref) = participant.entity_ref.as_deref() {
                            inspector_row(ui, "Source entity ref", entity_ref);
                        }

                        let resolution = participant_resolutions.get(index);
                        match resolution {
                            Some(Ok(ParticipantEntityResolution::Manual { entity, .. })) => {
                                inspector_row(
                                    ui,
                                    "Canonical entity",
                                    &format!("{} · manual binding", entity.canonical_name),
                                );
                            }
                            Some(Ok(ParticipantEntityResolution::SourceReference(entity))) => {
                                inspector_row(
                                    ui,
                                    "Canonical entity",
                                    &format!("{} · source ref", entity.canonical_name),
                                );
                            }
                            Some(Ok(ParticipantEntityResolution::ExactLabel(entity))) => {
                                inspector_row(
                                    ui,
                                    "Canonical entity",
                                    &format!("{} · exact label", entity.canonical_name),
                                );
                            }
                            Some(Ok(ParticipantEntityResolution::Ambiguous(candidates))) => {
                                inspector_row(
                                    ui,
                                    "Canonical entity",
                                    &format!("ambiguous · {} matches", candidates.len()),
                                );
                            }
                            Some(Ok(ParticipantEntityResolution::UnresolvedReference(reference))) => {
                                inspector_row(
                                    ui,
                                    "Canonical entity",
                                    &format!("unresolved source ref · {reference}"),
                                );
                            }
                            Some(Ok(ParticipantEntityResolution::Unresolved)) => {
                                inspector_row(ui, "Canonical entity", "unresolved");
                            }
                            Some(Err(error)) => {
                                inspector_row(
                                    ui,
                                    "Canonical entity",
                                    &format!("resolution error · {error}"),
                                );
                            }
                            None => {}
                        }

                        ui.horizontal(|ui| {
                            if ui.small_button("Resolve…").clicked() {
                                self.begin_participant_entity_resolution(
                                    canonical_id,
                                    index,
                                    participant,
                                );
                            }
                            if matches!(
                                resolution,
                                Some(Ok(ParticipantEntityResolution::Manual { .. }))
                            ) && ui
                                .small_button("Unbind")
                                .on_hover_text("Remove only the Ephemeris-local binding")
                                .clicked()
                            {
                                participant_action =
                                    Some(ParticipantInspectorAction::UnbindEntity(index));
                            }
                        });

                        if self.participant_entity_edit == Some((canonical_id, index)) {
                            ui.separator();
                            ui.small("Bind this participant to a durable canonical entity.");
                            if ui
                                .add(
                                    egui::TextEdit::singleline(
                                        &mut self.participant_entity_search,
                                    )
                                    .hint_text("Search canonical entity"),
                                )
                                .changed()
                            {
                                refresh_participant_entity_search = true;
                            }

                            let candidates = self.participant_entity_candidates.clone();
                            if !candidates.is_empty() {
                                let selected = self
                                    .participant_entity_target_id
                                    .and_then(|target_id| {
                                        candidates
                                            .iter()
                                            .find(|(id, _, _)| *id == target_id)
                                            .map(|(_, name, entity_type)| {
                                                entity_type.as_deref().map_or_else(
                                                    || name.clone(),
                                                    |kind| format!("{name} · {kind}"),
                                                )
                                            })
                                    })
                                    .unwrap_or_else(|| "Select canonical entity…".to_string());
                                egui::ComboBox::from_id_salt((
                                    "participant-entity-target",
                                    canonical_id,
                                    index,
                                ))
                                .selected_text(selected)
                                .show_ui(ui, |ui| {
                                    for (entity_id, name, entity_type) in &candidates {
                                        let label = entity_type.as_deref().map_or_else(
                                            || name.clone(),
                                            |kind| format!("{name} · {kind}"),
                                        );
                                        ui.selectable_value(
                                            &mut self.participant_entity_target_id,
                                            Some(*entity_id),
                                            label,
                                        );
                                    }
                                });
                            }

                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(
                                        self.participant_entity_target_id.is_some(),
                                        egui::Button::new("Bind selected"),
                                    )
                                    .clicked()
                                    && let Some(entity_id) = self.participant_entity_target_id
                                {
                                    participant_action = Some(
                                        ParticipantInspectorAction::BindEntity(index, entity_id),
                                    );
                                }
                                if ui.button("Create entity from participant").clicked() {
                                    participant_action =
                                        Some(ParticipantInspectorAction::CreateEntity(index));
                                }
                                if ui.small_button("Cancel").clicked() {
                                    self.reset_participant_entity_editor();
                                }
                            });
                        }
                    });
                }

                if self.event_is_editable(&event) {
                    ui.collapsing("Add participant", |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.participant_new_name)
                                .hint_text("Name"),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut self.participant_new_role)
                                .hint_text("Role (optional)"),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut self.participant_new_type)
                                .hint_text("Type: person, organization, team…"),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut self.participant_new_entity_ref)
                                .hint_text("Stable source entity ref (optional)"),
                        );
                        if ui
                            .add_enabled(
                                !self.participant_new_name.trim().is_empty(),
                                egui::Button::new("Add participant"),
                            )
                            .clicked()
                        {
                            participant_action = Some(ParticipantInspectorAction::Add);
                        }
                    });
                }
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

            ui.separator();
            ui.strong("Topology");

            for (collection_id, collection_name, ordered, position, member_count) in
                &collection_rows
            {
                ui.horizontal(|ui| {
                    ui.label(if *ordered { "Sequence" } else { "Collection" });
                    ui.label(collection_name);
                    if *ordered && let Some(position) = *position {
                        ui.label(format!("#{}", position + 1));
                        if ui
                            .add_enabled(position > 0, egui::Button::new("↑").small())
                            .on_hover_text("Move earlier in sequence")
                            .clicked()
                        {
                            topology_action = Some(
                                TopologyInspectorAction::MoveCollectionMemberUp(*collection_id),
                            );
                        }
                        if ui
                            .add_enabled(
                                position + 1 < *member_count,
                                egui::Button::new("↓").small(),
                            )
                            .on_hover_text("Move later in sequence")
                            .clicked()
                        {
                            topology_action = Some(
                                TopologyInspectorAction::MoveCollectionMemberDown(*collection_id),
                            );
                        }
                    }
                    if ui
                        .small_button("Remove")
                        .on_hover_text("Remove this event from the collection")
                        .clicked()
                    {
                        topology_action = Some(TopologyInspectorAction::RemoveFromCollection(
                            *collection_id,
                        ));
                    }
                    if ui
                        .small_button("Manage")
                        .on_hover_text("Rename, describe, convert, or delete this collection")
                        .clicked()
                        && let Err(error) = self.begin_manage_collection(*collection_id)
                    {
                        self.last_error =
                            Some(format!("Failed to open collection manager: {error:#}"));
                    }
                });
            }

            if !available_collections.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    let selected = self
                        .topology_collection_choice
                        .and_then(|selected_id| {
                            available_collections
                                .iter()
                                .find(|(id, _, _)| *id == selected_id)
                                .map(|(_, name, ordered)| {
                                    if *ordered {
                                        format!("{name} · sequence")
                                    } else {
                                        name.clone()
                                    }
                                })
                        })
                        .unwrap_or_else(|| "Add to collection…".to_string());
                    egui::ComboBox::from_id_salt(("topology-add-collection", canonical_id))
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            for (collection_id, name, ordered) in &available_collections {
                                let label = if *ordered {
                                    format!("{name} · sequence")
                                } else {
                                    name.clone()
                                };
                                ui.selectable_value(
                                    &mut self.topology_collection_choice,
                                    Some(*collection_id),
                                    label,
                                );
                            }
                        });
                    if ui
                        .add_enabled(
                            self.topology_collection_choice.is_some(),
                            egui::Button::new("Add"),
                        )
                        .clicked()
                        && let Some(collection_id) = self.topology_collection_choice
                    {
                        topology_action =
                            Some(TopologyInspectorAction::AddToCollection(collection_id));
                    }
                });
            }

            ui.collapsing("New collection / sequence", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.topology_new_collection_name)
                        .hint_text("Collection name"),
                );
                ui.checkbox(
                    &mut self.topology_new_collection_ordered,
                    "Ordered sequence",
                );
                if ui
                    .add_enabled(
                        !self.topology_new_collection_name.trim().is_empty(),
                        egui::Button::new("Create + add"),
                    )
                    .clicked()
                {
                    topology_action = Some(TopologyInspectorAction::CreateCollection);
                }
            });

            if let Some(collection_id) = self.topology_manage_collection_id {
                ui.collapsing("Manage collection / sequence", |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.topology_manage_collection_name)
                            .hint_text("Collection name"),
                    );
                    ui.add(
                        egui::TextEdit::multiline(
                            &mut self.topology_manage_collection_description,
                        )
                        .desired_rows(2)
                        .hint_text("Optional description"),
                    );
                    ui.checkbox(
                        &mut self.topology_manage_collection_ordered,
                        "Ordered sequence",
                    );
                    ui.small(
                        "Switching to a sequence assigns deterministic positions to all current members. Switching to a collection removes ordering but keeps every member.",
                    );
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(
                                !self.topology_manage_collection_name.trim().is_empty(),
                                egui::Button::new("Save collection"),
                            )
                            .clicked()
                        {
                            topology_action =
                                Some(TopologyInspectorAction::SaveCollection(collection_id));
                        }
                        if ui.button("Close").clicked() {
                            self.topology_manage_collection_id = None;
                        }
                        if ui
                            .button("Delete collection")
                            .on_hover_text(
                                "Delete the collection and its memberships; events remain intact",
                            )
                            .clicked()
                        {
                            topology_action =
                                Some(TopologyInspectorAction::DeleteCollection(collection_id));
                        }
                    });
                });
            }

            for relation in &relation_rows {
                let (direction, counterpart_id) = if relation.from_event_id == canonical_id {
                    ("Outgoing", relation.to_event_id)
                } else {
                    ("Incoming", relation.from_event_id)
                };
                let counterpart = self
                    .event_relation_titles
                    .get(&counterpart_id)
                    .map_or_else(|| counterpart_id.to_string(), Clone::clone);
                ui.horizontal(|ui| {
                    ui.label(direction);
                    ui.label(format!("{} · {}", relation.relation_type, counterpart));
                    if ui
                        .small_button("×")
                        .on_hover_text("Delete relation")
                        .clicked()
                    {
                        topology_action =
                            Some(TopologyInspectorAction::DeleteRelation(relation.id));
                    }
                });
            }

            ui.collapsing("New relation", |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(&mut self.topology_relation_outgoing, true, "Outgoing");
                    ui.selectable_value(&mut self.topology_relation_outgoing, false, "Incoming");
                });

                egui::ComboBox::from_id_salt(("topology-relation-type", canonical_id))
                    .selected_text(if self.topology_relation_type.trim().is_empty() {
                        "Relation type…"
                    } else {
                        self.topology_relation_type.as_str()
                    })
                    .show_ui(ui, |ui| {
                        for relation_type in &self.event_relation_types {
                            ui.selectable_value(
                                &mut self.topology_relation_type,
                                relation_type.clone(),
                                relation_type,
                            );
                        }
                    });
                ui.add(
                    egui::TextEdit::singleline(&mut self.topology_relation_type)
                        .hint_text("causes, precedes, references, ..."),
                );

                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.topology_relation_search)
                            .hint_text("Search target event title/ref"),
                    )
                    .changed()
                {
                    refresh_relation_search = true;
                }

                if !self.topology_relation_candidates.is_empty() {
                    let selected = self
                        .topology_relation_target_id
                        .and_then(|target_id| {
                            self.topology_relation_candidates
                                .iter()
                                .find(|(id, _)| *id == target_id)
                                .map(|(id, title)| format!("{title} · {id}"))
                        })
                        .unwrap_or_else(|| "Select target event…".to_string());
                    egui::ComboBox::from_id_salt(("topology-relation-target", canonical_id))
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            for (target_id, title) in &self.topology_relation_candidates {
                                ui.selectable_value(
                                    &mut self.topology_relation_target_id,
                                    Some(*target_id),
                                    format!("{title} · {target_id}"),
                                );
                            }
                        });
                }

                if ui
                    .add_enabled(
                        !self.topology_relation_type.trim().is_empty()
                            && self.topology_relation_target_id.is_some(),
                        egui::Button::new("Create relation"),
                    )
                    .clicked()
                {
                    topology_action = Some(TopologyInspectorAction::AddRelation);
                }
            });

            ui.separator();
            ui.strong("Duplicate / entity resolution");
            ui.small(
                "Identity assessments are external evidence about canonical events; changing state does not merge or delete either event.",
            );

            for assessment in &identity_rows {
                let counterpart_id = if assessment.left_event_id == canonical_id {
                    assessment.right_event_id
                } else {
                    assessment.left_event_id
                };
                let counterpart = self
                    .event_identity_titles
                    .get(&counterpart_id)
                    .map_or_else(|| counterpart_id.to_string(), Clone::clone);
                ui.group(|ui| {
                    ui.label(RichText::new(&counterpart).strong());
                    ui.small(counterpart_id.to_string());

                    let mut state = assessment.state;
                    egui::ComboBox::from_id_salt(("identity-state", assessment.id))
                        .selected_text(state.as_str())
                        .show_ui(ui, |ui| {
                            for candidate in EventIdentityState::ALL {
                                ui.selectable_value(&mut state, candidate, candidate.as_str());
                            }
                        });
                    if state != assessment.state {
                        topology_action = Some(TopologyInspectorAction::SetIdentityState(
                            assessment.id,
                            state,
                        ));
                    }

                    if let Some(confidence) = assessment.confidence {
                        inspector_row(ui, "Identity confidence", &format!("{confidence:.3}"));
                    }
                    if let Some(rationale) = assessment.rationale.as_deref()
                        && !rationale.trim().is_empty()
                    {
                        ui.small(rationale);
                    }
                    if ui
                        .small_button("Delete assessment")
                        .on_hover_text(
                            "Delete only this identity assessment; neither event is deleted",
                        )
                        .clicked()
                    {
                        topology_action = Some(
                            TopologyInspectorAction::DeleteIdentityAssessment(assessment.id),
                        );
                    }
                });
            }

            ui.collapsing("New identity assessment", |ui| {
                ui.small(
                    "Search another canonical event, then record whether the pair is a candidate duplicate, the same event, or definitely distinct.",
                );
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.identity_search)
                            .hint_text("Search event title/ref"),
                    )
                    .changed()
                {
                    refresh_identity_search = true;
                }

                if !self.identity_candidates.is_empty() {
                    let selected = self
                        .identity_target_id
                        .and_then(|target_id| {
                            self.identity_candidates
                                .iter()
                                .find(|(id, _)| *id == target_id)
                                .map(|(id, title)| format!("{title} · {id}"))
                        })
                        .unwrap_or_else(|| "Select comparison event…".to_string());
                    egui::ComboBox::from_id_salt(("identity-target", canonical_id))
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            for (target_id, title) in &self.identity_candidates {
                                ui.selectable_value(
                                    &mut self.identity_target_id,
                                    Some(*target_id),
                                    format!("{title} · {target_id}"),
                                );
                            }
                        });
                }

                egui::ComboBox::from_id_salt(("identity-new-state", canonical_id))
                    .selected_text(self.identity_new_state.as_str())
                    .show_ui(ui, |ui| {
                        for candidate in EventIdentityState::ALL {
                            ui.selectable_value(
                                &mut self.identity_new_state,
                                candidate,
                                candidate.as_str(),
                            );
                        }
                    });

                ui.add(
                    egui::TextEdit::singleline(&mut self.identity_confidence_text)
                        .hint_text("Optional confidence 0..1"),
                );
                ui.add(
                    egui::TextEdit::multiline(&mut self.identity_rationale)
                        .desired_rows(2)
                        .hint_text("Optional rationale"),
                );

                if ui
                    .add_enabled(
                        self.identity_target_id.is_some(),
                        egui::Button::new("Create assessment"),
                    )
                    .clicked()
                {
                    topology_action = Some(TopologyInspectorAction::AddIdentityAssessment);
                }
            });

            ui.separator();
            ui.strong("Structured provenance");
            ui.small(
                "Typed assertion/source/provenance references stored separately from source-backed event fields and legacy raw ref arrays.",
            );
            if provenance_rows.is_empty() {
                ui.small("No structured provenance records.");
            }
            for record in &provenance_rows {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(record.role.as_str()).strong());
                        if ui
                            .small_button("Delete")
                            .on_hover_text(
                                "Delete this provenance record only; the event and linked source remain intact",
                            )
                            .clicked()
                        {
                            topology_action =
                                Some(TopologyInspectorAction::DeleteProvenance(record.id));
                        }
                    });
                    ui.monospace(&record.reference);
                    if let Some(source_id) = record.source_id {
                        let source_label = self
                            .sources
                            .iter()
                            .find(|source| source.id == source_id)
                            .map_or_else(|| source_id.to_string(), |source| source.name.clone());
                        inspector_row(ui, "Source", &source_label);
                    }
                    if let Some(note) = record.note.as_deref() {
                        ui.label(note);
                    }
                    if record
                        .properties
                        .as_object()
                        .is_some_and(|properties| !properties.is_empty())
                    {
                        ui.monospace(
                            serde_json::to_string_pretty(&record.properties)
                                .unwrap_or_else(|_| "<invalid properties>".to_string()),
                        );
                    }
                });
            }

            ui.collapsing("New provenance record", |ui| {
                egui::ComboBox::from_id_salt(("provenance-role", canonical_id))
                    .selected_text(self.provenance_new_role.as_str())
                    .show_ui(ui, |ui| {
                        for role in EventProvenanceRole::ALL {
                            ui.selectable_value(
                                &mut self.provenance_new_role,
                                role,
                                role.as_str(),
                            );
                        }
                    });
                ui.add(
                    egui::TextEdit::singleline(&mut self.provenance_new_reference)
                        .hint_text("Reference / URI / assertion identifier"),
                );
                ui.add(
                    egui::TextEdit::multiline(&mut self.provenance_new_note)
                        .desired_rows(2)
                        .hint_text("Optional note"),
                );
                if ui
                    .add_enabled(
                        !self.provenance_new_reference.trim().is_empty(),
                        egui::Button::new("Create provenance record"),
                    )
                    .clicked()
                {
                    topology_action = Some(TopologyInspectorAction::AddProvenance);
                }
            });

            ui.separator();
            ui.strong("Annotations");
            ui.small(
                "User-owned metadata stored separately from source-backed event fields; source refresh does not overwrite it.",
            );
            for annotation in &annotation_rows {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&annotation.kind).strong());
                        if ui
                            .small_button("Delete")
                            .on_hover_text(
                                "Delete this annotation only; the event and source record remain intact",
                            )
                            .clicked()
                        {
                            topology_action =
                                Some(TopologyInspectorAction::DeleteAnnotation(annotation.id));
                        }
                    });
                    let value = serde_json::to_string_pretty(&annotation.value)
                        .unwrap_or_else(|_| "<invalid JSON value>".to_string());
                    ui.monospace(value);
                });
            }

            ui.collapsing("New annotation", |ui| {
                ui.small(
                    "Values are explicit JSON: quote text strings, or use true/false, numbers, arrays, or objects.",
                );
                egui::ComboBox::from_id_salt(("annotation-kind", canonical_id))
                    .selected_text(if self.annotation_new_kind.trim().is_empty() {
                        "Annotation kind…"
                    } else {
                        self.annotation_new_kind.as_str()
                    })
                    .show_ui(ui, |ui| {
                        for kind in &self.event_annotation_kinds {
                            ui.selectable_value(
                                &mut self.annotation_new_kind,
                                kind.clone(),
                                kind,
                            );
                        }
                    });
                ui.add(
                    egui::TextEdit::singleline(&mut self.annotation_new_kind)
                        .hint_text("note, watched, rating, custom_tag, ..."),
                );
                ui.add(
                    egui::TextEdit::multiline(&mut self.annotation_new_value)
                        .desired_rows(3)
                        .hint_text("\"Review this source discrepancy\""),
                );
                if ui
                    .add_enabled(
                        !self.annotation_new_kind.trim().is_empty()
                            && !self.annotation_new_value.trim().is_empty(),
                        egui::Button::new("Create annotation"),
                    )
                    .clicked()
                {
                    topology_action = Some(TopologyInspectorAction::AddAnnotation);
                }
            });

            ui.separator();
            ui.collapsing(
                format!("Canonical history · {} revision(s)", revision_rows.len()),
                |ui| {
                    ui.small(
                        "Immutable snapshots recorded after real canonical event changes since schema v20. Existing events are not backfilled retroactively.",
                    );
                    if revision_rows.is_empty() {
                        ui.small("No recorded revisions yet.");
                    }
                    let hidden = revision_rows.len().saturating_sub(25);
                    if hidden > 0 {
                        ui.small(format!("{hidden} older revision(s) omitted from this panel."));
                    }
                    for index in (0..revision_rows.len()).rev().take(25) {
                        let revision = &revision_rows[index];
                        let previous = index
                            .checked_sub(1)
                            .and_then(|previous| revision_rows.get(previous))
                            .map(|revision| &revision.event);
                        ui.group(|ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(format!("Revision {}", index + 1)).strong());
                                ui.small(event_revision_change_summary(previous, &revision.event));
                            });
                            inspector_row(ui, "Recorded", &revision.recorded_at.to_rfc3339());
                            inspector_row(
                                ui,
                                "Event updated",
                                &revision.event_updated_at.to_rfc3339(),
                            );
                            inspector_row(ui, "Title", &revision.event.normalized_title);
                            inspector_row(ui, "Status", revision.event.status.as_str());
                            inspector_row(
                                ui,
                                "Placement",
                                &revision.event.display_time_label(self.timezone()),
                            );
                        });
                    }
                },
            );

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
            if let Some(occurrence) = self.occurrence_contexts.get(&event.id) {
                let occurrence_label = match occurrence.origin {
                    RecurrenceOccurrenceOrigin::Rule => occurrence.recurrence_index.map_or_else(
                        || "Rule occurrence".to_string(),
                        |index| format!("Rule occurrence {}", index.saturating_add(1)),
                    ),
                    _ => occurrence.origin.as_str().to_string(),
                };
                inspector_row(ui, "Occurrence", &occurrence_label);
                if occurrence.override_applied {
                    let moved = occurrence.original_time != event.time;
                    let override_label = match (moved, occurrence.cancelled_by_override) {
                        (true, true) => "moved + cancelled",
                        (true, false) => "moved",
                        (false, true) => "cancelled",
                        (false, false) => "override",
                    };
                    inspector_row(ui, "Override", override_label);
                    inspector_row(
                        ui,
                        "Original occurrence",
                        &occurrence_time_label(&occurrence.original_time, self.timezone()),
                    );
                }
                inspector_row(ui, "Occurrence ID", &event.id.to_string());
                inspector_row(ui, "Event ID", &canonical_id.to_string());
            } else {
                inspector_row(ui, "Event ID", &event.id.to_string());
            }
            inspector_row(ui, "Created", &event.created_at.to_rfc3339());
            inspector_row(ui, "Updated", &event.updated_at.to_rfc3339());
        });

        match event_details_action {
            Some(EventDetailsEditorAction::Save) => self.save_event_details_edit(),
            Some(EventDetailsEditorAction::Cancel) => {
                self.event_details_editor = None;
                self.last_error = None;
            }
            None => {}
        }
        match event_location_action {
            Some(EventLocationEditorAction::Save) => self.save_event_location_edit(),
            Some(EventLocationEditorAction::Cancel) => {
                self.event_location_editor = None;
                self.last_error = None;
            }
            None => {}
        }
        if refresh_participant_entity_search {
            self.refresh_participant_entity_candidates();
        }
        if refresh_relation_search {
            self.refresh_topology_relation_candidates(canonical_id);
        }
        if refresh_identity_search {
            self.refresh_identity_candidates(canonical_id);
        }
        if let Some(action) = topology_action {
            self.apply_topology_action(canonical_id, action);
        }
        if let Some(action) = participant_action {
            self.apply_participant_action(canonical_id, action);
        }
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
        self.poll_remote_ics_import();
        self.poll_conflict_alternatives();
        self.poll_recurring_occurrence_alternatives();
        self.refresh_notification_evaluation_if_needed();
        if self.taria_update_receiver.is_some()
            || self.remote_ics_import_receiver.is_some()
            || self.conflict_alternative_receiver.is_some()
            || self.recurrence_alternative_receiver.is_some()
        {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        } else if self.notification_rules.iter().any(|rule| rule.enabled) {
            ui.ctx().request_repaint_after(Duration::from_secs(30));
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
            self.import_dropped_path(&path);
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
                                    memberships: &self.event_memberships,
                                    occurrence_contexts: &self.occurrence_contexts,
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
    collections: Vec<EventCollection>,
    relation_types: Vec<String>,
    annotation_kinds: Vec<String>,
    canonical_entities: Vec<CanonicalEntity>,
    canonical_entity_types: Vec<String>,
}

fn render_contextual_week_start(ui: &mut egui::Ui, draft: &mut RecurrenceEditDraft) {
    let has_week_no = !draft.week_no_text.trim().is_empty();
    let available = recurrence_editor_week_start_available(
        draft.rule.frequency,
        !draft.rule.by_weekday.is_empty(),
        has_week_no,
    );

    if available {
        egui::ComboBox::from_label("Week start")
            .selected_text(draft.rule.week_start.short_label())
            .show_ui(ui, |ui| {
                for weekday in RecurrenceWeekday::ALL {
                    ui.selectable_value(&mut draft.rule.week_start, weekday, weekday.short_label());
                }
            });
        return;
    }

    if draft.rule.week_start != RecurrenceWeekday::Monday {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Week start (preserved)").strong());
                ui.label(draft.rule.week_start.short_label());
                if ui.small_button("Reset to Monday").clicked() {
                    draft.rule.week_start = RecurrenceWeekday::Monday;
                }
            });
            ui.small(
                "Custom WKST is active only for WEEKLY+BYDAY or YEARLY+BYWEEKNO; the current value is preserved until reset or that context is restored.",
            );
        });
    } else if draft.rule.frequency == RecurrenceFrequency::Weekly {
        ui.small("Week start becomes editable after at least one BYDAY weekday is selected.");
    } else if draft.rule.frequency == RecurrenceFrequency::Yearly {
        ui.small("Week start becomes editable when BYWEEKNO is populated.");
    }
}

fn render_structured_weekno(ui: &mut egui::Ui, draft: &mut RecurrenceEditDraft, available: bool) {
    if !available {
        if draft.week_no_text.trim().is_empty() && draft.week_no_rows.is_empty() {
            return;
        }

        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("BYWEEKNO (preserved)").strong());
                ui.add_enabled(
                    false,
                    egui::TextEdit::singleline(&mut draft.week_no_text)
                        .desired_width(150.0)
                        .hint_text("20,-1"),
                );
                if ui.small_button("Clear").clicked() {
                    draft.week_no_text.clear();
                    draft.week_no_rows.clear();
                }
            });
            ui.small(RecurrenceEditorSelector::WeekNo.unavailable_reason());
            ui.small("The structured rows are preserved until cleared or the context is restored.");
        });
        return;
    }

    ui.label("BYWEEKNO");
    ui.small(
        "One signed week number per row. Positive values count from week 1; negative values count backward from the final numbered week.",
    );

    let mut structured_changed = false;
    let mut remove_index = None;
    for (index, value) in draft.week_no_rows.iter_mut().enumerate() {
        ui.push_id(("byweekno", index), |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("#{}", index + 1));
                structured_changed |= ui
                    .add(
                        egui::TextEdit::singleline(value)
                            .desired_width(84.0)
                            .hint_text("20 or -1"),
                    )
                    .changed();
                if ui.small_button("Remove").clicked() {
                    remove_index = Some(index);
                }
            });
        });
    }

    if let Some(index) = remove_index {
        draft.week_no_rows.remove(index);
        structured_changed = true;
    }

    if ui.small_button("+ Add BYWEEKNO").clicked() {
        draft.week_no_rows.push("1".to_string());
        structured_changed = true;
    }

    if structured_changed {
        draft.week_no_text = format_recurrence_weekno_edit_rows(&draft.week_no_rows);
    }

    egui::CollapsingHeader::new("Raw BYWEEKNO syntax")
        .default_open(false)
        .show(ui, |ui| {
            ui.small("Power-user form: comma- or space-separated signed weeks such as 20,-1.");
            ui.add(
                egui::TextEdit::singleline(&mut draft.week_no_text)
                    .desired_width(260.0)
                    .hint_text("20,-1"),
            );
            if ui.small_button("Load raw syntax into rows").clicked()
                && let Ok(rows) = parse_recurrence_weekno_edit_rows(&draft.week_no_text)
            {
                draft.week_no_text = format_recurrence_weekno_edit_rows(&rows);
                draft.week_no_rows = rows;
            }
        });
}

fn render_structured_yearday(ui: &mut egui::Ui, draft: &mut RecurrenceEditDraft, available: bool) {
    if !available {
        if draft.year_day_text.trim().is_empty() && draft.year_day_rows.is_empty() {
            return;
        }

        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("BYYEARDAY (preserved)").strong());
                ui.add_enabled(
                    false,
                    egui::TextEdit::singleline(&mut draft.year_day_text)
                        .desired_width(150.0)
                        .hint_text("1,100,-1"),
                );
                if ui.small_button("Clear").clicked() {
                    draft.year_day_text.clear();
                    draft.year_day_rows.clear();
                }
            });
            ui.small(RecurrenceEditorSelector::YearDay.unavailable_reason());
            ui.small("The structured rows are preserved until cleared or the context is restored.");
        });
        return;
    }

    ui.label("BYYEARDAY");
    ui.small(
        "One signed year day per row. Positive values count from January 1; negative values count backward from year end.",
    );

    let mut structured_changed = false;
    let mut remove_index = None;
    for (index, value) in draft.year_day_rows.iter_mut().enumerate() {
        ui.push_id(("byyearday", index), |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("#{}", index + 1));
                structured_changed |= ui
                    .add(
                        egui::TextEdit::singleline(value)
                            .desired_width(90.0)
                            .hint_text("100 or -1"),
                    )
                    .changed();
                if ui.small_button("Remove").clicked() {
                    remove_index = Some(index);
                }
            });
        });
    }

    if let Some(index) = remove_index {
        draft.year_day_rows.remove(index);
        structured_changed = true;
    }

    if ui.small_button("+ Add BYYEARDAY").clicked() {
        draft.year_day_rows.push("1".to_string());
        structured_changed = true;
    }

    if structured_changed {
        draft.year_day_text = format_recurrence_yearday_edit_rows(&draft.year_day_rows);
    }

    egui::CollapsingHeader::new("Raw BYYEARDAY syntax")
        .default_open(false)
        .show(ui, |ui| {
            ui.small(
                "Power-user form: comma- or space-separated signed year days such as 1,100,-1.",
            );
            ui.add(
                egui::TextEdit::singleline(&mut draft.year_day_text)
                    .desired_width(260.0)
                    .hint_text("1,100,-1"),
            );
            if ui.small_button("Load raw syntax into rows").clicked()
                && let Ok(rows) = parse_recurrence_yearday_edit_rows(&draft.year_day_text)
            {
                draft.year_day_text = format_recurrence_yearday_edit_rows(&rows);
                draft.year_day_rows = rows;
            }
        });
}

fn render_structured_monthday(ui: &mut egui::Ui, draft: &mut RecurrenceEditDraft, available: bool) {
    if !available {
        if draft.month_day_text.trim().is_empty() && draft.month_day_rows.is_empty() {
            return;
        }

        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("BYMONTHDAY (preserved)").strong());
                ui.add_enabled(
                    false,
                    egui::TextEdit::singleline(&mut draft.month_day_text)
                        .desired_width(150.0)
                        .hint_text("1,15,-1"),
                );
                if ui.small_button("Clear").clicked() {
                    draft.month_day_text.clear();
                    draft.month_day_rows.clear();
                }
            });
            ui.small(RecurrenceEditorSelector::MonthDay.unavailable_reason());
            ui.small("The structured rows are preserved until cleared or the context is restored.");
        });
        return;
    }

    ui.label("BYMONTHDAY");
    ui.small(
        "One signed civil day per row. Positive values count from month start; negative values count backward from month end.",
    );

    let mut structured_changed = false;
    let mut remove_index = None;
    for (index, value) in draft.month_day_rows.iter_mut().enumerate() {
        ui.push_id(("bymonthday", index), |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("#{}", index + 1));
                structured_changed |= ui
                    .add(
                        egui::TextEdit::singleline(value)
                            .desired_width(80.0)
                            .hint_text("1 or -1"),
                    )
                    .changed();
                if ui.small_button("Remove").clicked() {
                    remove_index = Some(index);
                }
            });
        });
    }

    if let Some(index) = remove_index {
        draft.month_day_rows.remove(index);
        structured_changed = true;
    }

    if ui.small_button("+ Add BYMONTHDAY").clicked() {
        draft.month_day_rows.push("1".to_string());
        structured_changed = true;
    }

    if structured_changed {
        draft.month_day_text = format_recurrence_monthday_edit_rows(&draft.month_day_rows);
    }

    egui::CollapsingHeader::new("Raw BYMONTHDAY syntax")
        .default_open(false)
        .show(ui, |ui| {
            ui.small("Power-user form: comma- or space-separated signed days such as 1,15,-1.");
            ui.add(
                egui::TextEdit::singleline(&mut draft.month_day_text)
                    .desired_width(260.0)
                    .hint_text("1,15,-1"),
            );
            if ui.small_button("Load raw syntax into rows").clicked()
                && let Ok(rows) = parse_recurrence_monthday_edit_rows(&draft.month_day_text)
            {
                draft.month_day_text = format_recurrence_monthday_edit_rows(&rows);
                draft.month_day_rows = rows;
            }
        });
}

fn render_structured_hour(ui: &mut egui::Ui, draft: &mut RecurrenceEditDraft, available: bool) {
    if !available {
        if draft.hour_text.trim().is_empty() && draft.hour_rows.is_empty() {
            return;
        }

        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("BYHOUR (preserved)").strong());
                ui.add_enabled(
                    false,
                    egui::TextEdit::singleline(&mut draft.hour_text)
                        .desired_width(150.0)
                        .hint_text("9,17"),
                );
                if ui.small_button("Clear").clicked() {
                    draft.hour_text.clear();
                    draft.hour_rows.clear();
                }
            });
            ui.small(RecurrenceEditorSelector::Hour.unavailable_reason());
            ui.small("The structured rows are preserved until cleared or the context is restored.");
        });
        return;
    }

    ui.label("BYHOUR");
    ui.small("One civil hour per row, using 0 through 23.");

    let mut structured_changed = false;
    let mut remove_index = None;
    for (index, value) in draft.hour_rows.iter_mut().enumerate() {
        ui.push_id(("byhour", index), |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("#{}", index + 1));
                structured_changed |= ui
                    .add(
                        egui::TextEdit::singleline(value)
                            .desired_width(84.0)
                            .hint_text("0..23"),
                    )
                    .changed();
                if ui.small_button("Remove").clicked() {
                    remove_index = Some(index);
                }
            });
        });
    }

    if let Some(index) = remove_index {
        draft.hour_rows.remove(index);
        structured_changed = true;
    }

    if ui.small_button("+ Add BYHOUR").clicked() {
        draft.hour_rows.push("0".to_string());
        structured_changed = true;
    }

    if structured_changed {
        draft.hour_text = format_recurrence_hour_edit_rows(&draft.hour_rows);
    }

    egui::CollapsingHeader::new("Raw BYHOUR syntax")
        .default_open(false)
        .show(ui, |ui| {
            ui.small("Power-user form: comma- or space-separated hours such as 9,17.");
            ui.add(
                egui::TextEdit::singleline(&mut draft.hour_text)
                    .desired_width(260.0)
                    .hint_text("9,17"),
            );
            if ui.small_button("Load raw syntax into rows").clicked()
                && let Ok(rows) = parse_recurrence_hour_edit_rows(&draft.hour_text)
            {
                draft.hour_text = format_recurrence_hour_edit_rows(&rows);
                draft.hour_rows = rows;
            }
        });
}

fn render_structured_minute(ui: &mut egui::Ui, draft: &mut RecurrenceEditDraft, available: bool) {
    if !available {
        if draft.minute_text.trim().is_empty() && draft.minute_rows.is_empty() {
            return;
        }

        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("BYMINUTE (preserved)").strong());
                ui.add_enabled(
                    false,
                    egui::TextEdit::singleline(&mut draft.minute_text)
                        .desired_width(150.0)
                        .hint_text("0,30"),
                );
                if ui.small_button("Clear").clicked() {
                    draft.minute_text.clear();
                    draft.minute_rows.clear();
                }
            });
            ui.small(RecurrenceEditorSelector::Minute.unavailable_reason());
            ui.small("The structured rows are preserved until cleared or the context is restored.");
        });
        return;
    }

    ui.label("BYMINUTE");
    ui.small("One civil minute per row, using 0 through 59.");

    let mut structured_changed = false;
    let mut remove_index = None;
    for (index, value) in draft.minute_rows.iter_mut().enumerate() {
        ui.push_id(("byminute", index), |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("#{}", index + 1));
                structured_changed |= ui
                    .add(
                        egui::TextEdit::singleline(value)
                            .desired_width(84.0)
                            .hint_text("0..59"),
                    )
                    .changed();
                if ui.small_button("Remove").clicked() {
                    remove_index = Some(index);
                }
            });
        });
    }

    if let Some(index) = remove_index {
        draft.minute_rows.remove(index);
        structured_changed = true;
    }

    if ui.small_button("+ Add BYMINUTE").clicked() {
        draft.minute_rows.push("0".to_string());
        structured_changed = true;
    }

    if structured_changed {
        draft.minute_text = format_recurrence_minute_edit_rows(&draft.minute_rows);
    }

    egui::CollapsingHeader::new("Raw BYMINUTE syntax")
        .default_open(false)
        .show(ui, |ui| {
            ui.small("Power-user form: comma- or space-separated minutes such as 0,30.");
            ui.add(
                egui::TextEdit::singleline(&mut draft.minute_text)
                    .desired_width(260.0)
                    .hint_text("0,30"),
            );
            if ui.small_button("Load raw syntax into rows").clicked()
                && let Ok(rows) = parse_recurrence_minute_edit_rows(&draft.minute_text)
            {
                draft.minute_text = format_recurrence_minute_edit_rows(&rows);
                draft.minute_rows = rows;
            }
        });
}

fn render_structured_second(ui: &mut egui::Ui, draft: &mut RecurrenceEditDraft, available: bool) {
    if !available {
        if draft.second_text.trim().is_empty() && draft.second_rows.is_empty() {
            return;
        }

        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("BYSECOND (preserved)").strong());
                ui.add_enabled(
                    false,
                    egui::TextEdit::singleline(&mut draft.second_text)
                        .desired_width(150.0)
                        .hint_text("0,15,30,45"),
                );
                if ui.small_button("Clear").clicked() {
                    draft.second_text.clear();
                    draft.second_rows.clear();
                }
            });
            ui.small(RecurrenceEditorSelector::Second.unavailable_reason());
            ui.small("The structured rows are preserved until cleared or the context is restored.");
        });
        return;
    }

    ui.label("BYSECOND");
    ui.small("One ordinary civil second per row, using 0 through 59.");

    let mut structured_changed = false;
    let mut remove_index = None;
    for (index, value) in draft.second_rows.iter_mut().enumerate() {
        ui.push_id(("bysecond", index), |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("#{}", index + 1));
                structured_changed |= ui
                    .add(
                        egui::TextEdit::singleline(value)
                            .desired_width(84.0)
                            .hint_text("0..59"),
                    )
                    .changed();
                if ui.small_button("Remove").clicked() {
                    remove_index = Some(index);
                }
            });
        });
    }

    if let Some(index) = remove_index {
        draft.second_rows.remove(index);
        structured_changed = true;
    }

    if ui.small_button("+ Add BYSECOND").clicked() {
        draft.second_rows.push("0".to_string());
        structured_changed = true;
    }

    if structured_changed {
        draft.second_text = format_recurrence_second_edit_rows(&draft.second_rows);
    }

    egui::CollapsingHeader::new("Raw BYSECOND syntax")
        .default_open(false)
        .show(ui, |ui| {
            ui.small(
                "Power-user form: comma- or space-separated ordinary seconds such as 0,15,30,45.",
            );
            ui.add(
                egui::TextEdit::singleline(&mut draft.second_text)
                    .desired_width(260.0)
                    .hint_text("0,15,30,45"),
            );
            if ui.small_button("Load raw syntax into rows").clicked()
                && let Ok(rows) = parse_recurrence_second_edit_rows(&draft.second_text)
            {
                draft.second_text = format_recurrence_second_edit_rows(&rows);
                draft.second_rows = rows;
            }
        });
}

fn render_structured_set_pos(ui: &mut egui::Ui, draft: &mut RecurrenceEditDraft) {
    ui.label("BYSETPOS");
    ui.small("One signed set position per row, using -366 through -1 or 1 through 366.");

    let mut structured_changed = false;
    let mut remove_index = None;
    for (index, value) in draft.set_pos_rows.iter_mut().enumerate() {
        ui.push_id(("bysetpos", index), |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("#{}", index + 1));
                structured_changed |= ui
                    .add(
                        egui::TextEdit::singleline(value)
                            .desired_width(84.0)
                            .hint_text("1 or -1"),
                    )
                    .changed();
                if ui.small_button("Remove").clicked() {
                    remove_index = Some(index);
                }
            });
        });
    }

    if let Some(index) = remove_index {
        draft.set_pos_rows.remove(index);
        structured_changed = true;
    }

    if ui.small_button("+ Add BYSETPOS").clicked() {
        draft.set_pos_rows.push("1".to_string());
        structured_changed = true;
    }

    if structured_changed {
        draft.set_pos_text = format_recurrence_setpos_edit_rows(&draft.set_pos_rows);
    }

    egui::CollapsingHeader::new("Raw BYSETPOS syntax")
        .default_open(false)
        .show(ui, |ui| {
            ui.small("Power-user form: comma- or space-separated signed positions such as 1,-1.");
            ui.add(
                egui::TextEdit::singleline(&mut draft.set_pos_text)
                    .desired_width(260.0)
                    .hint_text("1,-1"),
            );
            if ui.small_button("Load raw syntax into rows").clicked()
                && let Ok(rows) = parse_recurrence_setpos_edit_rows(&draft.set_pos_text)
            {
                draft.set_pos_text = format_recurrence_setpos_edit_rows(&rows);
                draft.set_pos_rows = rows;
            }
        });

    if !draft.set_pos_text.trim().is_empty() || !draft.set_pos_rows.is_empty() {
        ui.small(
            "BYSETPOS requires at least one other BY selector; live validation enforces that rule.",
        );
    }
}

fn render_structured_ordinal_byday(
    ui: &mut egui::Ui,
    draft: &mut RecurrenceEditDraft,
    available: bool,
) {
    if !available {
        if draft.ordinal_byday_text.trim().is_empty() && draft.ordinal_byday_rows.is_empty() {
            return;
        }

        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Ordinal BYDAY (preserved)").strong());
                ui.add_enabled(
                    false,
                    egui::TextEdit::singleline(&mut draft.ordinal_byday_text)
                        .desired_width(150.0)
                        .hint_text("1MO,-1FR"),
                );
                if ui.small_button("Clear").clicked() {
                    draft.ordinal_byday_text.clear();
                    draft.ordinal_byday_rows.clear();
                }
            });
            ui.small(RecurrenceEditorSelector::OrdinalByDay.unavailable_reason());
            ui.small("The structured rows are preserved until cleared or the context is restored.");
        });
        return;
    }

    ui.label("Ordinal BYDAY");
    ui.small(
        "One ordinal weekday per row. Positive ordinals count from the start; negative ordinals count from the end.",
    );

    let mut structured_changed = false;
    let mut remove_index = None;

    for (index, row) in draft.ordinal_byday_rows.iter_mut().enumerate() {
        ui.push_id(("ordinal_byday", index), |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("#{}", index + 1));
                structured_changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut row.ordinal_text)
                            .desired_width(64.0)
                            .hint_text("1 or -1"),
                    )
                    .changed();

                let previous_weekday = row.weekday;
                egui::ComboBox::from_id_salt(("ordinal_byday_weekday", index))
                    .selected_text(row.weekday.short_label())
                    .show_ui(ui, |ui| {
                        for weekday in RecurrenceWeekday::ALL {
                            ui.selectable_value(&mut row.weekday, weekday, weekday.short_label());
                        }
                    });
                structured_changed |= row.weekday != previous_weekday;

                if ui.small_button("Remove").clicked() {
                    remove_index = Some(index);
                }
            });
        });
    }

    if let Some(index) = remove_index {
        draft.ordinal_byday_rows.remove(index);
        structured_changed = true;
    }

    if ui.small_button("+ Add ordinal BYDAY").clicked() {
        draft
            .ordinal_byday_rows
            .push(RecurrenceOrdinalByDayEditRow {
                ordinal_text: "1".to_string(),
                weekday: RecurrenceWeekday::Monday,
            });
        structured_changed = true;
    }

    if structured_changed {
        draft.ordinal_byday_text =
            format_recurrence_ordinal_byday_edit_rows(&draft.ordinal_byday_rows);
    }

    egui::CollapsingHeader::new("Raw ordinal BYDAY syntax")
        .default_open(false)
        .show(ui, |ui| {
            ui.small("Power-user form: tokens such as 1MO,-1FR.");
            ui.add(
                egui::TextEdit::singleline(&mut draft.ordinal_byday_text)
                    .desired_width(260.0)
                    .hint_text("1MO,-1FR"),
            );
            if ui.small_button("Load raw syntax into rows").clicked()
                && let Ok(rows) =
                    parse_recurrence_ordinal_byday_edit_rows(&draft.ordinal_byday_text)
            {
                draft.ordinal_byday_text = format_recurrence_ordinal_byday_edit_rows(&rows);
                draft.ordinal_byday_rows = rows;
            }
        });
}

fn render_structured_exception_dates(
    ui: &mut egui::Ui,
    title: &str,
    parser_label: &str,
    rows: &mut Vec<String>,
    raw_text: &mut String,
    base_time: &TimeSpec,
) {
    ui.label(title);
    ui.small("One occurrence start per row; raw compact syntax remains available below.");

    let value_hint = recurrence_exception_value_hint(base_time);
    let mut structured_changed = false;
    let mut remove_index = None;

    for (index, value) in rows.iter_mut().enumerate() {
        ui.push_id((parser_label, index), |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("#{}", index + 1));
                structured_changed |= ui
                    .add(
                        egui::TextEdit::singleline(value)
                            .desired_width(240.0)
                            .hint_text(value_hint),
                    )
                    .changed();
                if ui.small_button("Remove").clicked() {
                    remove_index = Some(index);
                }
            });
        });
    }

    if let Some(index) = remove_index {
        rows.remove(index);
        structured_changed = true;
    }

    if ui.small_button(format!("+ Add {parser_label}")).clicked() {
        rows.push(String::new());
        structured_changed = true;
    }

    if structured_changed {
        *raw_text = format_recurrence_exception_edit_rows(rows);
    }

    egui::CollapsingHeader::new(format!("Raw {parser_label} syntax"))
        .default_open(false)
        .show(ui, |ui| {
            ui.small("Power-user form: comma- or space-separated occurrence starts.");
            let raw_hint = recurrence_exception_hint(base_time);
            ui.add(
                egui::TextEdit::multiline(raw_text)
                    .desired_width(430.0)
                    .desired_rows(2)
                    .hint_text(raw_hint),
            );
            if ui
                .small_button(format!("Load raw {parser_label} syntax into rows"))
                .clicked()
                && let Ok(parsed_rows) =
                    parse_recurrence_exception_edit_rows(raw_text, base_time, parser_label)
            {
                *rows = parsed_rows;
            }
        });
}

fn render_structured_recurrence_overrides(ui: &mut egui::Ui, draft: &mut RecurrenceEditDraft) {
    ui.label("Occurrence overrides");
    ui.small("Edit each exception explicitly; raw RFC-like syntax remains available below.");

    let value_hint = recurrence_exception_value_hint(&draft.base_time);
    let mut structured_changed = false;
    let mut remove_index = None;

    for index in 0..draft.override_rows.len() {
        ui.push_id(("recurrence-override-row", index), |ui| {
            let row = &mut draft.override_rows[index];
            ui.horizontal(|ui| {
                ui.label(format!("#{}", index + 1));
                structured_changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut row.original_text)
                            .desired_width(150.0)
                            .hint_text(value_hint),
                    )
                    .changed();

                let previous_action = row.action;
                egui::ComboBox::from_id_salt("action")
                    .selected_text(row.action.label())
                    .show_ui(ui, |ui| {
                        for action in [
                            RecurrenceOverrideEditAction::Move,
                            RecurrenceOverrideEditAction::Cancel,
                            RecurrenceOverrideEditAction::CancelMove,
                            RecurrenceOverrideEditAction::Keep,
                        ] {
                            ui.selectable_value(&mut row.action, action, action.label());
                        }
                    });
                structured_changed |= row.action != previous_action;

                if row.action.needs_replacement() {
                    structured_changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut row.replacement_text)
                                .desired_width(150.0)
                                .hint_text(value_hint),
                        )
                        .changed();
                } else {
                    ui.add_sized([150.0, 18.0], egui::Label::new("no replacement"));
                }

                if ui.small_button("Remove").clicked() {
                    remove_index = Some(index);
                }
            });
        });
    }

    if let Some(index) = remove_index {
        draft.override_rows.remove(index);
        structured_changed = true;
    }

    if ui.small_button("+ Add override").clicked() {
        draft.override_rows.push(RecurrenceOverrideEditRow {
            original_text: String::new(),
            action: RecurrenceOverrideEditAction::Cancel,
            replacement_text: String::new(),
        });
        structured_changed = true;
    }

    if structured_changed {
        draft.override_text = format_recurrence_override_edit_rows(&draft.override_rows);
    }

    egui::CollapsingHeader::new("Raw override syntax")
        .default_open(false)
        .show(ui, |ui| {
            ui.small(
                "Power-user form: original=>replacement, original=>CANCEL, original=>CANCEL@replacement, or original=>KEEP.",
            );
            let override_hint = recurrence_override_hint(&draft.base_time);
            ui.add(
                egui::TextEdit::multiline(&mut draft.override_text)
                    .desired_width(430.0)
                    .desired_rows(3)
                    .hint_text(override_hint),
            );
            if ui.small_button("Load raw syntax into rows").clicked()
                && let Ok(rows) =
                    parse_recurrence_override_edit_rows(&draft.override_text, &draft.base_time)
            {
                draft.override_rows = rows;
            }
        });
}

fn render_focused_occurrence_override(
    ui: &mut egui::Ui,
    draft: &mut RecurrenceEditDraft,
    timezone: Tz,
) -> Option<RecurrenceEditorAction> {
    let original = draft.focused_occurrence_original.clone()?;
    if draft.alternative_rule.is_some()
        && draft.alternative_rule.as_ref() != draft.parsed_rule().ok().as_ref()
    {
        draft.alternative_slots.clear();
        draft.alternative_rule = None;
        draft.alternative_note = Some("Recurrence draft changed; search again.".to_string());
    }
    let slot = format_exception_start_value(&original);
    ui.group(|ui| {
        ui.strong("Selected occurrence");
        ui.small(format!("Original recurrence slot: {slot}"));
        ui.small("Only this original slot is affected. Changes are not stored until Save.");
        if draft.focused_conflict_confirmed_rule.is_some()
            && draft.focused_conflict_confirmed_rule.as_ref() != draft.parsed_rule().ok().as_ref()
        {
            draft.focused_conflict_confirmed_rule = None;
            draft.focused_conflict_warning = None;
        }
        if let Some(warning) = draft.focused_conflict_warning.as_deref() {
            ui.colored_label(Color32::from_rgb(220, 150, 70), warning);
        }

        let existing = draft
            .override_rows
            .iter()
            .position(|row| row.original_text == slot);
        if let Some(index) = existing {
            let row = &mut draft.override_rows[index];
            let mut changed = false;
            ui.horizontal_wrapped(|ui| {
                let action_before = row.action;
                egui::ComboBox::from_id_salt("focused-occurrence-action")
                    .selected_text(row.action.label())
                    .show_ui(ui, |ui| {
                        for action in [
                            RecurrenceOverrideEditAction::Move,
                            RecurrenceOverrideEditAction::Cancel,
                            RecurrenceOverrideEditAction::CancelMove,
                            RecurrenceOverrideEditAction::Keep,
                        ] {
                            ui.selectable_value(&mut row.action, action, action.label());
                        }
                    });
                changed |= row.action != action_before;
                if row.action.needs_replacement() {
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut row.replacement_text)
                                .hint_text(recurrence_exception_value_hint(&draft.base_time))
                                .desired_width(180.0),
                        )
                        .changed();
                }
            });
            let clear_label = if matches!(
                row.action,
                RecurrenceOverrideEditAction::Cancel | RecurrenceOverrideEditAction::CancelMove
            ) {
                "Restore original occurrence"
            } else {
                "Remove occurrence override"
            };
            if ui.small_button(clear_label).clicked() {
                draft.override_rows.remove(index);
                changed = true;
            }
            if changed {
                draft.override_text = format_recurrence_override_edit_rows(&draft.override_rows);
            }
        } else {
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("Move this occurrence").clicked() {
                    draft.override_rows.push(RecurrenceOverrideEditRow {
                        original_text: slot.clone(),
                        action: RecurrenceOverrideEditAction::Move,
                        replacement_text: String::new(),
                    });
                    draft.override_text =
                        format_recurrence_override_edit_rows(&draft.override_rows);
                }
                if ui.small_button("Cancel this occurrence").clicked() {
                    draft.override_rows.push(RecurrenceOverrideEditRow {
                        original_text: slot.clone(),
                        action: RecurrenceOverrideEditAction::Cancel,
                        replacement_text: String::new(),
                    });
                    draft.override_text =
                        format_recurrence_override_edit_rows(&draft.override_rows);
                }
            });
            ui.small("No override yet; opening this editor does not modify the series.");
        }
    });

    if !matches!(
        draft.focused_occurrence_current,
        Some(TimeSpec::Instant { .. } | TimeSpec::Floating { .. } | TimeSpec::AllDay { .. })
    ) {
        ui.small("Automatic openings require a definite timed or all-day occurrence.");
        return None;
    }

    let cancelled = draft.parsed_rule().ok().is_some_and(|rule| {
        rule.overrides
            .iter()
            .any(|value| value.original == original && value.cancelled)
    });
    let mut action = None;
    let button_label = if cancelled {
        "Find restoration openings"
    } else {
        "Find later openings"
    };
    if ui.small_button(button_label).clicked() {
        action = Some(RecurrenceEditorAction::FindAlternatives);
    }
    if let Some(note) = &draft.alternative_note {
        ui.small(note);
    }
    ui.horizontal_wrapped(|ui| {
        for proposed in &draft.alternative_slots {
            let label = if matches!(&draft.base_time, TimeSpec::AllDay { .. }) {
                format_civil_alternative(proposed, timezone)
            } else {
                format_availability_interval(proposed.start_utc, proposed.end_utc, timezone)
            };
            if ui.small_button(label).clicked() {
                action = Some(RecurrenceEditorAction::UseAlternative(proposed.clone()));
            }
        }
    });
    action
}

fn render_recurrence_editor(
    ui: &mut egui::Ui,
    draft: &mut RecurrenceEditDraft,
    timezone: Tz,
) -> Option<RecurrenceEditorAction> {
    ui.strong(if draft.had_recurrence {
        "Edit recurrence"
    } else {
        "Add recurrence"
    });
    ui.small("Edits the canonical series definition, not the selected materialized occurrence.");

    let focused_action = render_focused_occurrence_override(ui, draft, timezone);

    ui.label("Quick presets");
    ui.horizontal_wrapped(|ui| {
        for preset in RecurrencePreset::ALL {
            if ui.small_button(preset.label()).clicked() {
                draft.apply_preset(preset);
            }
        }
    });
    ui.small(
        "Presets reset cadence selectors and interval to 1; COUNT, UNTIL, and exceptions are preserved.",
    );

    egui::ComboBox::from_label("Frequency")
        .selected_text(draft.rule.frequency.as_str())
        .show_ui(ui, |ui| {
            for frequency in RecurrenceFrequency::ALL {
                if recurrence_editor_frequency_available(frequency, &draft.base_time) {
                    ui.selectable_value(&mut draft.rule.frequency, frequency, frequency.as_str());
                }
            }
        });
    if !recurrence_editor_base_is_datetime(&draft.base_time) {
        ui.small(
            "Secondly, minutely, and hourly frequencies require a date-time series and are hidden.",
        );
    }

    ui.horizontal(|ui| {
        ui.label("Interval");
        ui.add(
            egui::TextEdit::singleline(&mut draft.interval_text)
                .desired_width(70.0)
                .hint_text("1"),
        );
    });
    ui.horizontal(|ui| {
        ui.label("Count");
        ui.add(
            egui::TextEdit::singleline(&mut draft.count_text)
                .desired_width(90.0)
                .hint_text("unbounded"),
        );
    });
    ui.horizontal(|ui| {
        ui.label("Until");
        ui.add(
            egui::TextEdit::singleline(&mut draft.until_text)
                .desired_width(120.0)
                .hint_text("YYYY-MM-DD"),
        );
    });

    ui.label("BYDAY");
    ui.horizontal_wrapped(|ui| {
        for weekday in RecurrenceWeekday::ALL {
            let mut selected = draft.rule.by_weekday.contains(&weekday);
            if ui.checkbox(&mut selected, weekday.short_label()).changed() {
                if selected {
                    draft.rule.by_weekday.push(weekday);
                } else {
                    draft.rule.by_weekday.retain(|value| *value != weekday);
                }
            }
        }
    });
    render_contextual_week_start(ui, draft);

    ui.label("BYMONTH");
    ui.horizontal_wrapped(|ui| {
        for month in 1_u8..=12 {
            let mut selected = draft.rule.by_month.contains(&month);
            if ui.checkbox(&mut selected, month.to_string()).changed() {
                if selected {
                    draft.rule.by_month.push(month);
                } else {
                    draft.rule.by_month.retain(|value| *value != month);
                }
            }
        }
    });

    ui.separator();
    ui.small(
        "Advanced selectors adapt to the current frequency and event time kind. Inapplicable empty fields are hidden; existing values are preserved and can be explicitly cleared.",
    );
    let frequency = draft.rule.frequency;
    let week_no_available = recurrence_editor_selector_available(
        RecurrenceEditorSelector::WeekNo,
        frequency,
        &draft.base_time,
    );
    let year_day_available = recurrence_editor_selector_available(
        RecurrenceEditorSelector::YearDay,
        frequency,
        &draft.base_time,
    );
    let month_day_available = recurrence_editor_selector_available(
        RecurrenceEditorSelector::MonthDay,
        frequency,
        &draft.base_time,
    );
    let ordinal_byday_available = recurrence_editor_selector_available(
        RecurrenceEditorSelector::OrdinalByDay,
        frequency,
        &draft.base_time,
    );
    let hour_available = recurrence_editor_selector_available(
        RecurrenceEditorSelector::Hour,
        frequency,
        &draft.base_time,
    );
    let minute_available = recurrence_editor_selector_available(
        RecurrenceEditorSelector::Minute,
        frequency,
        &draft.base_time,
    );
    let second_available = recurrence_editor_selector_available(
        RecurrenceEditorSelector::Second,
        frequency,
        &draft.base_time,
    );
    render_structured_weekno(ui, draft, week_no_available);
    render_structured_yearday(ui, draft, year_day_available);
    render_structured_monthday(ui, draft, month_day_available);
    render_structured_ordinal_byday(ui, draft, ordinal_byday_available);
    render_structured_hour(ui, draft, hour_available);
    render_structured_minute(ui, draft, minute_available);
    render_structured_second(ui, draft, second_available);
    render_structured_set_pos(ui, draft);

    ui.separator();
    ui.small("Recurrence exceptions");
    render_structured_exception_dates(
        ui,
        "RDATE additions",
        "RDATE",
        &mut draft.rdate_rows,
        &mut draft.rdate_text,
        &draft.base_time,
    );
    render_structured_exception_dates(
        ui,
        "EXDATE exclusions",
        "EXDATE",
        &mut draft.exdate_rows,
        &mut draft.exdate_text,
        &draft.base_time,
    );
    render_structured_recurrence_overrides(ui, draft);

    let validation = draft.parsed_rule();
    match &validation {
        Ok(_) => {
            ui.small(RichText::new("Rule is valid.").color(Color32::from_rgb(90, 180, 110)));
        }
        Err(error) => {
            ui.small(RichText::new(error).color(Color32::from_rgb(220, 90, 90)));
        }
    }

    let mut action = None;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(validation.is_ok(), egui::Button::new("Save"))
            .clicked()
        {
            action = Some(RecurrenceEditorAction::Save);
        }
        if ui.button("Cancel").clicked() {
            action = Some(RecurrenceEditorAction::Cancel);
        }
        if draft.had_recurrence && ui.button("Remove recurrence").clicked() {
            action = Some(RecurrenceEditorAction::Remove);
        }
    });
    action.or(focused_action)
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
    UncertainStartOverlaps,
    BundleMembership,
    ProjectedCalendarMembership,
    CollectionMembership,
    RelationType,
    IdentityStateAnyOf,
    AnnotationKind,
    ProvenanceRoleAnyOf,
    ProvenanceReference,
    CanonicalEntityMembership,
    CanonicalEntityType,
}

impl QueryPredicateKind {
    const ALL: [Self; 19] = [
        Self::Text,
        Self::TextAnyOf,
        Self::StatusAnyOf,
        Self::Integer,
        Self::Exists,
        Self::TemporalKindAnyOf,
        Self::DateOverlaps,
        Self::RelativeDateOverlaps,
        Self::UncertainStartOverlaps,
        Self::BundleMembership,
        Self::ProjectedCalendarMembership,
        Self::CollectionMembership,
        Self::RelationType,
        Self::IdentityStateAnyOf,
        Self::AnnotationKind,
        Self::ProvenanceRoleAnyOf,
        Self::ProvenanceReference,
        Self::CanonicalEntityMembership,
        Self::CanonicalEntityType,
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
            Self::UncertainStartOverlaps => "Uncertain start overlap",
            Self::BundleMembership => "Taria bundle membership",
            Self::ProjectedCalendarMembership => "Taria projected calendar membership",
            Self::CollectionMembership => "Event collection membership",
            Self::RelationType => "Event relation type",
            Self::IdentityStateAnyOf => "Event identity state",
            Self::AnnotationKind => "Annotation kind",
            Self::ProvenanceRoleAnyOf => "Provenance role",
            Self::ProvenanceReference => "Provenance reference",
            Self::CanonicalEntityMembership => "Canonical participant entity",
            Self::CanonicalEntityType => "Canonical participant entity type",
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
        QueryPredicateKind::UncertainStartOverlaps => QueryPredicate::UncertainStartOverlaps {
            start: None,
            end_exclusive: None,
        },
        QueryPredicateKind::BundleMembership => QueryPredicate::BundleMembership {
            bundle_ref: String::new(),
        },
        QueryPredicateKind::ProjectedCalendarMembership => {
            QueryPredicate::ProjectedCalendarMembership {
                calendar_id: String::new(),
            }
        }
        QueryPredicateKind::CollectionMembership => QueryPredicate::CollectionMembership {
            collection_id: Uuid::nil(),
        },
        QueryPredicateKind::RelationType => QueryPredicate::RelationType {
            relation_type: String::new(),
            direction: RelationDirection::Either,
        },
        QueryPredicateKind::IdentityStateAnyOf => QueryPredicate::IdentityStateAnyOf {
            values: vec![EventIdentityState::Candidate],
        },
        QueryPredicateKind::AnnotationKind => QueryPredicate::AnnotationKind {
            annotation_kind: String::new(),
        },
        QueryPredicateKind::ProvenanceRoleAnyOf => QueryPredicate::ProvenanceRoleAnyOf {
            values: vec![EventProvenanceRole::Source],
        },
        QueryPredicateKind::ProvenanceReference => QueryPredicate::ProvenanceReference {
            reference: String::new(),
        },
        QueryPredicateKind::CanonicalEntityMembership => {
            QueryPredicate::CanonicalEntityMembership {
                entity_id: Uuid::nil(),
            }
        }
        QueryPredicateKind::CanonicalEntityType => QueryPredicate::CanonicalEntityType {
            entity_type: String::new(),
        },
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
        QueryPredicate::UncertainStartOverlaps { .. } => QueryPredicateKind::UncertainStartOverlaps,
        QueryPredicate::BundleMembership { .. } => QueryPredicateKind::BundleMembership,
        QueryPredicate::ProjectedCalendarMembership { .. } => {
            QueryPredicateKind::ProjectedCalendarMembership
        }
        QueryPredicate::CollectionMembership { .. } => QueryPredicateKind::CollectionMembership,
        QueryPredicate::RelationType { .. } => QueryPredicateKind::RelationType,
        QueryPredicate::IdentityStateAnyOf { .. } => QueryPredicateKind::IdentityStateAnyOf,
        QueryPredicate::AnnotationKind { .. } => QueryPredicateKind::AnnotationKind,
        QueryPredicate::ProvenanceRoleAnyOf { .. } => QueryPredicateKind::ProvenanceRoleAnyOf,
        QueryPredicate::ProvenanceReference { .. } => QueryPredicateKind::ProvenanceReference,
        QueryPredicate::CanonicalEntityMembership { .. } => {
            QueryPredicateKind::CanonicalEntityMembership
        }
        QueryPredicate::CanonicalEntityType { .. } => QueryPredicateKind::CanonicalEntityType,
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
        QueryPredicate::UncertainStartOverlaps {
            start,
            end_exclusive,
        } => {
            ui.small(
                "Matches the bounded possible start-placement window, not the event's occupied duration.",
            );
            changed |= render_optional_date_editor(ui, "Start inclusive", start);
            changed |= render_optional_date_editor(ui, "End exclusive", end_exclusive);
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
        QueryPredicate::CollectionMembership { collection_id } => {
            ui.small("Matches canonical collection/sequence membership without changing event ownership.");
            let selected = membership_options
                .collections
                .iter()
                .find(|collection| collection.id == *collection_id)
                .map(|collection| collection.name.clone())
                .unwrap_or_else(|| {
                    if collection_id.is_nil() {
                        "Select collection…".to_string()
                    } else {
                        collection_id.to_string()
                    }
                });
            egui::ComboBox::from_id_salt(("advanced-collection-id", path))
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for collection in &membership_options.collections {
                        changed |= ui
                            .selectable_value(collection_id, collection.id, &collection.name)
                            .changed();
                    }
                });
        }
        QueryPredicate::RelationType {
            relation_type,
            direction,
        } => {
            ui.small(
                "Matches canonical directed event relations; direction is relative to the event.",
            );
            ui.horizontal_wrapped(|ui| {
                egui::ComboBox::from_id_salt(("advanced-relation-type", path))
                    .selected_text(if relation_type.is_empty() {
                        "Select relation type…"
                    } else {
                        relation_type.as_str()
                    })
                    .show_ui(ui, |ui| {
                        for candidate in &membership_options.relation_types {
                            changed |= ui
                                .selectable_value(relation_type, candidate.clone(), candidate)
                                .changed();
                        }
                    });
                egui::ComboBox::from_id_salt(("advanced-relation-direction", path))
                    .selected_text(direction.label())
                    .show_ui(ui, |ui| {
                        for candidate in RelationDirection::ALL {
                            changed |= ui
                                .selectable_value(direction, candidate, candidate.label())
                                .changed();
                        }
                    });
            });
            changed |= ui
                .add(egui::TextEdit::singleline(relation_type).hint_text("causes, precedes, ..."))
                .changed();
        }
        QueryPredicate::IdentityStateAnyOf { values } => {
            ui.small(
                "Matches external duplicate/entity-resolution assessments without changing event fields.",
            );
            ui.horizontal_wrapped(|ui| {
                for state in EventIdentityState::ALL {
                    let mut selected = values.contains(&state);
                    if ui.checkbox(&mut selected, state.as_str()).changed() {
                        if selected {
                            values.push(state);
                        } else {
                            values.retain(|value| *value != state);
                        }
                        changed = true;
                    }
                }
            });
        }
        QueryPredicate::AnnotationKind { annotation_kind } => {
            ui.small("Matches user-owned event annotations without changing source-backed fields.");
            egui::ComboBox::from_id_salt(("advanced-annotation-kind", path))
                .selected_text(if annotation_kind.is_empty() {
                    "Select annotation kind…"
                } else {
                    annotation_kind.as_str()
                })
                .show_ui(ui, |ui| {
                    for candidate in &membership_options.annotation_kinds {
                        changed |= ui
                            .selectable_value(annotation_kind, candidate.clone(), candidate)
                            .changed();
                    }
                });
            changed |= ui
                .add(
                    egui::TextEdit::singleline(annotation_kind)
                        .hint_text("note, watched, rating, ..."),
                )
                .changed();
        }
        QueryPredicate::CanonicalEntityMembership { entity_id } => {
            ui.small(
                "Matches participants resolved to a durable canonical entity; the saved view stores the entity UUID, so renames remain stable.",
            );
            let selected = membership_options
                .canonical_entities
                .iter()
                .find(|entity| entity.id == *entity_id)
                .map(|entity| {
                    entity.entity_type.as_deref().map_or_else(
                        || entity.canonical_name.clone(),
                        |kind| format!("{} · {kind}", entity.canonical_name),
                    )
                })
                .unwrap_or_else(|| {
                    if entity_id.is_nil() {
                        "Select canonical entity…".to_string()
                    } else {
                        entity_id.to_string()
                    }
                });
            egui::ComboBox::from_id_salt(("advanced-canonical-entity", path))
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for entity in &membership_options.canonical_entities {
                        let label = entity.entity_type.as_deref().map_or_else(
                            || entity.canonical_name.clone(),
                            |kind| format!("{} · {kind}", entity.canonical_name),
                        );
                        changed |= ui.selectable_value(entity_id, entity.id, label).changed();
                    }
                });
        }
        QueryPredicate::CanonicalEntityType { entity_type } => {
            ui.small(
                "Matches events whose participants resolve to a canonical entity of this type.",
            );
            egui::ComboBox::from_id_salt(("advanced-canonical-entity-type", path))
                .selected_text(if entity_type.is_empty() {
                    "Select entity type…"
                } else {
                    entity_type.as_str()
                })
                .show_ui(ui, |ui| {
                    for candidate in &membership_options.canonical_entity_types {
                        changed |= ui
                            .selectable_value(entity_type, candidate.clone(), candidate)
                            .changed();
                    }
                });
            changed |= ui
                .add(
                    egui::TextEdit::singleline(entity_type)
                        .hint_text("person, organization, team, ..."),
                )
                .changed();
        }
        QueryPredicate::ProvenanceRoleAnyOf { values } => {
            ui.small(
                "Matches structured provenance records stored outside canonical event fields.",
            );
            ui.horizontal_wrapped(|ui| {
                for role in EventProvenanceRole::ALL {
                    let mut selected = values.contains(&role);
                    if ui.checkbox(&mut selected, role.as_str()).changed() {
                        if selected {
                            values.push(role);
                        } else {
                            values.retain(|value| *value != role);
                        }
                        changed = true;
                    }
                }
            });
        }
        QueryPredicate::ProvenanceReference { reference } => {
            ui.small("Matches an exact structured provenance reference.");
            changed |= ui
                .add(
                    egui::TextEdit::singleline(reference)
                        .hint_text("https://…, archive:123, assertion:…"),
                )
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
    occurrence_contexts: &'a HashMap<Uuid, OccurrenceContext>,
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
        CalendarLayout::Density => render_density(ui, events, view, focus, timezone, monday_start),
        CalendarLayout::Summary => render_summary(ui, events, timezone, group_by),
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct SummaryRow {
    label: String,
    count: usize,
    first_date: Option<NaiveDate>,
    last_date: Option<NaiveDate>,
}

fn render_summary(
    ui: &mut egui::Ui,
    events: &[TemporalEvent],
    timezone: Tz,
    group_by: GroupBy,
) -> Option<CalendarAction> {
    let rows = summary_rows(events, timezone, group_by);
    let total = events.len();

    ui.horizontal_wrapped(|ui| {
        ui.strong("Summary");
        ui.small(format!("{total} visible event(s)"));
        if group_by == GroupBy::None {
            ui.small("· no pivot grouping");
        } else {
            ui.small(format!("· pivot: {}", group_by.label()));
        }
    });
    ui.separator();

    egui::Grid::new("summary-grid")
        .striped(true)
        .spacing([16.0, 5.0])
        .show(ui, |ui| {
            ui.strong(if group_by == GroupBy::None {
                "Scope"
            } else {
                group_by.label()
            });
            ui.strong("Events");
            ui.strong("Share");
            ui.strong("First");
            ui.strong("Last");
            ui.end_row();

            for row in &rows {
                ui.label(&row.label);
                ui.monospace(row.count.to_string());
                let share = if total == 0 {
                    0.0
                } else {
                    row.count as f64 / total as f64 * 100.0
                };
                ui.monospace(format!("{share:.1}%"));
                ui.monospace(
                    row.first_date
                        .map(|date| date.to_string())
                        .unwrap_or_else(|| "—".to_string()),
                );
                ui.monospace(
                    row.last_date
                        .map(|date| date.to_string())
                        .unwrap_or_else(|| "—".to_string()),
                );
                ui.end_row();
            }
        });

    ui.add_space(8.0);
    ui.strong("Precision breakdown");
    let mut precision_counts = BTreeMap::<&'static str, usize>::new();
    for event in events {
        *precision_counts.entry(event.time.kind_name()).or_default() += 1;
    }
    ui.horizontal_wrapped(|ui| {
        for (kind, count) in precision_counts {
            ui.small(format!("{kind}: {count}"));
        }
    });

    None
}

fn summary_rows(events: &[TemporalEvent], timezone: Tz, group_by: GroupBy) -> Vec<SummaryRow> {
    if group_by == GroupBy::None {
        let dates = events
            .iter()
            .filter_map(|event| agenda_sort_date(event, timezone))
            .collect::<Vec<_>>();
        return vec![SummaryRow {
            label: "All visible events".to_string(),
            count: events.len(),
            first_date: dates.iter().min().copied(),
            last_date: dates.iter().max().copied(),
        }];
    }

    let mut buckets = BTreeMap::<String, Vec<&TemporalEvent>>::new();
    for event in events {
        buckets
            .entry(agenda_group_label(event, timezone, group_by))
            .or_default()
            .push(event);
    }

    buckets
        .into_iter()
        .map(|(label, grouped)| {
            let dates = grouped
                .iter()
                .filter_map(|event| agenda_sort_date(event, timezone))
                .collect::<Vec<_>>();
            SummaryRow {
                label,
                count: grouped.len(),
                first_date: dates.iter().min().copied(),
                last_date: dates.iter().max().copied(),
            }
        })
        .collect()
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
                    egui::Button::new(text).fill(fill).selected(false),
                );
                if response
                    .on_hover_text(format!("{} · {} event(s)", day, count))
                    .clicked()
                {
                    action = Some(CalendarAction::OpenDay(*day));
                }

                column += 1;
                if column.is_multiple_of(7) {
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
    let canonical_id = colors
        .occurrence_contexts
        .get(&event.id)
        .map_or(event.id, |occurrence| occurrence.event_id);
    let membership = colors.memberships.get(&canonical_id);

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

fn occurrence_time_label(time: &TimeSpec, timezone: Tz) -> String {
    let date = time
        .display_date(timezone)
        .map_or_else(|| "unplaced".to_string(), |date| date.to_string());
    format!("{date} · {}", time.display_time_label(timezone))
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

fn temporal_uncertainty_labels(uncertainty: &TimeUncertainty, timezone: Tz) -> (String, String) {
    match uncertainty {
        TimeUncertainty::DateWindow { earliest, latest } => {
            (earliest.to_string(), latest.to_string())
        }
        TimeUncertainty::FloatingWindow { earliest, latest } => (
            earliest.format("%Y-%m-%d %H:%M:%S").to_string(),
            latest.format("%Y-%m-%d %H:%M:%S").to_string(),
        ),
        TimeUncertainty::InstantWindow {
            earliest_utc,
            latest_utc,
        } => (
            earliest_utc
                .with_timezone(&timezone)
                .format("%Y-%m-%d %H:%M:%S %Z")
                .to_string(),
            latest_utc
                .with_timezone(&timezone)
                .format("%Y-%m-%d %H:%M:%S %Z")
                .to_string(),
        ),
    }
}

fn event_revision_change_summary(
    previous: Option<&TemporalEvent>,
    current: &TemporalEvent,
) -> String {
    let Some(previous) = previous else {
        return "created".to_string();
    };

    let mut changes = Vec::new();
    if previous.normalized_title != current.normalized_title {
        changes.push("title");
    }
    if previous.time != current.time || previous.time_uncertainty != current.time_uncertainty {
        changes.push("time");
    }
    if previous.recurrence != current.recurrence {
        changes.push("recurrence");
    }
    if previous.status != current.status {
        changes.push("status");
    }
    if previous.description != current.description {
        changes.push("description");
    }
    if previous.event_type != current.event_type
        || previous.domain != current.domain
        || previous.jurisdiction != current.jurisdiction
        || previous.institution != current.institution
    {
        changes.push("classification");
    }
    if previous.location != current.location {
        changes.push("location");
    }
    if previous.confidence != current.confidence
        || previous.importance != current.importance
        || previous.personal_relevance != current.personal_relevance
    {
        changes.push("scoring");
    }
    if previous.source_id != current.source_id
        || previous.source_record_key != current.source_record_key
        || previous.upstream_event_ref != current.upstream_event_ref
        || previous.upstream_reconciled_key != current.upstream_reconciled_key
        || previous.assertion_refs != current.assertion_refs
        || previous.source_refs != current.source_refs
        || previous.provenance_refs != current.provenance_refs
    {
        changes.push("identity/provenance");
    }
    if previous.tags != current.tags {
        changes.push("tags");
    }
    if previous.properties != current.properties {
        changes.push("properties");
    }

    if changes.is_empty() {
        "canonical metadata changed".to_string()
    } else {
        changes.join(", ")
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

    #[test]
    fn uncertain_time_editor_moves_entire_window_and_preserves_original_draft() {
        let anchor = Utc
            .with_ymd_and_hms(2026, 10, 8, 15, 0, 0)
            .single()
            .expect("start");
        let mut event = TemporalEvent::new(
            "Possible appointment",
            TimeSpec::Instant {
                start_utc: anchor,
                end_utc: Some(anchor + ChronoDuration::hours(1)),
                source_timezone: Some("UTC".to_string()),
            },
        );
        let original_window = TimeUncertainty::InstantWindow {
            earliest_utc: anchor - ChronoDuration::hours(2),
            latest_utc: anchor + ChronoDuration::hours(1),
        };
        event.time_uncertainty = Some(original_window.clone());
        let mut draft =
            EventTimeEditDraft::from_event(&event, chrono_tz::UTC).expect("uncertain time draft");
        assert_eq!(
            draft.uncertain_origin,
            Some((event.time.clone(), original_window))
        );
        draft.date = "2026-10-10".to_string();
        let proposed = draft.parsed_time().expect("proposed time");
        let moved = move_uncertain_placement(&event, &proposed).expect("window move");
        assert_eq!(
            moved.time_uncertainty,
            Some(TimeUncertainty::InstantWindow {
                earliest_utc: anchor + ChronoDuration::days(2) - ChronoDuration::hours(2),
                latest_utc: anchor + ChronoDuration::days(2) + ChronoDuration::hours(1),
            })
        );
        assert_ne!(event.time, moved.time);
        assert!(draft.conflict_confirmation.is_none());
        let confirmation =
            ConflictConfirmation::for_event(&moved, "Provisional uncertain placement".to_string());
        assert!(confirmation.matches(&moved));
        draft.date = "2026-10-11".to_string();
        let changed = move_uncertain_placement(&event, &draft.parsed_time().expect("new proposal"))
            .expect("changed window");
        assert!(!confirmation.matches(&changed));
    }

    #[test]
    fn uncertain_all_day_time_editor_derives_multiday_exclusive_end() {
        let original_start = NaiveDate::from_ymd_opt(2026, 10, 30).expect("date");
        let mut event = TemporalEvent::new(
            "Possible retreat",
            TimeSpec::AllDay {
                start: original_start,
                end_exclusive: Some(original_start + chrono::Days::new(2)),
            },
        );
        event.time_uncertainty = Some(TimeUncertainty::DateWindow {
            earliest: original_start - chrono::Days::new(1),
            latest: original_start + chrono::Days::new(1),
        });
        let mut draft = EventTimeEditDraft::from_event(&event, chrono_tz::America::New_York)
            .expect("bounded civil edit");
        draft.date = "2026-11-01".to_string();
        let proposed = draft.parsed_time().expect("derived civil span");
        let expected_start = NaiveDate::from_ymd_opt(2026, 11, 1).expect("new date");
        assert_eq!(
            proposed,
            TimeSpec::AllDay {
                start: expected_start,
                end_exclusive: Some(expected_start + chrono::Days::new(2)),
            }
        );
        let moved = move_uncertain_placement(&event, &proposed).expect("window shift");
        assert_eq!(
            moved.time_uncertainty,
            Some(TimeUncertainty::DateWindow {
                earliest: expected_start - chrono::Days::new(1),
                latest: expected_start + chrono::Days::new(1),
            })
        );
        assert_eq!(event.time, draft.uncertain_origin.expect("original").0);
    }

    #[test]
    fn uncertain_time_editor_refuses_subminute_loss_and_recurring_master() {
        let anchor = Utc
            .with_ymd_and_hms(2026, 10, 8, 15, 0, 30)
            .single()
            .expect("subminute start");
        let mut event = TemporalEvent::new(
            "Precise uncertain appointment",
            TimeSpec::Instant {
                start_utc: anchor,
                end_utc: Some(anchor + ChronoDuration::hours(1)),
                source_timezone: Some("UTC".to_string()),
            },
        );
        event.time_uncertainty = Some(TimeUncertainty::InstantWindow {
            earliest_utc: anchor - ChronoDuration::minutes(30),
            latest_utc: anchor + ChronoDuration::minutes(30),
        });
        assert!(EventTimeEditDraft::from_event(&event, chrono_tz::UTC).is_err());
        event.time = TimeSpec::Instant {
            start_utc: anchor - ChronoDuration::seconds(30),
            end_utc: Some(anchor + ChronoDuration::hours(1) - ChronoDuration::seconds(30)),
            source_timezone: Some("UTC".to_string()),
        };
        assert!(EventTimeEditDraft::from_event(&event, chrono_tz::UTC).is_ok());
        event.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Daily));
        assert!(EventTimeEditDraft::from_event(&event, chrono_tz::UTC).is_err());
    }

    #[test]
    fn focused_recurrence_conflict_confirmation_keeps_sister_slots_busy() {
        let store = TemporalStore::open_in_memory().expect("store");
        let start = Utc
            .with_ymd_and_hms(2026, 10, 8, 9, 0, 0)
            .single()
            .expect("start");
        let first = TimeSpec::Instant {
            start_utc: start,
            end_utc: Some(start + ChronoDuration::hours(1)),
            source_timezone: None,
        };
        let mut event = TemporalEvent::new("Daily series", first.clone());
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.count = Some(2);
        event.recurrence = Some(rule.clone());
        store.upsert_event(&event).expect("store series");
        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.focus_occurrence(&first).expect("focus");

        let next_day = TimeSpec::Instant {
            start_utc: start + ChronoDuration::days(1),
            end_utc: Some(start + ChronoDuration::days(1) + ChronoDuration::hours(1)),
            source_timezone: None,
        };
        let mut proposed = rule.clone();
        proposed.overrides.push(RecurrenceOverride {
            original: first.clone(),
            replacement: Some(next_day),
            cancelled: false,
        });
        let warning =
            focused_recurrence_conflict_warning(&store, chrono_tz::UTC, &event, &draft, &proposed)
                .expect("detect sibling conflict")
                .expect("warning");
        assert!(warning.contains("Daily series"));
        assert!(warning.contains("Save again"));
        assert_eq!(
            store
                .event_by_id(event.id)
                .expect("stored event")
                .expect("event")
                .recurrence,
            Some(rule.clone())
        );

        let third_day = TimeSpec::Instant {
            start_utc: start + ChronoDuration::days(2),
            end_utc: Some(start + ChronoDuration::days(2) + ChronoDuration::hours(1)),
            source_timezone: None,
        };
        proposed.overrides[0].replacement = Some(third_day);
        assert!(
            focused_recurrence_conflict_warning(&store, chrono_tz::UTC, &event, &draft, &proposed,)
                .expect("free replacement")
                .is_none()
        );
        proposed.count = Some(5);
        assert!(
            focused_recurrence_conflict_warning(&store, chrono_tz::UTC, &event, &draft, &proposed,)
                .expect_err("focused edit must not mutate unrelated cadence")
                .to_string()
                .contains("cannot change other recurrence")
        );
    }

    #[test]
    fn focused_recurrence_confirmation_reports_uncheckable_uncertain_blockers() {
        let store = TemporalStore::open_in_memory().expect("store");
        let start = Utc
            .with_ymd_and_hms(2026, 10, 8, 9, 0, 0)
            .single()
            .expect("start");
        let first = TimeSpec::Instant {
            start_utc: start,
            end_utc: Some(start + ChronoDuration::hours(1)),
            source_timezone: None,
        };
        let mut series = TemporalEvent::new("Daily series", first.clone());
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.count = Some(2);
        series.recurrence = Some(rule.clone());
        store.upsert_event(&series).expect("series");

        let proposed_start = start + ChronoDuration::days(2);
        let mut uncertain = TemporalEvent::new(
            "Uncertain appointment",
            TimeSpec::Instant {
                start_utc: proposed_start,
                end_utc: Some(proposed_start + ChronoDuration::hours(1)),
                source_timezone: None,
            },
        );
        uncertain.time_uncertainty = Some(TimeUncertainty::InstantWindow {
            earliest_utc: proposed_start - ChronoDuration::hours(1),
            latest_utc: proposed_start + ChronoDuration::hours(1),
        });
        store.upsert_event(&uncertain).expect("uncertain blocker");

        let mut draft = RecurrenceEditDraft::from_event(&series);
        draft.focus_occurrence(&first).expect("focus");
        rule.overrides.push(RecurrenceOverride {
            original: first,
            replacement: Some(TimeSpec::Instant {
                start_utc: proposed_start,
                end_utc: Some(proposed_start + ChronoDuration::hours(1)),
                source_timezone: None,
            }),
            cancelled: false,
        });
        let warning =
            focused_recurrence_conflict_warning(&store, chrono_tz::UTC, &series, &draft, &rule)
                .expect("advisory check")
                .expect("provisional warning");
        assert!(warning.contains("provisional"));
        assert!(warning.contains("could not be checked"));
        assert!(warning.contains("Save again"));
    }

    #[test]
    fn focused_cancelled_occurrence_requires_confirmation_when_restored_into_busy_slot() {
        let store = TemporalStore::open_in_memory().expect("store");
        let start = Utc
            .with_ymd_and_hms(2026, 10, 8, 9, 0, 0)
            .single()
            .expect("start");
        let original = TimeSpec::Instant {
            start_utc: start,
            end_utc: Some(start + ChronoDuration::hours(1)),
            source_timezone: None,
        };
        let second = TimeSpec::Instant {
            start_utc: start + ChronoDuration::days(1),
            end_utc: Some(start + ChronoDuration::days(1) + ChronoDuration::hours(1)),
            source_timezone: None,
        };
        let mut event = TemporalEvent::new("Series", original.clone());
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.count = Some(2);
        rule.overrides.push(RecurrenceOverride {
            original: original.clone(),
            replacement: None,
            cancelled: true,
        });
        event.recurrence = Some(rule.clone());
        store
            .upsert_event(&event)
            .expect("persist cancelled series");
        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.focus_occurrence(&original).expect("focus");

        let mut restored = rule.clone();
        restored.overrides[0] = RecurrenceOverride {
            original: original.clone(),
            replacement: Some(second),
            cancelled: false,
        };
        let warning =
            focused_recurrence_conflict_warning(&store, chrono_tz::UTC, &event, &draft, &restored)
                .expect("restoration conflict")
                .expect("warning");
        assert!(warning.contains("Series"));
        assert!(warning.contains("Save again"));

        restored.overrides.clear();
        assert!(
            focused_recurrence_conflict_warning(&store, chrono_tz::UTC, &event, &draft, &restored)
                .expect("restoring original slot")
                .is_none()
        );
    }

    #[test]
    fn focused_recurrence_cancellation_does_not_create_a_new_busy_commitment() {
        let store = TemporalStore::open_in_memory().expect("store");
        let start = Utc
            .with_ymd_and_hms(2026, 10, 8, 9, 0, 0)
            .single()
            .expect("start");
        let original = TimeSpec::Instant {
            start_utc: start,
            end_utc: Some(start + ChronoDuration::hours(1)),
            source_timezone: None,
        };
        let mut event = TemporalEvent::new("Series", original.clone());
        let rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        event.recurrence = Some(rule.clone());
        store.upsert_event(&event).expect("store series");
        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.focus_occurrence(&original).expect("focus");
        let mut cancelled = rule;
        cancelled.overrides.push(RecurrenceOverride {
            original: original.clone(),
            replacement: None,
            cancelled: true,
        });
        assert!(
            focused_recurrence_conflict_warning(&store, chrono_tz::UTC, &event, &draft, &cancelled)
                .expect("cancellation")
                .is_none()
        );
    }

    #[test]
    fn recurring_alternative_applies_only_a_replacement_for_the_original_slot() {
        let start = Utc
            .with_ymd_and_hms(2026, 10, 8, 9, 0, 0)
            .single()
            .expect("start");
        let base_time = TimeSpec::Instant {
            start_utc: start,
            end_utc: Some(start + ChronoDuration::hours(1)),
            source_timezone: Some("America/Mexico_City".to_string()),
        };
        let mut event = TemporalEvent::new("Daily", base_time.clone());
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.count = Some(3);
        event.recurrence = Some(rule.clone());
        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.focus_occurrence(&base_time).expect("focus");
        let slot = FreeInterval {
            start_utc: start + ChronoDuration::hours(2),
            end_utc: start + ChronoDuration::hours(3),
        };
        draft.alternative_rule = Some(rule.clone());
        draft.alternative_slots = vec![slot.clone()];
        apply_recurring_alternative_to_draft(&mut draft, &slot, chrono_tz::UTC).expect("apply");
        let changed = draft.parsed_rule().expect("rule");
        assert_eq!(changed.overrides.len(), 1);
        assert_eq!(changed.overrides[0].original, base_time);
        assert_eq!(
            changed.overrides[0]
                .replacement
                .as_ref()
                .expect("replacement"),
            &TimeSpec::Instant {
                start_utc: slot.start_utc,
                end_utc: Some(slot.end_utc),
                source_timezone: Some("America/Mexico_City".to_string()),
            }
        );
        assert!(!changed.overrides[0].cancelled);
        assert_eq!(draft.rule, rule);
        assert!(draft.alternative_slots.is_empty());
    }

    #[test]
    fn recurring_alternative_rejects_stale_drafts_and_different_durations() {
        let start = Utc
            .with_ymd_and_hms(2026, 10, 8, 9, 0, 0)
            .single()
            .expect("start");
        let base_time = TimeSpec::Instant {
            start_utc: start,
            end_utc: Some(start + ChronoDuration::hours(1)),
            source_timezone: None,
        };
        let mut event = TemporalEvent::new("Daily", base_time.clone());
        event.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Daily));
        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.focus_occurrence(&base_time).expect("focus");
        let wrong_duration = FreeInterval {
            start_utc: start + ChronoDuration::hours(2),
            end_utc: start + ChronoDuration::hours(4),
        };
        draft.alternative_rule = Some(draft.parsed_rule().expect("rule"));
        draft.alternative_slots = vec![wrong_duration.clone()];
        assert!(
            apply_recurring_alternative_to_draft(&mut draft, &wrong_duration, chrono_tz::UTC)
                .is_err()
        );
        assert!(draft.override_rows.is_empty());

        draft.rule.count = Some(5);
        assert!(
            apply_recurring_alternative_to_draft(&mut draft, &wrong_duration, chrono_tz::UTC)
                .is_err()
        );
        assert!(draft.override_rows.is_empty());
    }

    #[test]
    fn cancelled_occurrence_suggestion_drafts_reactivation_without_changing_sisters() {
        let start = Utc
            .with_ymd_and_hms(2026, 10, 8, 9, 0, 0)
            .single()
            .expect("start");
        let original = TimeSpec::Instant {
            start_utc: start,
            end_utc: Some(start + ChronoDuration::hours(1)),
            source_timezone: None,
        };
        let mut event = TemporalEvent::new("Daily", original.clone());
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.count = Some(3);
        rule.overrides.push(RecurrenceOverride {
            original: original.clone(),
            replacement: None,
            cancelled: true,
        });
        event.recurrence = Some(rule.clone());
        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.focus_occurrence(&original).expect("focus");
        let slot = FreeInterval {
            start_utc: start + ChronoDuration::days(3),
            end_utc: start + ChronoDuration::days(3) + ChronoDuration::hours(1),
        };
        draft.alternative_rule = Some(rule.clone());
        draft.alternative_slots = vec![slot.clone()];
        apply_recurring_alternative_to_draft(&mut draft, &slot, chrono_tz::UTC)
            .expect("draft reactivation");
        let proposed = draft.parsed_rule().expect("rule");
        assert_eq!(proposed.count, rule.count);
        assert_eq!(proposed.overrides.len(), 1);
        assert_eq!(proposed.overrides[0].original, original);
        assert!(!proposed.overrides[0].cancelled);
        assert_eq!(
            proposed.overrides[0].replacement,
            Some(TimeSpec::Instant {
                start_utc: slot.start_utc,
                end_utc: Some(slot.end_utc),
                source_timezone: None,
            })
        );
        assert_eq!(draft.rule, rule);
    }

    #[test]
    fn recurring_all_day_alternative_preserves_civil_span_across_dst() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 30).expect("date");
        let base = TimeSpec::AllDay {
            start,
            end_exclusive: Some(start + chrono::Days::new(2)),
        };
        let mut event = TemporalEvent::new("Retreat", base.clone());
        event.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Weekly));
        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.focus_occurrence(&base).expect("focus");
        let timezone = chrono_tz::America::New_York;
        let suggested_date = NaiveDate::from_ymd_opt(2026, 11, 1).expect("dst date");
        let suggested_end = suggested_date + chrono::Days::new(2);
        let slot = FreeInterval {
            start_utc: timezone
                .from_local_datetime(&suggested_date.and_hms_opt(0, 0, 0).expect("midnight"))
                .single()
                .expect("unambiguous start")
                .with_timezone(&Utc),
            end_utc: timezone
                .from_local_datetime(&suggested_end.and_hms_opt(0, 0, 0).expect("midnight"))
                .single()
                .expect("unambiguous end")
                .with_timezone(&Utc),
        };
        draft.alternative_rule = Some(draft.parsed_rule().expect("rule"));
        draft.alternative_slots = vec![slot.clone()];
        apply_recurring_alternative_to_draft(&mut draft, &slot, timezone).expect("apply civil");
        let rule = draft.parsed_rule().expect("rule");
        assert_eq!(rule.overrides[0].original, base);
        assert_eq!(
            rule.overrides[0].replacement,
            Some(TimeSpec::AllDay {
                start: suggested_date,
                end_exclusive: Some(suggested_end),
            })
        );
        assert_eq!((slot.end_utc - slot.start_utc).num_hours(), 49);
    }

    #[test]
    fn newly_opened_drafts_have_distinct_async_result_identity() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 8).expect("date");
        let first = NewLocalEventDraft::for_date(date);
        let second = NewLocalEventDraft::for_date(date);
        assert_ne!(first.draft_token, second.draft_token);

        let start = Utc
            .with_ymd_and_hms(2026, 10, 8, 15, 0, 0)
            .single()
            .expect("start");
        let event = TemporalEvent::new(
            "Existing",
            TimeSpec::Instant {
                start_utc: start,
                end_utc: Some(start + ChronoDuration::hours(1)),
                source_timezone: None,
            },
        );
        let original =
            EventTimeEditDraft::from_event(&event, chrono_tz::UTC).expect("first editor");
        let reopened =
            EventTimeEditDraft::from_event(&event, chrono_tz::UTC).expect("reopened editor");
        assert_eq!(original.event_id, reopened.event_id);
        assert_ne!(original.draft_token, reopened.draft_token);
        assert_eq!(
            original.parsed_time().expect("time"),
            reopened.parsed_time().expect("time")
        );
    }

    #[test]
    fn background_suggestions_ignore_stale_scheduling_drafts() {
        let timezone = chrono_tz::UTC;
        let day = NaiveDate::from_ymd_opt(2026, 10, 8).expect("date");
        let mut draft = NewLocalEventDraft::for_date(day);
        draft.title = "New".to_string();
        let (time, _) = parse_new_local_event_time(&draft, timezone).expect("time");
        let candidate = TemporalEvent::new("New", time);
        draft.conflict_confirmation = Some(ConflictConfirmation::for_event(
            &candidate,
            "collision".to_string(),
        ));

        assert!(new_conflict_confirmation_is_current(
            &draft, &candidate, timezone
        ));
        draft.start_time = "10:00".to_string();
        assert!(!new_conflict_confirmation_is_current(
            &draft, &candidate, timezone
        ));

        let timed = TemporalEvent::new(
            "Existing",
            TimeSpec::Instant {
                start_utc: Utc
                    .with_ymd_and_hms(2026, 10, 8, 15, 0, 0)
                    .single()
                    .expect("start"),
                end_utc: Some(
                    Utc.with_ymd_and_hms(2026, 10, 8, 16, 0, 0)
                        .single()
                        .expect("end"),
                ),
                source_timezone: None,
            },
        );
        let mut edit = EventTimeEditDraft::from_event(&timed, timezone).expect("edit");
        edit.conflict_confirmation = Some(ConflictConfirmation::for_event(
            &timed,
            "collision".to_string(),
        ));
        assert!(time_conflict_confirmation_is_current(
            &edit, &timed, timed.id
        ));
        edit.duration_minutes = "30".to_string();
        assert!(!time_conflict_confirmation_is_current(
            &edit, &timed, timed.id
        ));
    }

    #[test]
    fn new_local_event_authoring_preserves_participants_and_location() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 7).expect("date");
        let mut draft = NewLocalEventDraft::for_date(day);
        draft.title = "Project meeting".to_string();
        draft.participant_names = "  Alex  \n\nBea\n  Cai  ".to_string();
        draft.location_name = "Library".to_string();
        draft.location_address = "Main Street".to_string();
        draft.location_virtual_url = "https://example.com/meeting".to_string();
        let (event, _) = new_local_event_from_draft(&draft, chrono_tz::UTC).expect("event");
        assert_eq!(
            event
                .participants
                .iter()
                .map(|participant| participant.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Alex", "Bea", "Cai"]
        );
        assert!(
            event
                .participants
                .iter()
                .all(|participant| participant.role.is_none())
        );
        let location = event.location.expect("location");
        assert_eq!(location.name.as_deref(), Some("Library"));
        assert_eq!(location.address.as_deref(), Some("Main Street"));
        assert_eq!(
            location.virtual_url.as_deref(),
            Some("https://example.com/meeting")
        );
    }

    #[test]
    fn new_local_event_authoring_omits_blank_optional_structures() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 7).expect("date");
        let mut draft = NewLocalEventDraft::for_date(day);
        draft.title = "Simple".to_string();
        draft.participant_names = " \n\n ".to_string();
        draft.location_name = " ".to_string();
        let (event, _) = new_local_event_from_draft(&draft, chrono_tz::UTC).expect("event");
        assert!(event.participants.is_empty());
        assert!(event.location.is_none());
    }

    #[test]
    fn conflict_alternative_keeps_quick_create_metadata_and_requires_new_confirmation() {
        let timezone = chrono_tz::UTC;
        let day = NaiveDate::from_ymd_opt(2026, 10, 8).expect("date");
        let mut draft = NewLocalEventDraft::for_date(day);
        draft.title = "Keep my title".to_string();
        draft.description = "Keep my notes".to_string();
        draft.participant_names = "Alex\nBea".to_string();
        draft.location_name = "Meeting room".to_string();
        draft.location_virtual_url = "https://example.com/meet".to_string();
        let original = TemporalEvent::new(
            "Original",
            TimeSpec::AllDay {
                start: day,
                end_exclusive: None,
            },
        );
        draft.conflict_confirmation = Some(ConflictConfirmation::for_event(
            &original,
            "collision".to_string(),
        ));
        let start = timezone
            .with_ymd_and_hms(2026, 10, 8, 11, 30, 0)
            .single()
            .expect("start")
            .with_timezone(&Utc);
        let slot = FreeInterval {
            start_utc: start,
            end_utc: start + ChronoDuration::minutes(45),
        };

        apply_alternative_to_new_draft(&mut draft, &slot, timezone).expect("apply alternative");
        assert_eq!(draft.title, "Keep my title");
        assert_eq!(draft.description, "Keep my notes");
        assert_eq!(draft.participant_names, "Alex\nBea");
        assert_eq!(draft.location_name, "Meeting room");
        assert_eq!(draft.location_virtual_url, "https://example.com/meet");
        let (candidate, _) = new_local_event_from_draft(&draft, timezone).expect("candidate");
        assert_eq!(candidate.participants.len(), 2);
        assert_eq!(
            candidate
                .location
                .as_ref()
                .and_then(|location| location.name.as_deref()),
            Some("Meeting room")
        );
        assert_eq!(draft.start_time, "11:30");
        assert_eq!(draft.duration_minutes, "45");
        assert!(draft.conflict_confirmation.is_none());
        let (time, _) = parse_new_local_event_time(&draft, timezone).expect("parsed");
        assert!(matches!(
            time,
            TimeSpec::Instant { start_utc, end_utc: Some(end_utc), .. }
                if start_utc == slot.start_utc && end_utc == slot.end_utc
        ));
    }

    #[test]
    fn all_day_quick_create_alternative_preserves_kind_and_metadata() {
        let timezone = chrono_tz::America::New_York;
        let original_date = NaiveDate::from_ymd_opt(2026, 10, 30).expect("date");
        let target_date = NaiveDate::from_ymd_opt(2026, 11, 2).expect("target date");
        let mut draft = NewLocalEventDraft::for_date(original_date);
        draft.all_day = true;
        draft.title = "Day of rest".to_string();
        draft.description = "Preserve notes".to_string();
        let candidate = TemporalEvent::new(
            draft.title.clone(),
            TimeSpec::AllDay {
                start: original_date,
                end_exclusive: None,
            },
        );
        draft.conflict_confirmation = Some(ConflictConfirmation::for_event(
            &candidate,
            "collision".to_string(),
        ));

        let start_utc = timezone
            .from_local_datetime(&target_date.and_hms_opt(0, 0, 0).expect("midnight"))
            .single()
            .expect("local start")
            .with_timezone(&Utc);
        let end_utc = timezone
            .from_local_datetime(
                &target_date
                    .succ_opt()
                    .expect("next day")
                    .and_hms_opt(0, 0, 0)
                    .expect("midnight"),
            )
            .single()
            .expect("local end")
            .with_timezone(&Utc);
        let slot = FreeInterval { start_utc, end_utc };

        apply_alternative_to_new_draft(&mut draft, &slot, timezone)
            .expect("apply civil alternative");
        assert!(draft.all_day);
        assert_eq!(draft.title, "Day of rest");
        assert_eq!(draft.description, "Preserve notes");
        assert_eq!(draft.date, "2026-11-02");
        assert!(draft.conflict_confirmation.is_none());
        assert_eq!(
            parse_new_local_event_time(&draft, timezone)
                .expect("parse")
                .0,
            TimeSpec::AllDay {
                start: target_date,
                end_exclusive: None,
            }
        );
    }

    #[test]
    fn multi_day_quick_create_alternative_keeps_exclusive_end_and_metadata() {
        let timezone = chrono_tz::America::New_York;
        let original_date = NaiveDate::from_ymd_opt(2026, 10, 30).expect("start");
        let target_date = NaiveDate::from_ymd_opt(2026, 11, 4).expect("target");
        let target_end = target_date
            .checked_add_days(chrono::Days::new(3))
            .expect("target end");
        let mut draft = NewLocalEventDraft::for_date(original_date);
        draft.all_day = true;
        draft.end_date = original_date
            .checked_add_days(chrono::Days::new(3))
            .expect("original end")
            .to_string();
        draft.title = "Three-day workshop".to_string();
        draft.description = "Carry original notes".to_string();
        let before = parse_new_local_event_time(&draft, timezone)
            .expect("original event")
            .0;
        assert_eq!(
            before,
            TimeSpec::AllDay {
                start: original_date,
                end_exclusive: original_date.checked_add_days(chrono::Days::new(3)),
            }
        );
        let boundary = |date: NaiveDate| {
            timezone
                .from_local_datetime(&date.and_hms_opt(0, 0, 0).expect("midnight"))
                .single()
                .expect("unique midnight")
                .with_timezone(&Utc)
        };
        let slot = FreeInterval {
            start_utc: boundary(target_date),
            end_utc: boundary(target_end),
        };
        apply_alternative_to_new_draft(&mut draft, &slot, timezone).expect("apply alternative");
        assert_eq!(draft.title, "Three-day workshop");
        assert_eq!(draft.description, "Carry original notes");
        assert!(draft.all_day);
        assert_eq!(draft.date, target_date.to_string());
        assert_eq!(draft.end_date, target_end.to_string());
        assert_eq!(
            parse_new_local_event_time(&draft, timezone)
                .expect("rescheduled")
                .0,
            TimeSpec::AllDay {
                start: target_date,
                end_exclusive: Some(target_end),
            }
        );
    }

    #[test]
    fn all_day_quick_create_requires_exclusive_end_after_start() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 7).expect("start");
        let mut draft = NewLocalEventDraft::for_date(start);
        draft.all_day = true;
        draft.end_date = start.to_string();
        assert!(parse_new_local_event_time(&draft, chrono_tz::UTC).is_err());
        draft.end_date = "invalid".to_string();
        assert!(parse_new_local_event_time(&draft, chrono_tz::UTC).is_err());
        draft.end_date = String::new();
        assert_eq!(
            parse_new_local_event_time(&draft, chrono_tz::UTC)
                .expect("one-day fallback")
                .0,
            TimeSpec::AllDay {
                start,
                end_exclusive: None,
            }
        );
    }

    #[test]
    fn all_day_time_edit_alternative_preserves_explicit_civil_day_span() {
        let timezone = chrono_tz::America::New_York;
        let original_start = NaiveDate::from_ymd_opt(2026, 10, 30).expect("start");
        let target_start = NaiveDate::from_ymd_opt(2026, 11, 5).expect("target");
        let target_end = target_start
            .checked_add_days(chrono::Days::new(3))
            .expect("end");
        let event = TemporalEvent::new(
            "Three-day workshop",
            TimeSpec::AllDay {
                start: original_start,
                end_exclusive: original_start.checked_add_days(chrono::Days::new(3)),
            },
        );
        let mut draft = EventTimeEditDraft::from_event(&event, timezone).expect("draft");
        draft.conflict_confirmation = Some(ConflictConfirmation::for_event(
            &event,
            "collision".to_string(),
        ));

        let boundary = |date: NaiveDate| {
            timezone
                .from_local_datetime(&date.and_hms_opt(0, 0, 0).expect("midnight"))
                .single()
                .expect("unique midnight")
                .with_timezone(&Utc)
        };
        let slot = FreeInterval {
            start_utc: boundary(target_start),
            end_utc: boundary(target_end),
        };
        apply_alternative_to_time_draft(&mut draft, &slot, timezone).expect("apply alternative");
        assert_eq!(draft.date, target_start.to_string());
        assert_eq!(draft.end_date, target_end.to_string());
        assert!(draft.conflict_confirmation.is_none());
        assert_eq!(
            draft.parsed_time().expect("parsed edit"),
            TimeSpec::AllDay {
                start: target_start,
                end_exclusive: Some(target_end),
            }
        );
    }

    #[test]
    fn all_day_alternative_rejects_non_midnight_without_mutating_draft() {
        let timezone = chrono_tz::UTC;
        let date = NaiveDate::from_ymd_opt(2026, 10, 7).expect("date");
        let mut draft = NewLocalEventDraft::for_date(date);
        draft.all_day = true;
        let before = draft.date.clone();
        let slot = FreeInterval {
            start_utc: Utc
                .with_ymd_and_hms(2026, 10, 8, 1, 0, 0)
                .single()
                .expect("start"),
            end_utc: Utc
                .with_ymd_and_hms(2026, 10, 9, 1, 0, 0)
                .single()
                .expect("end"),
        };
        assert!(apply_alternative_to_new_draft(&mut draft, &slot, timezone).is_err());
        assert_eq!(draft.date, before);
        assert!(draft.all_day);
    }

    #[test]
    fn conflict_alternative_preserves_source_clock_for_time_edits() {
        let start = Utc
            .with_ymd_and_hms(2026, 10, 8, 15, 0, 0)
            .single()
            .expect("start");
        let event = TemporalEvent::new(
            "Reschedule",
            TimeSpec::Floating {
                start: NaiveDate::from_ymd_opt(2026, 10, 8)
                    .expect("day")
                    .and_hms_opt(9, 0, 0)
                    .expect("time"),
                end: Some(
                    NaiveDate::from_ymd_opt(2026, 10, 8)
                        .expect("day")
                        .and_hms_opt(10, 0, 0)
                        .expect("time"),
                ),
                source_timezone: Some("America/Mexico_City".to_string()),
            },
        );
        let mut draft = EventTimeEditDraft::from_event(&event, chrono_tz::UTC).expect("edit draft");
        let slot = FreeInterval {
            start_utc: start + ChronoDuration::hours(2),
            end_utc: start + ChronoDuration::hours(3),
        };
        apply_alternative_to_time_draft(&mut draft, &slot, chrono_tz::UTC)
            .expect("apply alternative");
        assert_eq!(draft.start_time, "11:00");
        assert_eq!(draft.duration_minutes, "60");
        assert!(matches!(
            draft.parsed_time().expect("parsed"),
            TimeSpec::Floating {
                source_timezone: Some(value),
                ..
            } if value == "America/Mexico_City"
        ));
    }

    #[test]
    fn alternate_time_edit_rejects_ambiguous_dst_clock_without_mutation() {
        let timezone = chrono_tz::America::New_York;
        let start = Utc
            .with_ymd_and_hms(2026, 11, 1, 6, 30, 0)
            .single()
            .expect("second occurrence");
        let event = TemporalEvent::new(
            "DST ambiguity",
            TimeSpec::Instant {
                start_utc: start,
                end_utc: Some(start + ChronoDuration::minutes(30)),
                source_timezone: Some(timezone.name().to_string()),
            },
        );
        let mut draft = EventTimeEditDraft::from_event(&event, chrono_tz::UTC).expect("edit draft");
        let before = draft.date.clone();
        let slot = FreeInterval {
            start_utc: start,
            end_utc: start + ChronoDuration::minutes(30),
        };
        assert!(apply_alternative_to_time_draft(&mut draft, &slot, chrono_tz::UTC).is_err());
        assert_eq!(draft.date, before);
        assert_eq!(draft.start_time, "01:30");
    }

    #[test]
    fn custom_snooze_minutes_are_bounded_and_whitespace_tolerant() {
        assert_eq!(parse_snooze_minutes(" 15 ").expect("15 minutes"), 15);
        assert_eq!(parse_snooze_minutes("10080").expect("seven days"), 10_080);
        assert!(parse_snooze_minutes("0").is_err());
        assert!(parse_snooze_minutes("10081").is_err());
        assert!(parse_snooze_minutes("1.5").is_err());
        assert!(parse_snooze_minutes("").is_err());
    }

    #[test]
    fn new_local_event_time_preserves_all_day_and_timezone_semantics() {
        let mut all_day =
            NewLocalEventDraft::for_date(NaiveDate::from_ymd_opt(2026, 10, 7).expect("date"));
        all_day.all_day = true;
        let (time, date) =
            parse_new_local_event_time(&all_day, chrono_tz::America::Mexico_City).expect("all day");
        assert_eq!(date, NaiveDate::from_ymd_opt(2026, 10, 7).expect("date"));
        assert!(matches!(time, TimeSpec::AllDay { .. }));

        let timed =
            NewLocalEventDraft::for_date(NaiveDate::from_ymd_opt(2026, 10, 7).expect("date"));
        let (time, _) = parse_new_local_event_time(&timed, chrono_tz::America::Mexico_City)
            .expect("timed event");
        assert!(matches!(
            time,
            TimeSpec::Instant {
                source_timezone: Some(ref timezone),
                ..
            } if timezone == "America/Mexico_City"
        ));
    }

    #[test]
    fn new_local_event_time_rejects_invalid_duration_and_ambiguous_wall_time() {
        let mut invalid =
            NewLocalEventDraft::for_date(NaiveDate::from_ymd_opt(2026, 10, 7).expect("date"));
        invalid.duration_minutes = "0".to_string();
        assert!(parse_new_local_event_time(&invalid, chrono_tz::America::Mexico_City).is_err());

        let mut ambiguous =
            NewLocalEventDraft::for_date(NaiveDate::from_ymd_opt(2026, 11, 1).expect("date"));
        ambiguous.start_time = "01:30".to_string();
        assert!(parse_new_local_event_time(&ambiguous, chrono_tz::America::New_York).is_err());
    }

    #[test]
    fn event_time_edit_roundtrips_exact_clock_context_and_optional_end() {
        let timezone = chrono_tz::America::Mexico_City;
        let local = timezone
            .with_ymd_and_hms(2026, 10, 7, 9, 30, 0)
            .single()
            .expect("local time");
        let start_utc = local.with_timezone(&Utc);
        let mut event = TemporalEvent::new(
            "Editable time",
            TimeSpec::Instant {
                start_utc,
                end_utc: Some(start_utc + ChronoDuration::minutes(90)),
                source_timezone: Some("America/Mexico_City".to_string()),
            },
        );

        let mut draft = EventTimeEditDraft::from_event(&event, chrono_tz::UTC).expect("time draft");
        assert_eq!(draft.date, "2026-10-07");
        assert_eq!(draft.start_time, "09:30");
        assert_eq!(draft.duration_minutes, "90");
        assert_eq!(draft.timezone_label(), Some("America/Mexico_City"));

        draft.start_time = "10:15".to_string();
        draft.duration_minutes.clear();
        event.time = draft.parsed_time().expect("parsed time");

        assert!(matches!(
            event.time,
            TimeSpec::Instant {
                end_utc: None,
                source_timezone: Some(ref timezone),
                ..
            } if timezone == "America/Mexico_City"
        ));
        let expected = chrono_tz::America::Mexico_City
            .with_ymd_and_hms(2026, 10, 7, 10, 15, 0)
            .single()
            .expect("expected")
            .with_timezone(&Utc);
        assert!(matches!(
            event.time,
            TimeSpec::Instant { start_utc, .. } if start_utc == expected
        ));
    }

    #[test]
    fn event_time_edit_preserves_date_kinds_and_end_exclusive() {
        let event = TemporalEvent::new(
            "All day",
            TimeSpec::AllDay {
                start: NaiveDate::from_ymd_opt(2026, 10, 7).expect("start"),
                end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 10, 9).expect("end")),
            },
        );
        let mut draft = EventTimeEditDraft::from_event(&event, chrono_tz::UTC).expect("time draft");
        assert_eq!(draft.end_date, "2026-10-09");
        draft.date = "2026-10-08".to_string();
        draft.end_date = "2026-10-10".to_string();
        assert_eq!(
            draft.parsed_time().expect("parsed"),
            TimeSpec::AllDay {
                start: NaiveDate::from_ymd_opt(2026, 10, 8).expect("start"),
                end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 10, 10).expect("end")),
            }
        );

        draft.end_date = "2026-10-08".to_string();
        assert!(draft.parsed_time().is_err());
    }

    #[test]
    fn event_time_edit_refuses_recurrence_but_preserves_bounded_uncertainty() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 7).expect("day");
        let mut recurring = TemporalEvent::new(
            "Recurring",
            TimeSpec::Floating {
                start: day.and_hms_opt(9, 0, 0).expect("time"),
                end: None,
                source_timezone: None,
            },
        );
        recurring.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Daily));
        assert!(EventTimeEditDraft::from_event(&recurring, chrono_tz::UTC).is_err());

        let mut uncertain = TemporalEvent::new(
            "Uncertain",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        uncertain.time_uncertainty = Some(TimeUncertainty::DateWindow {
            earliest: day,
            latest: day.succ_opt().expect("next day"),
        });
        let draft = EventTimeEditDraft::from_event(&uncertain, chrono_tz::UTC)
            .expect("bounded civil uncertainty is editable");
        assert_eq!(
            draft.uncertain_origin,
            Some((
                uncertain.time.clone(),
                uncertain.time_uncertainty.expect("uncertainty"),
            ))
        );
    }

    #[test]
    fn event_location_edit_roundtrips_structured_fields_and_removal() {
        let mut event = TemporalEvent::new(
            "Located",
            TimeSpec::DateOnly {
                start: NaiveDate::from_ymd_opt(2026, 10, 7).expect("date"),
                end_exclusive: None,
            },
        );
        event.location = Some(EventLocation {
            name: Some("Library".to_string()),
            address: Some("123 Main".to_string()),
            locality: Some("Ameca".to_string()),
            region: Some("Jalisco".to_string()),
            postal_code: Some("46600".to_string()),
            country: Some("Mexico".to_string()),
            latitude: Some(20.548),
            longitude: Some(-104.045),
            virtual_url: None,
        });

        let draft = EventLocationEditDraft::from_event(&event);
        assert_eq!(draft.name, "Library");
        assert_eq!(draft.locality, "Ameca");
        assert_eq!(draft.parsed_location().expect("location"), event.location);

        let empty = EventLocationEditDraft {
            event_id: event.id,
            ..EventLocationEditDraft::default()
        };
        assert_eq!(empty.parsed_location().expect("empty"), None);
    }

    #[test]
    fn event_location_edit_requires_paired_bounded_coordinates() {
        let mut draft = EventLocationEditDraft {
            event_id: Uuid::new_v4(),
            name: "Place".to_string(),
            ..EventLocationEditDraft::default()
        };
        draft.latitude = "20.0".to_string();
        assert!(draft.parsed_location().is_err());

        draft.longitude = "-104.0".to_string();
        assert!(draft.parsed_location().is_ok());

        draft.latitude = "91".to_string();
        assert!(draft.parsed_location().is_err());

        draft.latitude = "NaN".to_string();
        assert!(draft.parsed_location().is_err());
    }

    #[test]
    fn event_details_numeric_parsers_are_strict_and_blank_aware() {
        assert_eq!(
            parse_optional_confidence("").expect("blank confidence"),
            None
        );
        assert_eq!(
            parse_optional_confidence("0.75").expect("confidence"),
            Some(0.75)
        );
        assert!(parse_optional_confidence("1.1").is_err());
        assert!(parse_optional_confidence("NaN").is_err());

        assert_eq!(parse_optional_i32("", "importance").expect("blank"), None);
        assert_eq!(
            parse_optional_i32("-2", "importance").expect("integer"),
            Some(-2)
        );
        assert!(parse_optional_i32("2.5", "importance").is_err());
    }

    #[test]
    fn remote_locator_display_hides_private_path_and_query() {
        assert_eq!(
            remote_locator_display("https://example.com/private/feed.ics?token=secret"),
            "https://example.com/…"
        );
        assert_eq!(
            remote_locator_display("http://localhost:8080/calendar.ics"),
            "http://localhost:8080/…"
        );
    }

    #[test]
    fn temporal_uncertainty_labels_preserve_window_shapes() {
        let timezone = chrono_tz::America::Mexico_City;
        let date = TimeUncertainty::DateWindow {
            earliest: NaiveDate::from_ymd_opt(2026, 10, 5).expect("earliest"),
            latest: NaiveDate::from_ymd_opt(2026, 10, 9).expect("latest"),
        };
        assert_eq!(
            temporal_uncertainty_labels(&date, timezone),
            ("2026-10-05".to_string(), "2026-10-09".to_string())
        );

        let floating = TimeUncertainty::FloatingWindow {
            earliest: NaiveDate::from_ymd_opt(2026, 10, 7)
                .expect("date")
                .and_hms_opt(9, 0, 0)
                .expect("time"),
            latest: NaiveDate::from_ymd_opt(2026, 10, 7)
                .expect("date")
                .and_hms_opt(11, 0, 0)
                .expect("time"),
        };
        assert_eq!(
            temporal_uncertainty_labels(&floating, timezone),
            (
                "2026-10-07 09:00:00".to_string(),
                "2026-10-07 11:00:00".to_string()
            )
        );

        let instant = TimeUncertainty::InstantWindow {
            earliest_utc: DateTime::parse_from_rfc3339("2026-10-07T15:00:00Z")
                .expect("earliest")
                .with_timezone(&Utc),
            latest_utc: DateTime::parse_from_rfc3339("2026-10-07T17:00:00Z")
                .expect("latest")
                .with_timezone(&Utc),
        };
        let (earliest, latest) = temporal_uncertainty_labels(&instant, timezone);
        assert!(earliest.starts_with("2026-10-07 09:00:00"));
        assert!(latest.starts_with("2026-10-07 11:00:00"));
    }

    #[test]
    fn default_ics_export_path_uses_safe_sibling_name() {
        let mut source = TemporalSource::new(
            "Calendar",
            crate::domain::SourceKind::Ics,
            crate::domain::SourceAuthority::Unknown,
        );
        source.locator = Some("/tmp/calendar.ics".to_string());

        assert_eq!(
            default_ics_export_path(&source),
            "/tmp/calendar-ephemeris.ics"
        );

        source.locator = None;
        assert_eq!(
            default_ics_export_path(&source),
            format!("ephemeris-{}.ics", source.id)
        );

        source.kind = crate::domain::SourceKind::Webcal;
        source.locator = Some("https://example.com/calendar.ics".to_string());
        assert_eq!(
            default_ics_export_path(&source),
            format!("ephemeris-{}.ics", source.id)
        );
    }

    #[test]
    fn csv_routing_and_default_export_path_are_explicit() {
        assert!(is_csv_path(std::path::Path::new("/tmp/events.csv")));
        assert!(is_csv_path(std::path::Path::new("/tmp/EVENTS.CSV")));
        assert!(!is_csv_path(std::path::Path::new("/tmp/events.json")));

        let mut source = TemporalSource::new(
            "CSV",
            crate::domain::SourceKind::Csv,
            crate::domain::SourceAuthority::Unknown,
        );
        source.locator = Some("/tmp/events.csv".to_string());
        assert_eq!(
            default_csv_export_path(&source),
            "/tmp/events-ephemeris.csv"
        );
        source.locator = None;
        assert_eq!(
            default_csv_export_path(&source),
            format!("ephemeris-{}.csv", source.id)
        );
    }

    #[test]
    fn details_conflict_checks_only_new_blocking_commitments() {
        let time = TimeSpec::AllDay {
            start: NaiveDate::from_ymd_opt(2026, 10, 8).expect("date"),
            end_exclusive: None,
        };
        let mut original = TemporalEvent::new("Free event", time);
        original.availability = AvailabilityBehavior::Free;
        let mut changed = original.clone();
        changed.availability = AvailabilityBehavior::Busy;
        assert!(details_change_requires_conflict_check(&original, &changed));

        let mut renamed = changed.clone();
        renamed.normalized_title = "Renamed event".to_string();
        assert!(!details_change_requires_conflict_check(&changed, &renamed));

        let mut confirmed = changed.clone();
        confirmed.status = EventStatus::Confirmed;
        assert!(!details_change_requires_conflict_check(
            &changed, &confirmed
        ));

        let mut cancelled = changed.clone();
        cancelled.status = EventStatus::Cancelled;
        assert!(!details_change_requires_conflict_check(
            &changed, &cancelled
        ));

        let mut date_only = changed.clone();
        date_only.time = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 10, 8).expect("date"),
            end_exclusive: None,
        };
        assert!(!details_change_requires_conflict_check(
            &original, &date_only
        ));
    }

    #[test]
    fn conflict_confirmation_invalidates_when_uncertainty_bounds_change() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 8).expect("date");
        let mut event = TemporalEvent::new(
            "Uncertain event",
            TimeSpec::DateOnly {
                start: date,
                end_exclusive: None,
            },
        );
        event.time_uncertainty = Some(TimeUncertainty::DateWindow {
            earliest: date - chrono::Days::new(1),
            latest: date + chrono::Days::new(1),
        });
        let confirmation = ConflictConfirmation::for_event(
            &event,
            "provisional window acknowledgement".to_string(),
        );
        assert!(confirmation.matches(&event));
        event.time_uncertainty = Some(TimeUncertainty::DateWindow {
            earliest: date - chrono::Days::new(2),
            latest: date + chrono::Days::new(1),
        });
        assert!(!confirmation.matches(&event));
    }

    #[test]
    fn uncertain_newly_busy_details_require_provisional_confirmation_only_on_activation() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 8).expect("date");
        let mut original = TemporalEvent::new(
            "Uncertain event",
            TimeSpec::DateOnly {
                start: date,
                end_exclusive: None,
            },
        );
        original.time_uncertainty = Some(TimeUncertainty::DateWindow {
            earliest: date - chrono::Days::new(1),
            latest: date + chrono::Days::new(1),
        });
        original.availability = AvailabilityBehavior::Free;
        let mut activated = original.clone();
        activated.availability = AvailabilityBehavior::Busy;

        assert!(
            details_change_requires_provisional_uncertainty_confirmation(&original, &activated)
        );
        assert!(!details_change_requires_conflict_check(
            &original, &activated
        ));

        let mut renamed = activated.clone();
        renamed.normalized_title = "Updated title".to_string();
        assert!(
            !details_change_requires_provisional_uncertainty_confirmation(&activated, &renamed)
        );

        let mut cancelled = activated.clone();
        cancelled.status = EventStatus::Cancelled;
        assert!(
            !details_change_requires_provisional_uncertainty_confirmation(&activated, &cancelled)
        );
        let mut restored = cancelled.clone();
        restored.status = EventStatus::Scheduled;
        assert!(
            details_change_requires_provisional_uncertainty_confirmation(&cancelled, &restored)
        );

        let mut definite = activated.clone();
        definite.time_uncertainty = None;
        assert!(
            !details_change_requires_provisional_uncertainty_confirmation(&original, &definite)
        );
        let mut recurring = activated.clone();
        recurring.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Daily));
        assert!(
            !details_change_requires_provisional_uncertainty_confirmation(&original, &recurring)
        );
    }

    #[test]
    fn dropped_import_routing_recognizes_ics_case_insensitively() {
        assert!(is_ics_path(std::path::Path::new("/tmp/calendar.ics")));
        assert!(is_ics_path(std::path::Path::new("/tmp/CALENDAR.ICAL")));
        assert!(!is_ics_path(std::path::Path::new("/tmp/events.json")));
        assert!(!is_ics_path(std::path::Path::new("/tmp/no-extension")));
    }

    #[test]
    fn dropped_import_routing_distinguishes_canonical_json_from_taria_json() {
        assert!(is_canonical_json_path(std::path::Path::new(
            "/tmp/full.ephemeris.json"
        )));
        assert!(!is_canonical_json_path(std::path::Path::new(
            "/tmp/reconciled-event-set.json"
        )));
        assert!(!is_canonical_json_path(std::path::Path::new(
            "/tmp/calendar.ics"
        )));
    }

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
    fn summary_rows_use_existing_grouping_dimension() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut politics = TemporalEvent::new(
            "Politics",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        politics.domain = Some("politics".to_string());

        let mut finance = TemporalEvent::new(
            "Finance",
            TimeSpec::DateOnly {
                start: day + chrono::Duration::days(1),
                end_exclusive: None,
            },
        );
        finance.domain = Some("finance".to_string());

        let mut politics_two = TemporalEvent::new(
            "Politics 2",
            TimeSpec::DateOnly {
                start: day + chrono::Duration::days(2),
                end_exclusive: None,
            },
        );
        politics_two.domain = Some("politics".to_string());

        let rows = summary_rows(
            &[politics, finance, politics_two],
            chrono_tz::UTC,
            GroupBy::Domain,
        );

        assert_eq!(rows.len(), 2);
        let politics = rows
            .iter()
            .find(|row| row.label == "politics")
            .expect("politics row");
        assert_eq!(politics.count, 2);
        assert_eq!(politics.first_date, Some(day));
        assert_eq!(politics.last_date, Some(day + chrono::Duration::days(2)));
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

    #[test]
    fn recurrence_edit_draft_parses_core_fields() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let event = TemporalEvent::new(
            "Editable",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.rule.frequency = RecurrenceFrequency::Weekly;
        draft.rule.by_weekday = vec![RecurrenceWeekday::Monday, RecurrenceWeekday::Friday];
        draft.interval_text = "2".to_string();
        draft.count_text = "6".to_string();
        draft.until_text = "2026-12-31".to_string();

        let rule = draft.parsed_rule().expect("parsed rule");

        assert_eq!(rule.frequency, RecurrenceFrequency::Weekly);
        assert_eq!(rule.interval, 2);
        assert_eq!(rule.count, Some(6));
        assert_eq!(
            rule.until,
            Some(NaiveDate::from_ymd_opt(2026, 12, 31).expect("until"))
        );
        assert_eq!(
            rule.by_weekday,
            vec![RecurrenceWeekday::Monday, RecurrenceWeekday::Friday]
        );
    }

    #[test]
    fn recurrence_edit_draft_preserves_advanced_selectors() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Advanced",
            TimeSpec::Floating {
                start: day.and_hms_opt(9, 0, 0).expect("time"),
                end: None,
                source_timezone: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.by_hour = vec![9, 17];
        rule.by_minute = vec![15, 45];
        rule.by_second = vec![30];
        rule.by_set_pos = vec![-1];
        rule.rdates = vec![TimeSpec::Floating {
            start: day.and_hms_opt(12, 0, 0).expect("rdate"),
            end: None,
            source_timezone: None,
        }];
        event.recurrence = Some(rule.clone());

        let draft = RecurrenceEditDraft::from_event(&event);
        let parsed = draft.parsed_rule().expect("parsed rule");

        assert_eq!(draft.rdate_text, "2026-10-05T12:00:00");
        assert!(draft.override_text.is_empty());
        assert_eq!(parsed.by_hour, rule.by_hour);
        assert_eq!(parsed.by_minute, rule.by_minute);
        assert_eq!(parsed.by_second, rule.by_second);
        assert_eq!(parsed.by_set_pos, rule.by_set_pos);
        assert_eq!(parsed.rdates, rule.rdates);
    }

    #[test]
    fn recurrence_edit_draft_rejects_invalid_core_text() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let event = TemporalEvent::new(
            "Invalid draft",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let mut draft = RecurrenceEditDraft::from_event(&event);

        draft.interval_text = "nope".to_string();
        assert!(draft.parsed_rule().is_err());

        draft.interval_text = "1".to_string();
        draft.count_text = "0".to_string();
        assert!(draft.parsed_rule().is_err());

        draft.count_text.clear();
        draft.until_text = "10/05/2026".to_string();
        assert!(draft.parsed_rule().is_err());
    }

    #[test]
    fn event_revision_summary_reports_semantic_changes_without_timestamps() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 7).expect("date");
        let original = TemporalEvent::new(
            "Original",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        assert_eq!(event_revision_change_summary(None, &original), "created");

        let mut touched = original.clone();
        touched.updated_at = Utc::now();
        assert_eq!(
            event_revision_change_summary(Some(&original), &touched),
            "canonical metadata changed"
        );

        let mut changed = touched.clone();
        changed.normalized_title = "Changed".to_string();
        changed.status = EventStatus::Cancelled;
        changed.location = Some(crate::domain::EventLocation {
            name: Some("Test venue".to_string()),
            ..crate::domain::EventLocation::default()
        });
        changed.tags.push("important".to_string());
        assert_eq!(
            event_revision_change_summary(Some(&original), &changed),
            "title, status, location, tags"
        );
    }

    #[test]
    fn recurrence_edit_draft_starts_new_events_as_daily_unbounded() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let event = TemporalEvent::new(
            "New recurrence",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );

        let draft = RecurrenceEditDraft::from_event(&event);

        assert!(!draft.had_recurrence);
        assert_eq!(draft.rule.frequency, RecurrenceFrequency::Daily);
        assert_eq!(draft.interval_text, "1");
        assert!(draft.count_text.is_empty());
        assert!(draft.until_text.is_empty());
    }

    #[test]
    fn recurrence_weekdays_preset_builds_daily_weekday_rule() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let event = TemporalEvent::new(
            "Weekdays preset",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let mut draft = RecurrenceEditDraft::from_event(&event);

        draft.apply_preset(RecurrencePreset::Weekdays);
        let parsed = draft.parsed_rule().expect("weekday preset");

        assert_eq!(parsed.frequency, RecurrenceFrequency::Daily);
        assert_eq!(parsed.interval, 1);
        assert_eq!(
            parsed.by_weekday,
            vec![
                RecurrenceWeekday::Monday,
                RecurrenceWeekday::Tuesday,
                RecurrenceWeekday::Wednesday,
                RecurrenceWeekday::Thursday,
                RecurrenceWeekday::Friday,
            ]
        );
        assert!(parsed.by_month.is_empty());
        assert!(parsed.by_set_pos.is_empty());
    }

    #[test]
    fn recurrence_last_weekday_preset_resets_conflicting_selectors() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let event = TemporalEvent::new(
            "Last weekday preset",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.rule.frequency = RecurrenceFrequency::Yearly;
        draft.rule.week_start = RecurrenceWeekday::Sunday;
        draft.rule.by_weekday = vec![RecurrenceWeekday::Saturday];
        draft.rule.by_month = vec![2, 8];
        draft.interval_text = "3".to_string();
        draft.week_no_text = "20".to_string();
        draft.year_day_text = "100".to_string();
        draft.month_day_text = "15".to_string();
        draft.ordinal_byday_text = "1MO".to_string();
        draft.hour_text = "9".to_string();
        draft.minute_text = "30".to_string();
        draft.second_text = "15".to_string();
        draft.set_pos_text = "2".to_string();

        draft.apply_preset(RecurrencePreset::LastWeekdayOfMonth);
        let parsed = draft.parsed_rule().expect("last weekday preset");

        assert_eq!(parsed.frequency, RecurrenceFrequency::Monthly);
        assert_eq!(parsed.interval, 1);
        assert_eq!(parsed.week_start, RecurrenceWeekday::Monday);
        assert_eq!(
            parsed.by_weekday,
            vec![
                RecurrenceWeekday::Monday,
                RecurrenceWeekday::Tuesday,
                RecurrenceWeekday::Wednesday,
                RecurrenceWeekday::Thursday,
                RecurrenceWeekday::Friday,
            ]
        );
        assert_eq!(parsed.by_set_pos, vec![-1]);
        assert!(parsed.by_month.is_empty());
        assert!(parsed.by_week_no.is_empty());
        assert!(parsed.by_year_day.is_empty());
        assert!(parsed.by_month_day.is_empty());
        assert!(parsed.by_month_weekday.is_empty());
        assert!(parsed.by_hour.is_empty());
        assert!(parsed.by_minute.is_empty());
        assert!(parsed.by_second.is_empty());
    }

    #[test]
    fn recurrence_basic_presets_use_frequency_defaults() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let event = TemporalEvent::new(
            "Basic presets",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let mut draft = RecurrenceEditDraft::from_event(&event);

        for (preset, frequency) in [
            (RecurrencePreset::Daily, RecurrenceFrequency::Daily),
            (RecurrencePreset::Weekly, RecurrenceFrequency::Weekly),
            (RecurrencePreset::Monthly, RecurrenceFrequency::Monthly),
            (RecurrencePreset::Yearly, RecurrenceFrequency::Yearly),
        ] {
            draft.apply_preset(preset);
            let parsed = draft.parsed_rule().expect("basic preset");
            assert_eq!(parsed.frequency, frequency);
            assert_eq!(parsed.interval, 1);
            assert!(parsed.by_weekday.is_empty());
            assert!(parsed.by_month.is_empty());
            assert!(parsed.by_set_pos.is_empty());
        }
    }

    #[test]
    fn recurrence_presets_preserve_bounds_and_exception_editor_state() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let mut event = TemporalEvent::new(
            "Preserve preset state",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.count = Some(8);
        rule.until = Some(NaiveDate::from_ymd_opt(2026, 12, 31).expect("until"));
        rule.rdates = vec![TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 11, 1).expect("rdate"),
            end_exclusive: None,
        }];
        rule.exdates = vec![TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 10, 7).expect("exdate"),
            end_exclusive: None,
        }];
        rule.overrides = vec![RecurrenceOverride {
            original: TimeSpec::DateOnly {
                start: NaiveDate::from_ymd_opt(2026, 10, 8).expect("original"),
                end_exclusive: None,
            },
            replacement: None,
            cancelled: true,
        }];
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        let count_text = draft.count_text.clone();
        let until_text = draft.until_text.clone();
        let rdate_rows = draft.rdate_rows.clone();
        let exdate_rows = draft.exdate_rows.clone();
        let override_rows = draft.override_rows.clone();
        let rdate_text = draft.rdate_text.clone();
        let exdate_text = draft.exdate_text.clone();
        let override_text = draft.override_text.clone();

        draft.apply_preset(RecurrencePreset::Monthly);

        assert_eq!(draft.count_text, count_text);
        assert_eq!(draft.until_text, until_text);
        assert_eq!(draft.rdate_rows, rdate_rows);
        assert_eq!(draft.exdate_rows, exdate_rows);
        assert_eq!(draft.override_rows, override_rows);
        assert_eq!(draft.rdate_text, rdate_text);
        assert_eq!(draft.exdate_text, exdate_text);
        assert_eq!(draft.override_text, override_text);
    }

    #[test]
    fn recurrence_selector_numeric_parsers_accept_commas_and_spaces() {
        assert_eq!(
            parse_i8_selector_values("1, 15 -1", "BYMONTHDAY").expect("i8 selectors"),
            vec![1, 15, -1]
        );
        assert_eq!(
            parse_i16_selector_values("1 100,-1", "BYYEARDAY").expect("i16 selectors"),
            vec![1, 100, -1]
        );
        assert_eq!(
            parse_u8_selector_values("9,17 23", "BYHOUR").expect("u8 selectors"),
            vec![9, 17, 23]
        );
    }

    #[test]
    fn recurrence_selector_ordinal_byday_parser_roundtrips() {
        let selectors = parse_ordinal_byday_values("1MO,-1fr 3WE").expect("ordinal selectors");
        assert_eq!(
            selectors,
            vec![
                RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday),
                RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday),
                RecurrenceOrdinalWeekday::new(3, RecurrenceWeekday::Wednesday),
            ]
        );
        assert_eq!(format_ordinal_byday_values(&selectors), "1MO,-1FR,3WE");
    }

    #[test]
    fn recurrence_selector_parsers_reject_malformed_values() {
        assert!(parse_i8_selector_values("1,nope", "BYMONTHDAY").is_err());
        assert!(parse_u8_selector_values("-1", "BYHOUR").is_err());
        assert!(parse_ordinal_byday_values("MO").is_err());
        assert!(parse_ordinal_byday_values("1XX").is_err());
    }

    #[test]
    fn recurrence_edit_draft_applies_advanced_selector_text() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Advanced edit",
            TimeSpec::Floating {
                start: day.and_hms_opt(9, 0, 0).expect("time"),
                end: None,
                source_timezone: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Yearly);
        rule.by_month = vec![1, 7];
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.year_day_text = "1,-1".to_string();
        draft.month_day_text = "1,-1".to_string();
        draft.ordinal_byday_text = "1MO,-1FR".to_string();
        draft.hour_text = "9,17".to_string();
        draft.minute_text = "0,30".to_string();
        draft.second_text = "15".to_string();
        draft.set_pos_text = "1,-1".to_string();

        let parsed = draft.parsed_rule().expect("advanced rule");

        assert_eq!(parsed.by_year_day, vec![1, -1]);
        assert_eq!(parsed.by_month_day, vec![1, -1]);
        assert_eq!(
            parsed.by_month_weekday,
            vec![
                RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday),
                RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday),
            ]
        );
        assert_eq!(parsed.by_hour, vec![9, 17]);
        assert_eq!(parsed.by_minute, vec![0, 30]);
        assert_eq!(parsed.by_second, vec![15]);
        assert_eq!(parsed.by_set_pos, vec![1, -1]);
    }

    #[test]
    fn recurrence_exception_date_parser_preserves_date_only_duration() {
        let base = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 1, 10).expect("start"),
            end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 1, 13).expect("end")),
        };

        let parsed =
            parse_exception_start_values("2026-03-10,2026-04-15", &base, "RDATE").expect("parse");

        assert_eq!(
            parsed,
            vec![
                TimeSpec::DateOnly {
                    start: NaiveDate::from_ymd_opt(2026, 3, 10).expect("expected date"),
                    end_exclusive: Some(
                        NaiveDate::from_ymd_opt(2026, 3, 13).expect("expected date")
                    ),
                },
                TimeSpec::DateOnly {
                    start: NaiveDate::from_ymd_opt(2026, 4, 15).expect("expected date"),
                    end_exclusive: Some(
                        NaiveDate::from_ymd_opt(2026, 4, 18).expect("expected date")
                    ),
                },
            ]
        );
    }

    #[test]
    fn recurrence_exception_date_parser_preserves_instant_duration_and_timezone() {
        let start_utc = DateTime::parse_from_rfc3339("2026-03-01T14:00:00Z")
            .expect("start")
            .with_timezone(&Utc);
        let base = TimeSpec::Instant {
            start_utc,
            end_utc: Some(start_utc + ChronoDuration::minutes(90)),
            source_timezone: Some("America/New_York".to_string()),
        };

        let parsed =
            parse_exception_start_values("2026-04-01T13:00:00Z", &base, "RDATE").expect("parse");
        let TimeSpec::Instant {
            start_utc,
            end_utc,
            source_timezone,
        } = &parsed[0]
        else {
            panic!("expected instant");
        };

        assert_eq!(
            *start_utc,
            DateTime::parse_from_rfc3339("2026-04-01T13:00:00Z")
                .expect("expected start")
                .with_timezone(&Utc)
        );
        assert_eq!(
            *end_utc,
            Some(
                DateTime::parse_from_rfc3339("2026-04-01T14:30:00Z")
                    .expect("expected end")
                    .with_timezone(&Utc)
            )
        );
        assert_eq!(source_timezone.as_deref(), Some("America/New_York"));
        assert_eq!(
            format_exception_start_values(&parsed),
            "2026-04-01T13:00:00Z"
        );
    }

    #[test]
    fn recurrence_edit_draft_applies_rdate_and_exdate_text_preserving_overrides() {
        use crate::domain::RecurrenceOverride;

        let start = NaiveDate::from_ymd_opt(2026, 10, 5).expect("start");
        let mut event = TemporalEvent::new(
            "Exception edit",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        let original = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 10, 12).expect("original date"),
            end_exclusive: None,
        };
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.overrides = vec![RecurrenceOverride {
            original: original.clone(),
            replacement: Some(TimeSpec::DateOnly {
                start: NaiveDate::from_ymd_opt(2026, 10, 13).expect("replacement date"),
                end_exclusive: None,
            }),
            cancelled: false,
        }];
        event.recurrence = Some(rule.clone());

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.rdate_text = "2026-10-20,2026-10-21".to_string();
        draft.exdate_text = "2026-10-07".to_string();

        let parsed = draft.parsed_rule().expect("parsed rule");

        assert_eq!(parsed.rdates.len(), 2);
        assert_eq!(parsed.exdates.len(), 1);
        assert_eq!(parsed.overrides, rule.overrides);
        assert_eq!(draft.override_text, "2026-10-12=>2026-10-13");
    }

    #[test]
    fn structured_exception_rows_roundtrip_occurrence_starts() {
        let base = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 1, 10).expect("base"),
            end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 1, 13).expect("base end")),
        };
        let raw = "2026-03-10,2026-04-15";

        let rows =
            parse_recurrence_exception_edit_rows(raw, &base, "RDATE").expect("structured rows");

        assert_eq!(rows, vec!["2026-03-10", "2026-04-15"]);
        assert_eq!(format_recurrence_exception_edit_rows(&rows), raw);
    }

    #[test]
    fn recurrence_edit_draft_applies_structured_rdate_and_exdate_rows() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 5).expect("start");
        let mut event = TemporalEvent::new(
            "Structured exception dates",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.count = Some(10);
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.rdate_rows = vec!["2026-10-20".to_string(), "2026-10-21".to_string()];
        draft.exdate_rows = vec!["2026-10-07".to_string()];

        let parsed = draft.parsed_rule().expect("valid structured exceptions");

        assert_eq!(
            format_exception_start_values(&parsed.rdates),
            "2026-10-20,2026-10-21"
        );
        assert_eq!(format_exception_start_values(&parsed.exdates), "2026-10-07");
    }

    #[test]
    fn recurrence_edit_draft_rejects_blank_structured_exception_row() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 5).expect("start");
        let mut event = TemporalEvent::new(
            "Incomplete exception row",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Daily));

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.rdate_rows.push(String::new());

        assert!(draft.parsed_rule().is_err());
    }

    #[test]
    fn untouched_structured_exception_rows_preserve_persisted_payloads() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 5).expect("start");
        let mut event = TemporalEvent::new(
            "Preserve exception payloads",
            TimeSpec::DateOnly {
                start,
                end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 10, 6).expect("base end")),
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.count = Some(10);
        rule.rdates = vec![TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 10, 20).expect("rdate"),
            end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 10, 23).expect("rdate end")),
        }];
        rule.exdates = vec![TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 10, 7).expect("exdate"),
            end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 10, 9).expect("exdate end")),
        }];
        event.recurrence = Some(rule.clone());

        let draft = RecurrenceEditDraft::from_event(&event);
        let parsed = draft
            .parsed_rule()
            .expect("unchanged structured exceptions");

        assert_eq!(parsed.rdates, rule.rdates);
        assert_eq!(parsed.exdates, rule.exdates);
    }

    #[test]
    fn recurrence_edit_draft_rejects_malformed_exception_value() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).expect("day");
        let event = TemporalEvent::new(
            "Invalid exception",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.rdate_text = "10/20/2026".to_string();

        assert!(draft.parsed_rule().is_err());
    }

    #[test]
    fn recurrence_edit_draft_rejects_exdate_override_conflict_live() {
        use crate::domain::RecurrenceOverride;

        let start = NaiveDate::from_ymd_opt(2026, 10, 5).expect("start");
        let original = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 10, 12).expect("original date"),
            end_exclusive: None,
        };
        let mut event = TemporalEvent::new(
            "Conflict",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.overrides = vec![RecurrenceOverride {
            original: original.clone(),
            replacement: None,
            cancelled: true,
        }];
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.exdate_text = "2026-10-12".to_string();

        assert!(draft.parsed_rule().is_err());
    }

    #[test]
    fn recurrence_override_parser_roundtrips_move_cancel_keep_and_cancelled_move() {
        let base = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 10, 5).expect("base"),
            end_exclusive: None,
        };
        let raw = concat!(
            "2026-10-12=>2026-10-13\n",
            "2026-10-19=>CANCEL\n",
            "2026-10-26=>KEEP\n",
            "2026-11-02=>CANCEL@2026-11-03"
        );

        let parsed = parse_recurrence_override_values(raw, &base).expect("parse overrides");

        assert_eq!(parsed.len(), 4);
        assert_eq!(format_recurrence_override_values(&parsed), raw);
        assert!(!parsed[0].cancelled);
        assert!(parsed[0].replacement.is_some());
        assert!(parsed[1].cancelled);
        assert!(parsed[1].replacement.is_none());
        assert!(!parsed[2].cancelled);
        assert!(parsed[2].replacement.is_none());
        assert!(parsed[3].cancelled);
        assert!(parsed[3].replacement.is_some());
    }

    #[test]
    fn recurrence_override_parser_preserves_canonical_duration() {
        let base = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 10, 5).expect("base"),
            end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 10, 7).expect("base end")),
        };

        let parsed = parse_recurrence_override_values("2026-10-12=>2026-10-20", &base)
            .expect("parse override");

        assert_eq!(
            parsed[0].original,
            TimeSpec::DateOnly {
                start: NaiveDate::from_ymd_opt(2026, 10, 12).expect("original"),
                end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 10, 14).expect("original end")),
            }
        );
        assert_eq!(
            parsed[0].replacement,
            Some(TimeSpec::DateOnly {
                start: NaiveDate::from_ymd_opt(2026, 10, 20).expect("replacement"),
                end_exclusive: Some(
                    NaiveDate::from_ymd_opt(2026, 10, 22).expect("replacement end")
                ),
            })
        );
    }

    #[test]
    fn recurrence_edit_draft_applies_move_and_cancel_overrides_live() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 5).expect("start");
        let mut event = TemporalEvent::new(
            "Override editor",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.count = Some(10);
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.override_text = "2026-10-07=>2026-10-20\n2026-10-08=>CANCEL".to_string();

        let parsed = draft.parsed_rule().expect("valid overrides");

        assert_eq!(parsed.overrides.len(), 2);
        assert_eq!(
            format_recurrence_override_values(&parsed.overrides),
            draft.override_text
        );
    }

    #[test]
    fn recurrence_edit_draft_rejects_unknown_override_target_live() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 5).expect("start");
        let mut event = TemporalEvent::new(
            "Monday series",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Weekly);
        rule.by_weekday = vec![RecurrenceWeekday::Monday];
        rule.count = Some(4);
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.override_text = "2026-10-06=>CANCEL".to_string();

        assert!(draft.parsed_rule().is_err());
    }

    #[test]
    fn recurrence_edit_draft_rejects_duplicate_override_original_live() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 5).expect("start");
        let mut event = TemporalEvent::new(
            "Duplicate override",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.count = Some(5);
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.override_text = "2026-10-06=>CANCEL\n2026-10-06=>2026-10-10".to_string();

        assert!(draft.parsed_rule().is_err());
    }

    #[test]
    fn structured_override_rows_roundtrip_all_override_actions() {
        let base = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 10, 5).expect("base"),
            end_exclusive: None,
        };
        let raw = concat!(
            "2026-10-12=>2026-10-13\n",
            "2026-10-19=>CANCEL\n",
            "2026-10-26=>KEEP\n",
            "2026-11-02=>CANCEL@2026-11-03"
        );

        let rows = parse_recurrence_override_edit_rows(raw, &base).expect("structured rows");

        assert_eq!(format_recurrence_override_edit_rows(&rows), raw);
        assert_eq!(rows[0].action, RecurrenceOverrideEditAction::Move);
        assert_eq!(rows[1].action, RecurrenceOverrideEditAction::Cancel);
        assert_eq!(rows[2].action, RecurrenceOverrideEditAction::Keep);
        assert_eq!(rows[3].action, RecurrenceOverrideEditAction::CancelMove);
        assert_eq!(rows[3].replacement_text, "2026-11-03");
    }

    #[test]
    fn focused_occurrence_editor_does_not_invent_override_on_open() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 5).expect("start");
        let mut event = TemporalEvent::new(
            "Recurring meeting",
            TimeSpec::AllDay {
                start,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.count = Some(4);
        event.recurrence = Some(rule.clone());
        let mut draft = RecurrenceEditDraft::from_event(&event);
        let original = TimeSpec::AllDay {
            start: start
                .checked_add_days(chrono::Days::new(1))
                .expect("next day"),
            end_exclusive: None,
        };
        draft.focus_occurrence(&original).expect("focus");
        assert_eq!(draft.focused_occurrence_original, Some(original));
        assert!(draft.override_rows.is_empty());
        assert_eq!(draft.parsed_rule().expect("unchanged draft"), rule);
    }

    #[test]
    fn focused_moved_occurrence_uses_original_slot_not_replacement() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 5).expect("start");
        let original = TimeSpec::DateOnly {
            start: start
                .checked_add_days(chrono::Days::new(1))
                .expect("original day"),
            end_exclusive: None,
        };
        let replacement = TimeSpec::DateOnly {
            start: start
                .checked_add_days(chrono::Days::new(8))
                .expect("moved day"),
            end_exclusive: None,
        };
        let mut event = TemporalEvent::new(
            "Recurring observation",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.count = Some(4);
        rule.overrides = vec![RecurrenceOverride {
            original: original.clone(),
            replacement: Some(replacement.clone()),
            cancelled: false,
        }];
        event.recurrence = Some(rule.clone());

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.focus_occurrence(&original).expect("focus original");
        assert_eq!(draft.focused_occurrence_original, Some(original.clone()));
        assert_eq!(draft.override_rows.len(), 1);
        assert_eq!(
            draft.override_rows[0].original_text,
            format_exception_start_value(&original)
        );
        assert_eq!(
            draft.override_rows[0].replacement_text,
            format_exception_start_value(&replacement)
        );
        assert_eq!(draft.parsed_rule().expect("untouched override"), rule);
    }

    #[test]
    fn focused_occurrence_rejects_unrelated_time_kind() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 5).expect("start");
        let mut event = TemporalEvent::new(
            "Civil series",
            TimeSpec::AllDay {
                start,
                end_exclusive: None,
            },
        );
        let mut draft = RecurrenceEditDraft::from_event(&event);
        assert!(draft.focus_occurrence(&event.time).is_err());
        event.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Daily));
        let mut draft = RecurrenceEditDraft::from_event(&event);
        let foreign = TimeSpec::DateOnly {
            start,
            end_exclusive: None,
        };
        assert!(draft.focus_occurrence(&foreign).is_err());
        assert!(draft.focused_occurrence_original.is_none());
    }

    #[test]
    fn recurrence_edit_draft_applies_structured_override_rows() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 5).expect("start");
        let mut event = TemporalEvent::new(
            "Structured override editor",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.count = Some(10);
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.override_rows = vec![
            RecurrenceOverrideEditRow {
                original_text: "2026-10-07".to_string(),
                action: RecurrenceOverrideEditAction::Move,
                replacement_text: "2026-10-20".to_string(),
            },
            RecurrenceOverrideEditRow {
                original_text: "2026-10-08".to_string(),
                action: RecurrenceOverrideEditAction::CancelMove,
                replacement_text: "2026-10-21".to_string(),
            },
            RecurrenceOverrideEditRow {
                original_text: "2026-10-09".to_string(),
                action: RecurrenceOverrideEditAction::Keep,
                replacement_text: String::new(),
            },
        ];

        let parsed = draft.parsed_rule().expect("valid structured overrides");

        assert_eq!(parsed.overrides.len(), 3);
        assert!(!parsed.overrides[0].cancelled);
        assert!(parsed.overrides[0].replacement.is_some());
        assert!(parsed.overrides[1].cancelled);
        assert!(parsed.overrides[1].replacement.is_some());
        assert!(!parsed.overrides[2].cancelled);
        assert!(parsed.overrides[2].replacement.is_none());
    }

    #[test]
    fn untouched_structured_override_rows_preserve_persisted_payloads() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 5).expect("start");
        let mut event = TemporalEvent::new(
            "Preserve override payload",
            TimeSpec::DateOnly {
                start,
                end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 10, 6).expect("base end")),
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.count = Some(10);
        rule.overrides = vec![RecurrenceOverride {
            original: TimeSpec::DateOnly {
                start: NaiveDate::from_ymd_opt(2026, 10, 7).expect("original"),
                end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 10, 8).expect("original end")),
            },
            replacement: Some(TimeSpec::DateOnly {
                start: NaiveDate::from_ymd_opt(2026, 10, 20).expect("replacement"),
                end_exclusive: Some(
                    NaiveDate::from_ymd_opt(2026, 10, 22).expect("replacement end"),
                ),
            }),
            cancelled: false,
        }];
        event.recurrence = Some(rule.clone());

        let draft = RecurrenceEditDraft::from_event(&event);
        let parsed = draft.parsed_rule().expect("unchanged structured draft");

        assert_eq!(parsed.overrides, rule.overrides);
    }

    #[test]
    fn slot_search_parser_accepts_clock_and_minute_constraints() {
        let workdays = [true, true, true, true, true, false, false];
        let search = parse_slot_search("90", "15", "08:30", "18:00", workdays).expect("search");
        assert_eq!(search.duration_minutes, 90);
        assert_eq!(search.step_minutes, 15);
        assert_eq!(
            search.day_start,
            NaiveTime::from_hms_opt(8, 30, 0).expect("start")
        );
        assert_eq!(
            search.day_end,
            NaiveTime::from_hms_opt(18, 0, 0).expect("end")
        );
        assert_eq!(search.workdays, workdays);
    }

    #[test]
    fn suggested_slot_prefills_exact_requested_duration() {
        let start = Utc
            .with_ymd_and_hms(2026, 10, 7, 14, 0, 0)
            .single()
            .expect("start");
        let interval = FreeInterval {
            start_utc: start,
            end_utc: start + ChronoDuration::minutes(90),
        };

        let draft = new_event_draft_for_suggested_slot(&interval, chrono_tz::UTC).expect("draft");
        assert_eq!(draft.duration_minutes, "90");
    }

    #[test]
    fn free_interval_prefills_local_event_with_capped_one_hour_duration() {
        let timezone = chrono_tz::America::Mexico_City;
        let start = timezone
            .with_ymd_and_hms(2026, 10, 7, 9, 30, 0)
            .single()
            .expect("local start")
            .with_timezone(&Utc);
        let interval = FreeInterval {
            start_utc: start,
            end_utc: start + ChronoDuration::hours(3),
        };

        let draft = new_event_draft_for_free_interval(&interval, timezone).expect("draft");
        assert_eq!(draft.date, "2026-10-07");
        assert_eq!(draft.start_time, "09:30");
        assert_eq!(draft.duration_minutes, "60");
        assert!(!draft.all_day);
    }

    #[test]
    fn short_free_interval_prefills_exact_available_duration() {
        let timezone = chrono_tz::UTC;
        let start = Utc
            .with_ymd_and_hms(2026, 10, 7, 9, 0, 0)
            .single()
            .expect("start");
        let interval = FreeInterval {
            start_utc: start,
            end_utc: start + ChronoDuration::minutes(25),
        };

        let draft = new_event_draft_for_free_interval(&interval, timezone).expect("draft");
        assert_eq!(draft.duration_minutes, "25");
    }

    #[test]
    fn recurrence_override_parser_rejects_malformed_entry() {
        let base = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 10, 5).expect("base"),
            end_exclusive: None,
        };

        assert!(parse_recurrence_override_values("2026-10-06", &base).is_err());
        assert!(parse_recurrence_override_values("=>CANCEL", &base).is_err());
        assert!(parse_recurrence_override_values("2026-10-06=>", &base).is_err());
    }

    #[test]
    fn recurrence_editor_frequency_availability_respects_base_time_kind() {
        let date_only = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 10, 6).expect("date"),
            end_exclusive: None,
        };
        let floating = TimeSpec::Floating {
            start: NaiveDate::from_ymd_opt(2026, 10, 6)
                .expect("date")
                .and_hms_opt(9, 0, 0)
                .expect("time"),
            end: None,
            source_timezone: None,
        };

        for frequency in [
            RecurrenceFrequency::Secondly,
            RecurrenceFrequency::Minutely,
            RecurrenceFrequency::Hourly,
        ] {
            assert!(!recurrence_editor_frequency_available(
                frequency, &date_only
            ));
            assert!(recurrence_editor_frequency_available(frequency, &floating));
        }
        for frequency in [
            RecurrenceFrequency::Daily,
            RecurrenceFrequency::Weekly,
            RecurrenceFrequency::Monthly,
            RecurrenceFrequency::Yearly,
        ] {
            assert!(recurrence_editor_frequency_available(frequency, &date_only));
        }
    }

    #[test]
    fn recurrence_editor_selector_availability_matches_domain_frequency_matrix() {
        let timed = TimeSpec::Floating {
            start: NaiveDate::from_ymd_opt(2026, 10, 6)
                .expect("date")
                .and_hms_opt(9, 0, 0)
                .expect("time"),
            end: None,
            source_timezone: None,
        };

        assert!(recurrence_editor_selector_available(
            RecurrenceEditorSelector::WeekNo,
            RecurrenceFrequency::Yearly,
            &timed,
        ));
        assert!(!recurrence_editor_selector_available(
            RecurrenceEditorSelector::WeekNo,
            RecurrenceFrequency::Monthly,
            &timed,
        ));

        for frequency in [
            RecurrenceFrequency::Secondly,
            RecurrenceFrequency::Minutely,
            RecurrenceFrequency::Hourly,
            RecurrenceFrequency::Yearly,
        ] {
            assert!(recurrence_editor_selector_available(
                RecurrenceEditorSelector::YearDay,
                frequency,
                &timed,
            ));
        }
        for frequency in [
            RecurrenceFrequency::Daily,
            RecurrenceFrequency::Weekly,
            RecurrenceFrequency::Monthly,
        ] {
            assert!(!recurrence_editor_selector_available(
                RecurrenceEditorSelector::YearDay,
                frequency,
                &timed,
            ));
        }

        assert!(!recurrence_editor_selector_available(
            RecurrenceEditorSelector::MonthDay,
            RecurrenceFrequency::Weekly,
            &timed,
        ));
        assert!(recurrence_editor_selector_available(
            RecurrenceEditorSelector::MonthDay,
            RecurrenceFrequency::Daily,
            &timed,
        ));

        for frequency in [RecurrenceFrequency::Monthly, RecurrenceFrequency::Yearly] {
            assert!(recurrence_editor_selector_available(
                RecurrenceEditorSelector::OrdinalByDay,
                frequency,
                &timed,
            ));
        }
        assert!(!recurrence_editor_selector_available(
            RecurrenceEditorSelector::OrdinalByDay,
            RecurrenceFrequency::Weekly,
            &timed,
        ));
    }

    #[test]
    fn recurrence_editor_time_selectors_require_datetime_base() {
        let date_only = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 10, 6).expect("date"),
            end_exclusive: None,
        };
        let exact = TimeSpec::Instant {
            start_utc: DateTime::parse_from_rfc3339("2026-10-06T15:00:00Z")
                .expect("timestamp")
                .with_timezone(&Utc),
            end_utc: None,
            source_timezone: Some("America/Mexico_City".to_string()),
        };

        for selector in [
            RecurrenceEditorSelector::Hour,
            RecurrenceEditorSelector::Minute,
            RecurrenceEditorSelector::Second,
        ] {
            assert!(!recurrence_editor_selector_available(
                selector,
                RecurrenceFrequency::Daily,
                &date_only,
            ));
            assert!(recurrence_editor_selector_available(
                selector,
                RecurrenceFrequency::Daily,
                &exact,
            ));
        }
    }

    #[test]
    fn recurrence_editor_week_start_affordance_tracks_real_week_context() {
        assert!(recurrence_editor_week_start_available(
            RecurrenceFrequency::Weekly,
            true,
            false,
        ));
        assert!(!recurrence_editor_week_start_available(
            RecurrenceFrequency::Weekly,
            false,
            false,
        ));
        assert!(recurrence_editor_week_start_available(
            RecurrenceFrequency::Yearly,
            false,
            true,
        ));
        assert!(!recurrence_editor_week_start_available(
            RecurrenceFrequency::Yearly,
            true,
            false,
        ));
        assert!(!recurrence_editor_week_start_available(
            RecurrenceFrequency::Daily,
            true,
            true,
        ));
    }

    #[test]
    fn recurrence_editor_preserves_hidden_selector_text_across_frequency_change() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Preserve hidden selector",
            TimeSpec::Floating {
                start: day.and_hms_opt(9, 0, 0).expect("time"),
                end: None,
                source_timezone: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Yearly);
        rule.by_week_no = vec![20, -1];
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.rule.frequency = RecurrenceFrequency::Daily;

        assert!(!recurrence_editor_selector_available(
            RecurrenceEditorSelector::WeekNo,
            draft.rule.frequency,
            &draft.base_time,
        ));
        assert_eq!(draft.week_no_text, "20,-1");
        assert!(draft.parsed_rule().is_err());

        draft.rule.frequency = RecurrenceFrequency::Yearly;
        assert_eq!(
            draft
                .parsed_rule()
                .expect("restored yearly rule")
                .by_week_no,
            vec![20, -1]
        );
    }

    #[test]
    fn recurrence_ordinal_byday_edit_rows_roundtrip_compact_syntax() {
        let selectors = vec![
            RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday),
            RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday),
        ];
        let rows = recurrence_ordinal_byday_edit_rows(&selectors);

        assert_eq!(rows[0].ordinal_text, "1");
        assert_eq!(rows[0].weekday, RecurrenceWeekday::Monday);
        assert_eq!(rows[1].ordinal_text, "-1");
        assert_eq!(rows[1].weekday, RecurrenceWeekday::Friday);
        assert_eq!(format_recurrence_ordinal_byday_edit_rows(&rows), "1MO,-1FR");
        assert_eq!(
            parse_recurrence_ordinal_byday_edit_rows("1MO,-1FR")
                .expect("parse")
                .iter()
                .map(RecurrenceOrdinalByDayEditRow::compact_token)
                .collect::<Vec<_>>(),
            vec!["1MO", "-1FR"]
        );
    }

    #[test]
    fn recurrence_edit_draft_applies_structured_ordinal_byday_rows() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Structured ordinal BYDAY",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Monthly));

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.ordinal_byday_rows = vec![
            RecurrenceOrdinalByDayEditRow {
                ordinal_text: "1".to_string(),
                weekday: RecurrenceWeekday::Monday,
            },
            RecurrenceOrdinalByDayEditRow {
                ordinal_text: "-1".to_string(),
                weekday: RecurrenceWeekday::Friday,
            },
        ];
        draft.ordinal_byday_text =
            format_recurrence_ordinal_byday_edit_rows(&draft.ordinal_byday_rows);

        let parsed = draft.parsed_rule().expect("structured rule");
        assert_eq!(
            parsed.by_month_weekday,
            vec![
                RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday),
                RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday),
            ]
        );
    }

    #[test]
    fn recurrence_edit_draft_raw_ordinal_byday_override_remains_supported() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Raw ordinal BYDAY",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Monthly);
        rule.by_month_weekday = vec![RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday)];
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.ordinal_byday_text = "-1FR".to_string();

        let parsed = draft.parsed_rule().expect("raw override");
        assert_eq!(
            parsed.by_month_weekday,
            vec![RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday)]
        );
        assert_eq!(draft.ordinal_byday_rows[0].ordinal_text, "1");
    }

    #[test]
    fn recurrence_ordinal_byday_edit_rows_reject_blank_ordinal() {
        let rows = vec![RecurrenceOrdinalByDayEditRow {
            ordinal_text: String::new(),
            weekday: RecurrenceWeekday::Monday,
        }];

        assert_eq!(
            validate_recurrence_ordinal_byday_edit_rows(&rows),
            Err("Ordinal BYDAY row #1 needs an ordinal or must be removed.".to_string())
        );
    }

    #[test]
    fn recurrence_preset_clears_structured_ordinal_byday_rows() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Preset clears structured ordinal BYDAY",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Monthly);
        rule.by_month_weekday = vec![RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday)];
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        assert_eq!(draft.ordinal_byday_rows.len(), 1);

        draft.apply_preset(RecurrencePreset::Daily);

        assert!(draft.ordinal_byday_text.is_empty());
        assert!(draft.ordinal_byday_rows.is_empty());
    }

    #[test]
    fn recurrence_monthday_edit_rows_roundtrip_compact_syntax() {
        let rows = recurrence_monthday_edit_rows(&[1, 15, -1]);

        assert_eq!(rows, vec!["1", "15", "-1"]);
        assert_eq!(format_recurrence_monthday_edit_rows(&rows), "1,15,-1");
        assert_eq!(
            parse_recurrence_monthday_edit_rows("1, 15 -1").expect("parse"),
            vec!["1", "15", "-1"]
        );
    }

    #[test]
    fn recurrence_edit_draft_applies_structured_monthday_rows() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Structured BYMONTHDAY",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Monthly));

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.month_day_rows = vec!["1".to_string(), "15".to_string(), "-1".to_string()];
        draft.month_day_text = format_recurrence_monthday_edit_rows(&draft.month_day_rows);

        let parsed = draft.parsed_rule().expect("structured rule");
        assert_eq!(parsed.by_month_day, vec![1, 15, -1]);
    }

    #[test]
    fn recurrence_edit_draft_raw_monthday_override_remains_supported() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Raw BYMONTHDAY",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Monthly);
        rule.by_month_day = vec![1];
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.month_day_text = "15,-1".to_string();

        let parsed = draft.parsed_rule().expect("raw override");
        assert_eq!(parsed.by_month_day, vec![15, -1]);
        assert_eq!(draft.month_day_rows, vec!["1"]);
    }

    #[test]
    fn recurrence_monthday_edit_rows_reject_blank_day() {
        let rows = vec![String::new()];

        assert_eq!(
            validate_recurrence_monthday_edit_rows(&rows),
            Err("BYMONTHDAY row #1 needs a signed day or must be removed.".to_string())
        );
    }

    #[test]
    fn recurrence_preset_clears_structured_monthday_rows() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Preset clears structured BYMONTHDAY",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Monthly);
        rule.by_month_day = vec![1, -1];
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        assert_eq!(draft.month_day_rows, vec!["1", "-1"]);

        draft.apply_preset(RecurrencePreset::Daily);

        assert!(draft.month_day_text.is_empty());
        assert!(draft.month_day_rows.is_empty());
    }

    #[test]
    fn recurrence_weekno_edit_rows_roundtrip_compact_syntax() {
        let rows = recurrence_weekno_edit_rows(&[20, -1]);

        assert_eq!(rows, vec!["20", "-1"]);
        assert_eq!(format_recurrence_weekno_edit_rows(&rows), "20,-1");
        assert_eq!(
            parse_recurrence_weekno_edit_rows("20 -1").expect("parse"),
            vec!["20", "-1"]
        );
    }

    #[test]
    fn recurrence_edit_draft_applies_structured_weekno_rows() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Structured BYWEEKNO",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Yearly));

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.week_no_rows = vec!["20".to_string(), "-1".to_string()];
        draft.week_no_text = format_recurrence_weekno_edit_rows(&draft.week_no_rows);

        let parsed = draft.parsed_rule().expect("structured rule");
        assert_eq!(parsed.by_week_no, vec![20, -1]);
    }

    #[test]
    fn recurrence_edit_draft_raw_weekno_override_remains_supported() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Raw BYWEEKNO",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Yearly);
        rule.by_week_no = vec![20];
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.week_no_text = "1,-1".to_string();

        let parsed = draft.parsed_rule().expect("raw override");
        assert_eq!(parsed.by_week_no, vec![1, -1]);
        assert_eq!(draft.week_no_rows, vec!["20"]);
    }

    #[test]
    fn recurrence_weekno_edit_rows_reject_blank_week() {
        let rows = vec![String::new()];

        assert_eq!(
            validate_recurrence_weekno_edit_rows(&rows),
            Err("BYWEEKNO row #1 needs a signed week number or must be removed.".to_string())
        );
    }

    #[test]
    fn recurrence_preset_clears_structured_weekno_rows() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Preset clears structured BYWEEKNO",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Yearly);
        rule.by_week_no = vec![20, -1];
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        assert_eq!(draft.week_no_rows, vec!["20", "-1"]);

        draft.apply_preset(RecurrencePreset::Daily);

        assert!(draft.week_no_text.is_empty());
        assert!(draft.week_no_rows.is_empty());
    }

    #[test]
    fn recurrence_yearday_edit_rows_roundtrip_compact_syntax() {
        let rows = recurrence_yearday_edit_rows(&[1, 100, -1]);

        assert_eq!(rows, vec!["1", "100", "-1"]);
        assert_eq!(format_recurrence_yearday_edit_rows(&rows), "1,100,-1");
        assert_eq!(
            parse_recurrence_yearday_edit_rows("1 100,-1").expect("parse"),
            vec!["1", "100", "-1"]
        );
    }

    #[test]
    fn recurrence_edit_draft_applies_structured_yearday_rows() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Structured BYYEARDAY",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Yearly));

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.year_day_rows = vec!["1".to_string(), "100".to_string(), "-1".to_string()];
        draft.year_day_text = format_recurrence_yearday_edit_rows(&draft.year_day_rows);

        let parsed = draft.parsed_rule().expect("structured rule");
        assert_eq!(parsed.by_year_day, vec![1, 100, -1]);
    }

    #[test]
    fn recurrence_edit_draft_raw_yearday_override_remains_supported() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Raw BYYEARDAY",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Yearly);
        rule.by_year_day = vec![1];
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.year_day_text = "100,-1".to_string();

        let parsed = draft.parsed_rule().expect("raw override");
        assert_eq!(parsed.by_year_day, vec![100, -1]);
        assert_eq!(draft.year_day_rows, vec!["1"]);
    }

    #[test]
    fn recurrence_yearday_edit_rows_reject_blank_day() {
        let rows = vec![String::new()];

        assert_eq!(
            validate_recurrence_yearday_edit_rows(&rows),
            Err("BYYEARDAY row #1 needs a signed year day or must be removed.".to_string())
        );
    }

    #[test]
    fn recurrence_preset_clears_structured_yearday_rows() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Preset clears structured BYYEARDAY",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Yearly);
        rule.by_year_day = vec![1, -1];
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        assert_eq!(draft.year_day_rows, vec!["1", "-1"]);

        draft.apply_preset(RecurrencePreset::Daily);

        assert!(draft.year_day_text.is_empty());
        assert!(draft.year_day_rows.is_empty());
    }

    #[test]
    fn recurrence_hour_edit_rows_roundtrip_compact_syntax() {
        let rows = recurrence_hour_edit_rows(&[9, 17, 23]);

        assert_eq!(rows, vec!["9", "17", "23"]);
        assert_eq!(format_recurrence_hour_edit_rows(&rows), "9,17,23");
        assert_eq!(
            parse_recurrence_hour_edit_rows("9 17,23").expect("parse"),
            vec!["9", "17", "23"]
        );
    }

    #[test]
    fn recurrence_edit_draft_applies_structured_hour_rows() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Structured BYHOUR",
            TimeSpec::Floating {
                start: day.and_hms_opt(9, 0, 0).expect("time"),
                end: None,
                source_timezone: None,
            },
        );
        event.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Daily));

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.hour_rows = vec!["9".to_string(), "17".to_string()];
        draft.hour_text = format_recurrence_hour_edit_rows(&draft.hour_rows);

        let parsed = draft.parsed_rule().expect("structured rule");
        assert_eq!(parsed.by_hour, vec![9, 17]);
    }

    #[test]
    fn recurrence_edit_draft_raw_hour_override_remains_supported() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Raw BYHOUR",
            TimeSpec::Floating {
                start: day.and_hms_opt(9, 0, 0).expect("time"),
                end: None,
                source_timezone: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.by_hour = vec![9];
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.hour_text = "17,23".to_string();

        let parsed = draft.parsed_rule().expect("raw override");
        assert_eq!(parsed.by_hour, vec![17, 23]);
        assert_eq!(draft.hour_rows, vec!["9"]);
    }

    #[test]
    fn recurrence_hour_edit_rows_reject_blank_hour() {
        let rows = vec![String::new()];

        assert_eq!(
            validate_recurrence_hour_edit_rows(&rows),
            Err("BYHOUR row #1 needs an hour or must be removed.".to_string())
        );
    }

    #[test]
    fn recurrence_preset_clears_structured_hour_rows() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Preset clears structured BYHOUR",
            TimeSpec::Floating {
                start: day.and_hms_opt(9, 0, 0).expect("time"),
                end: None,
                source_timezone: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.by_hour = vec![9, 17];
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        assert_eq!(draft.hour_rows, vec!["9", "17"]);

        draft.apply_preset(RecurrencePreset::Daily);

        assert!(draft.hour_text.is_empty());
        assert!(draft.hour_rows.is_empty());
    }

    #[test]
    fn recurrence_minute_edit_rows_roundtrip_compact_syntax() {
        let rows = recurrence_minute_edit_rows(&[0, 30, 59]);

        assert_eq!(rows, vec!["0", "30", "59"]);
        assert_eq!(format_recurrence_minute_edit_rows(&rows), "0,30,59");
        assert_eq!(
            parse_recurrence_minute_edit_rows("0 30,59").expect("parse"),
            vec!["0", "30", "59"]
        );
    }

    #[test]
    fn recurrence_edit_draft_applies_structured_minute_rows() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Structured BYMINUTE",
            TimeSpec::Floating {
                start: day.and_hms_opt(9, 0, 0).expect("time"),
                end: None,
                source_timezone: None,
            },
        );
        event.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Daily));

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.minute_rows = vec!["0".to_string(), "30".to_string()];
        draft.minute_text = format_recurrence_minute_edit_rows(&draft.minute_rows);

        let parsed = draft.parsed_rule().expect("structured rule");
        assert_eq!(parsed.by_minute, vec![0, 30]);
    }

    #[test]
    fn recurrence_edit_draft_raw_minute_override_remains_supported() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Raw BYMINUTE",
            TimeSpec::Floating {
                start: day.and_hms_opt(9, 0, 0).expect("time"),
                end: None,
                source_timezone: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.by_minute = vec![0];
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.minute_text = "15,45".to_string();

        let parsed = draft.parsed_rule().expect("raw override");
        assert_eq!(parsed.by_minute, vec![15, 45]);
        assert_eq!(draft.minute_rows, vec!["0"]);
    }

    #[test]
    fn recurrence_minute_edit_rows_reject_blank_minute() {
        let rows = vec![String::new()];

        assert_eq!(
            validate_recurrence_minute_edit_rows(&rows),
            Err("BYMINUTE row #1 needs a minute or must be removed.".to_string())
        );
    }

    #[test]
    fn recurrence_preset_clears_structured_minute_rows() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Preset clears structured BYMINUTE",
            TimeSpec::Floating {
                start: day.and_hms_opt(9, 0, 0).expect("time"),
                end: None,
                source_timezone: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.by_minute = vec![0, 30];
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        assert_eq!(draft.minute_rows, vec!["0", "30"]);

        draft.apply_preset(RecurrencePreset::Daily);

        assert!(draft.minute_text.is_empty());
        assert!(draft.minute_rows.is_empty());
    }

    #[test]
    fn recurrence_second_edit_rows_roundtrip_compact_syntax() {
        let rows = recurrence_second_edit_rows(&[0, 30, 59]);

        assert_eq!(rows, vec!["0", "30", "59"]);
        assert_eq!(format_recurrence_second_edit_rows(&rows), "0,30,59");
        assert_eq!(
            parse_recurrence_second_edit_rows("0 30,59").expect("parse"),
            vec!["0", "30", "59"]
        );
    }

    #[test]
    fn recurrence_edit_draft_applies_structured_second_rows() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Structured BYSECOND",
            TimeSpec::Floating {
                start: day.and_hms_opt(9, 0, 0).expect("time"),
                end: None,
                source_timezone: None,
            },
        );
        event.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Daily));

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.second_rows = vec!["0".to_string(), "30".to_string()];
        draft.second_text = format_recurrence_second_edit_rows(&draft.second_rows);

        let parsed = draft.parsed_rule().expect("structured rule");
        assert_eq!(parsed.by_second, vec![0, 30]);
    }

    #[test]
    fn recurrence_edit_draft_raw_second_override_remains_supported() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Raw BYSECOND",
            TimeSpec::Floating {
                start: day.and_hms_opt(9, 0, 0).expect("time"),
                end: None,
                source_timezone: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.by_second = vec![0];
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.second_text = "15,45".to_string();

        let parsed = draft.parsed_rule().expect("raw override");
        assert_eq!(parsed.by_second, vec![15, 45]);
        assert_eq!(draft.second_rows, vec!["0"]);
    }

    #[test]
    fn recurrence_second_edit_rows_reject_blank_second() {
        let rows = vec![String::new()];

        assert_eq!(
            validate_recurrence_second_edit_rows(&rows),
            Err("BYSECOND row #1 needs a second or must be removed.".to_string())
        );
    }

    #[test]
    fn recurrence_preset_clears_structured_second_rows() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Preset clears structured BYSECOND",
            TimeSpec::Floating {
                start: day.and_hms_opt(9, 0, 0).expect("time"),
                end: None,
                source_timezone: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.by_second = vec![0, 30];
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        assert_eq!(draft.second_rows, vec!["0", "30"]);

        draft.apply_preset(RecurrencePreset::Daily);

        assert!(draft.second_text.is_empty());
        assert!(draft.second_rows.is_empty());
    }

    #[test]
    fn recurrence_setpos_edit_rows_roundtrip_compact_syntax() {
        let rows = recurrence_setpos_edit_rows(&[1, -1, 2]);

        assert_eq!(rows, vec!["1", "-1", "2"]);
        assert_eq!(format_recurrence_setpos_edit_rows(&rows), "1,-1,2");
        assert_eq!(
            parse_recurrence_setpos_edit_rows("1 -1,2").expect("parse"),
            vec!["1", "-1", "2"]
        );
    }

    #[test]
    fn recurrence_edit_draft_applies_structured_setpos_rows() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Structured BYSETPOS",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Monthly);
        rule.by_weekday = vec![
            RecurrenceWeekday::Monday,
            RecurrenceWeekday::Tuesday,
            RecurrenceWeekday::Wednesday,
            RecurrenceWeekday::Thursday,
            RecurrenceWeekday::Friday,
        ];
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.set_pos_rows = vec!["1".to_string(), "-1".to_string()];
        draft.set_pos_text = format_recurrence_setpos_edit_rows(&draft.set_pos_rows);

        let parsed = draft.parsed_rule().expect("structured rule");
        assert_eq!(parsed.by_set_pos, vec![1, -1]);
    }

    #[test]
    fn recurrence_edit_draft_raw_setpos_override_remains_supported() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let mut event = TemporalEvent::new(
            "Raw BYSETPOS",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Monthly);
        rule.by_weekday = vec![RecurrenceWeekday::Monday];
        rule.by_set_pos = vec![1];
        event.recurrence = Some(rule);

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.set_pos_text = "-1,2".to_string();

        let parsed = draft.parsed_rule().expect("raw override");
        assert_eq!(parsed.by_set_pos, vec![-1, 2]);
        assert_eq!(draft.set_pos_rows, vec!["1"]);
    }

    #[test]
    fn recurrence_setpos_edit_rows_reject_blank_position() {
        let rows = vec![String::new()];

        assert_eq!(
            validate_recurrence_setpos_edit_rows(&rows),
            Err("BYSETPOS row #1 needs a position or must be removed.".to_string())
        );
    }

    #[test]
    fn recurrence_preset_synchronizes_structured_setpos_rows() {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("day");
        let event = TemporalEvent::new(
            "Preset synchronizes structured BYSETPOS",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );

        let mut draft = RecurrenceEditDraft::from_event(&event);
        draft.apply_preset(RecurrencePreset::LastWeekdayOfMonth);
        assert_eq!(draft.set_pos_text, "-1");
        assert_eq!(draft.set_pos_rows, vec!["-1"]);

        draft.apply_preset(RecurrencePreset::Daily);
        assert!(draft.set_pos_text.is_empty());
        assert!(draft.set_pos_rows.is_empty());
    }
}
