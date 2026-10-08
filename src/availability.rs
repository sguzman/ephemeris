use anyhow::Context;
use chrono::{
    DateTime, Datelike, Days, Duration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone,
    Utc,
};
use chrono_tz::Tz;
use uuid::Uuid;

use crate::domain::{EventOccurrence, EventStatus, TemporalEvent, TimeSpec};

type UtcInterval = (DateTime<Utc>, DateTime<Utc>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusyKind {
    Busy,
    Tentative,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusyInterval {
    pub event_id: Uuid,
    pub occurrence_id: Uuid,
    pub event_title: String,
    pub start_utc: DateTime<Utc>,
    pub end_utc: DateTime<Utc>,
    pub kind: BusyKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreeInterval {
    pub start_utc: DateTime<Utc>,
    pub end_utc: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailabilitySkip {
    pub event_id: Uuid,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AvailabilityResult {
    pub busy: Vec<BusyInterval>,
    pub free: Vec<FreeInterval>,
    pub skipped: Vec<AvailabilitySkip>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConflictCheck {
    pub conflicts: Vec<BusyInterval>,
    pub skipped: Vec<AvailabilitySkip>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotSearch {
    pub duration_minutes: u32,
    pub step_minutes: u32,
    pub day_start: NaiveTime,
    pub day_end: NaiveTime,
    /// Monday through Sunday.
    pub workdays: [bool; 7],
}

impl SlotSearch {
    pub fn validate(self) -> anyhow::Result<Self> {
        if self.duration_minutes == 0 {
            anyhow::bail!("slot duration must be greater than zero minutes");
        }
        if self.step_minutes == 0 {
            anyhow::bail!("slot step must be greater than zero minutes");
        }
        if self.day_end <= self.day_start {
            anyhow::bail!("daily availability end must be later than start");
        }
        if !self.workdays.iter().any(|enabled| *enabled) {
            anyhow::bail!("slot search must enable at least one workday");
        }
        Ok(self)
    }
}

pub fn conflicts_for_candidate_event(
    events: &[TemporalEvent],
    candidate: &TemporalEvent,
    display_timezone: Tz,
    exclude_event_id: Option<Uuid>,
) -> anyhow::Result<ConflictCheck> {
    if !candidate.availability.blocks_time() || busy_kind(candidate.status).is_none() {
        return Ok(ConflictCheck::default());
    }
    if candidate.recurrence.is_some() {
        anyhow::bail!("conflict checking requires a non-recurring candidate event");
    }

    let occurrence = EventOccurrence {
        id: candidate.id,
        event_id: candidate.id,
        recurrence_index: None,
        origin: crate::domain::RecurrenceOccurrenceOrigin::Rule,
        original_time: candidate.time.clone(),
        time: candidate.time.clone(),
        status: candidate.status,
        override_applied: false,
        cancelled_by_override: false,
    };
    let candidate_interval = occurrence_interval_utc(&occurrence, display_timezone)
        .map_err(anyhow::Error::msg)?
        .ok_or_else(|| anyhow::anyhow!("candidate event has no blocking interval"))?;
    let (start_utc, end_utc) = candidate_interval;

    let other_events = events
        .iter()
        .filter(|event| Some(event.id) != exclude_event_id)
        .cloned()
        .collect::<Vec<_>>();
    let availability =
        availability_for_events(&other_events, display_timezone, start_utc, end_utc)?;

    Ok(ConflictCheck {
        conflicts: availability.busy,
        skipped: availability.skipped,
    })
}

/// Candidate alternatives are computed against all supplied canonical events,
/// not the caller's currently filtered calendar view. The candidate is excluded
/// by canonical UUID when editing, and the replacement must fit the original
/// definite duration without moving into hidden blocking commitments.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlternativeSlots {
    pub slots: Vec<FreeInterval>,
    pub skipped: Vec<AvailabilitySkip>,
}

pub fn alternative_slots_for_candidate(
    events: &[TemporalEvent],
    candidate: &TemporalEvent,
    display_timezone: Tz,
    exclude_event_id: Option<Uuid>,
    search: SlotSearch,
    max_suggestions: usize,
) -> anyhow::Result<AlternativeSlots> {
    if max_suggestions == 0 {
        return Ok(AlternativeSlots::default());
    }
    if candidate.recurrence.is_some() || candidate.time_uncertainty.is_some() {
        anyhow::bail!("alternative slots require a definite non-recurring event");
    }
    if !matches!(
        &candidate.time,
        TimeSpec::Instant { .. } | TimeSpec::Floating { .. }
    ) {
        anyhow::bail!("timed alternatives require an exact or floating DATE-TIME");
    }

    let occurrence = EventOccurrence {
        id: candidate.id,
        event_id: candidate.id,
        recurrence_index: None,
        origin: crate::domain::RecurrenceOccurrenceOrigin::Rule,
        original_time: candidate.time.clone(),
        time: candidate.time.clone(),
        status: candidate.status,
        override_applied: false,
        cancelled_by_override: false,
    };
    let (candidate_start, candidate_end) = occurrence_interval_utc(&occurrence, display_timezone)
        .map_err(anyhow::Error::msg)?
        .ok_or_else(|| anyhow::anyhow!("candidate has no definite duration"))?;
    let duration_minutes = (candidate_end - candidate_start).num_minutes();
    if duration_minutes <= 0
        || candidate_start + Duration::minutes(duration_minutes) != candidate_end
    {
        anyhow::bail!("timed alternatives require whole-minute positive duration");
    }
    let duration_minutes =
        u32::try_from(duration_minutes).context("candidate duration exceeds slot search limits")?;

    let first_date = candidate_start
        .with_timezone(&display_timezone)
        .date_naive();
    let last_date = first_date
        .checked_add_days(Days::new(15))
        .ok_or_else(|| anyhow::anyhow!("alternative search date overflow"))?;
    let start_utc = resolve_local(
        display_timezone,
        first_date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| anyhow::anyhow!("invalid alternative search start"))?,
    )
    .ok_or_else(|| anyhow::anyhow!("alternative search start is not representable"))?;
    let end_utc = resolve_local(
        display_timezone,
        last_date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| anyhow::anyhow!("invalid alternative search end"))?,
    )
    .ok_or_else(|| anyhow::anyhow!("alternative search end is not representable"))?;

    let other_events = events
        .iter()
        .filter(|event| Some(event.id) != exclude_event_id)
        .cloned()
        .collect::<Vec<_>>();
    let availability =
        availability_for_events(&other_events, display_timezone, start_utc, end_utc)?;
    let slots = suggest_slots(
        &availability.free,
        display_timezone,
        first_date,
        last_date,
        SlotSearch {
            duration_minutes,
            ..search
        },
    )?
    .into_iter()
    .filter(|slot| slot.start_utc > candidate_start)
    .take(max_suggestions)
    .collect();

    Ok(AlternativeSlots {
        slots,
        skipped: availability.skipped,
    })
}

pub fn availability_for_materialized_date_window(
    events: &[TemporalEvent],
    display_timezone: Tz,
    start_date: NaiveDate,
    end_exclusive: NaiveDate,
) -> anyhow::Result<AvailabilityResult> {
    let start_local = start_date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow::anyhow!("invalid availability window start date"))?;
    let end_local = end_exclusive
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow::anyhow!("invalid availability window end date"))?;
    let window_start_utc = resolve_local(display_timezone, start_local).ok_or_else(|| {
        anyhow::anyhow!(
            "availability window start midnight is not representable in {display_timezone}"
        )
    })?;
    let window_end_utc = resolve_local(display_timezone, end_local).ok_or_else(|| {
        anyhow::anyhow!(
            "availability window end midnight is not representable in {display_timezone}"
        )
    })?;

    availability_for_materialized_events(events, display_timezone, window_start_utc, window_end_utc)
}

#[derive(Debug, Clone, Copy)]
struct AvailabilityWindow {
    display_timezone: Tz,
    start_utc: DateTime<Utc>,
    end_utc: DateTime<Utc>,
}

pub fn availability_for_materialized_events(
    events: &[TemporalEvent],
    display_timezone: Tz,
    window_start_utc: DateTime<Utc>,
    window_end_utc: DateTime<Utc>,
) -> anyhow::Result<AvailabilityResult> {
    if window_end_utc <= window_start_utc {
        return Ok(AvailabilityResult::default());
    }

    let window = AvailabilityWindow {
        display_timezone,
        start_utc: window_start_utc,
        end_utc: window_end_utc,
    };
    let mut result = AvailabilityResult::default();
    for event in events {
        if skip_uncertain_blocker(event, &mut result) {
            continue;
        }
        let occurrence = EventOccurrence {
            id: event.id,
            event_id: event.id,
            recurrence_index: None,
            origin: crate::domain::RecurrenceOccurrenceOrigin::Rule,
            original_time: event.time.clone(),
            time: event.time.clone(),
            status: event.status,
            override_applied: false,
            cancelled_by_override: false,
        };
        append_occurrence(
            &mut result,
            event.id,
            &event.normalized_title,
            event.availability.blocks_time(),
            &occurrence,
            window,
        );
    }
    finish_availability(result, window_start_utc, window_end_utc)
}

pub fn availability_for_events(
    events: &[TemporalEvent],
    display_timezone: Tz,
    window_start_utc: DateTime<Utc>,
    window_end_utc: DateTime<Utc>,
) -> anyhow::Result<AvailabilityResult> {
    if window_end_utc <= window_start_utc {
        return Ok(AvailabilityResult::default());
    }

    let start_date = window_start_utc
        .with_timezone(&display_timezone)
        .date_naive()
        .checked_sub_days(Days::new(1))
        .unwrap_or(NaiveDate::MIN);
    let end_exclusive = window_end_utc
        .with_timezone(&display_timezone)
        .date_naive()
        .checked_add_days(Days::new(2))
        .unwrap_or(NaiveDate::MAX);

    let window = AvailabilityWindow {
        display_timezone,
        start_utc: window_start_utc,
        end_utc: window_end_utc,
    };
    let mut result = AvailabilityResult::default();
    for event in events {
        if event.time_uncertainty.is_some()
            && potential_uncertain_busy_overlap(
                event,
                display_timezone,
                window_start_utc,
                window_end_utc,
            )
            && skip_uncertain_blocker(event, &mut result)
        {
            continue;
        }

        let occurrences = event
            .occurrences_in_window(start_date, end_exclusive, display_timezone)
            .map_err(|error| {
                anyhow::anyhow!("failed to expand availability event {}: {error}", event.id)
            })?;

        for occurrence in occurrences {
            append_occurrence(
                &mut result,
                event.id,
                &event.normalized_title,
                event.availability.blocks_time(),
                &occurrence,
                window,
            );
        }
    }

    finish_availability(result, window_start_utc, window_end_utc)
}

/// A representative event start is only one possible placement when an
/// uncertainty window is present. Check whether any plausible placement could
/// intersect the requested availability window, even if the representative
/// start falls outside the materialization range. Timezone conversion is
/// conservative around civil-clock jumps, and unresolvable coordinates remain
/// potentially relevant rather than becoming false "free" assertions.
fn potential_uncertain_busy_overlap(
    event: &TemporalEvent,
    display_timezone: Tz,
    window_start_utc: DateTime<Utc>,
    window_end_utc: DateTime<Utc>,
) -> bool {
    let Some(uncertainty) = event.time_uncertainty.as_ref() else {
        return false;
    };

    let (earliest, latest, civil) = match uncertainty {
        crate::domain::TimeUncertainty::InstantWindow {
            earliest_utc,
            latest_utc,
        } => (*earliest_utc, *latest_utc, false),
        crate::domain::TimeUncertainty::FloatingWindow { earliest, latest } => {
            let timezone = match &event.time {
                TimeSpec::Floating {
                    source_timezone: Some(raw),
                    ..
                } => match raw.parse::<Tz>() {
                    Ok(timezone) => timezone,
                    Err(_) => return true,
                },
                _ => display_timezone,
            };
            let (Some(first), Some(last)) = (
                resolve_local(timezone, *earliest),
                resolve_local(timezone, *latest),
            ) else {
                return true;
            };
            (first, last, true)
        }
        crate::domain::TimeUncertainty::DateWindow { earliest, latest } => {
            let (Some(first), Some(last)) =
                (earliest.and_hms_opt(0, 0, 0), latest.and_hms_opt(0, 0, 0))
            else {
                return true;
            };
            let (Some(first), Some(last)) = (
                resolve_local(display_timezone, first),
                resolve_local(display_timezone, last),
            ) else {
                return true;
            };
            (first, last, true)
        }
    };

    let duration = match &event.time {
        TimeSpec::Instant {
            start_utc,
            end_utc: Some(end_utc),
            ..
        } => *end_utc - *start_utc,
        TimeSpec::Floating {
            start,
            end: Some(end),
            ..
        } => *end - *start,
        TimeSpec::AllDay {
            start,
            end_exclusive,
        }
        | TimeSpec::DateOnly {
            start,
            end_exclusive,
        } => Duration::days(
            end_exclusive.map_or(1, |end| end.signed_duration_since(*start).num_days()),
        ),
        _ => return true,
    };
    if duration <= Duration::zero() {
        return true;
    }

    // Civil coordinates can straddle DST or historical offset changes.
    // Two days of slack avoids converting a wall-clock uncertainty into a
    // falsely definite absence around those transitions.
    let slack = if civil {
        Duration::days(2)
    } else {
        Duration::zero()
    };
    let Some(possible_start) = earliest.checked_sub_signed(slack) else {
        return true;
    };
    let Some(possible_end) = latest
        .checked_add_signed(duration)
        .and_then(|value| value.checked_add_signed(slack))
    else {
        return true;
    };
    possible_start < window_end_utc && possible_end > window_start_utc
}

fn skip_uncertain_blocker(event: &TemporalEvent, result: &mut AvailabilityResult) -> bool {
    if event.time_uncertainty.is_none()
        || !event.availability.blocks_time()
        || busy_kind(event.status).is_none()
    {
        return false;
    }
    result.skipped.push(AvailabilitySkip {
        event_id: event.id,
        reason: "event has bounded start-placement uncertainty; its representative time is not a definite busy interval".to_string(),
    });
    true
}

fn append_occurrence(
    result: &mut AvailabilityResult,
    event_id: Uuid,
    event_title: &str,
    blocks_time: bool,
    occurrence: &EventOccurrence,
    window: AvailabilityWindow,
) {
    if !blocks_time {
        return;
    }
    let Some(kind) = busy_kind(occurrence.status) else {
        return;
    };
    match occurrence_interval_utc(occurrence, window.display_timezone) {
        Ok(Some((start_utc, end_utc))) => {
            let clipped_start = start_utc.max(window.start_utc);
            let clipped_end = end_utc.min(window.end_utc);
            if clipped_end > clipped_start {
                result.busy.push(BusyInterval {
                    event_id,
                    occurrence_id: occurrence.id,
                    event_title: event_title.to_string(),
                    start_utc: clipped_start,
                    end_utc: clipped_end,
                    kind,
                });
            }
        }
        Ok(None) => {}
        Err(reason) => result.skipped.push(AvailabilitySkip { event_id, reason }),
    }
}

fn finish_availability(
    mut result: AvailabilityResult,
    window_start_utc: DateTime<Utc>,
    window_end_utc: DateTime<Utc>,
) -> anyhow::Result<AvailabilityResult> {
    result.busy.sort_by_key(|interval| {
        (
            interval.start_utc,
            interval.end_utc,
            interval.event_title.clone(),
            interval.occurrence_id,
        )
    });
    result
        .skipped
        .sort_by_key(|skip| (skip.event_id, skip.reason.clone()));
    result.skipped.dedup();
    result.free = free_intervals_from_busy(&result.busy, window_start_utc, window_end_utc);
    Ok(result)
}

pub fn suggest_slots(
    free: &[FreeInterval],
    timezone: Tz,
    start_date: NaiveDate,
    end_exclusive: NaiveDate,
    search: SlotSearch,
) -> anyhow::Result<Vec<FreeInterval>> {
    let search = search.validate()?;
    if end_exclusive <= start_date {
        return Ok(Vec::new());
    }

    let duration = Duration::minutes(i64::from(search.duration_minutes));
    let step = i64::from(search.step_minutes);
    let mut slots = Vec::new();
    let mut date = start_date;

    while date < end_exclusive {
        let weekday_index = usize::try_from(date.weekday().num_days_from_monday())
            .expect("weekday index is within 0..=6");
        if !search.workdays[weekday_index] {
            date = date
                .checked_add_days(Days::new(1))
                .ok_or_else(|| anyhow::anyhow!("slot-search date range overflow"))?;
            continue;
        }

        let day_start_local = date.and_time(search.day_start);
        let day_end_local = date.and_time(search.day_end);
        let day_start_utc = resolve_local(timezone, day_start_local).ok_or_else(|| {
            anyhow::anyhow!(
                "daily availability start {} is not representable in {timezone}",
                day_start_local
            )
        })?;
        let day_end_utc = resolve_local(timezone, day_end_local).ok_or_else(|| {
            anyhow::anyhow!(
                "daily availability end {} is not representable in {timezone}",
                day_end_local
            )
        })?;
        if day_end_utc <= day_start_utc {
            anyhow::bail!("daily availability window resolves to a non-positive UTC duration");
        }

        for interval in free {
            let free_start = interval.start_utc.max(day_start_utc);
            let free_end = interval.end_utc.min(day_end_utc);
            if free_end <= free_start {
                continue;
            }

            let offset_minutes = (free_start - day_start_utc).num_minutes().max(0);
            let aligned_steps = (offset_minutes + step - 1) / step;
            let mut candidate =
                day_start_utc + Duration::minutes(aligned_steps.saturating_mul(step));

            while candidate + duration <= free_end {
                slots.push(FreeInterval {
                    start_utc: candidate,
                    end_utc: candidate + duration,
                });
                candidate += Duration::minutes(step);
            }
        }

        date = date
            .checked_add_days(Days::new(1))
            .ok_or_else(|| anyhow::anyhow!("slot-search date range overflow"))?;
    }

    slots.sort_by_key(|slot| (slot.start_utc, slot.end_utc));
    slots.dedup();
    Ok(slots)
}

pub fn free_intervals_from_busy(
    busy: &[BusyInterval],
    window_start_utc: DateTime<Utc>,
    window_end_utc: DateTime<Utc>,
) -> Vec<FreeInterval> {
    if window_end_utc <= window_start_utc {
        return Vec::new();
    }

    let mut ranges = busy
        .iter()
        .filter_map(|interval| {
            let start = interval.start_utc.max(window_start_utc);
            let end = interval.end_utc.min(window_end_utc);
            (end > start).then_some((start, end))
        })
        .collect::<Vec<_>>();
    ranges.sort_by_key(|(start, end)| (*start, *end));

    let mut merged: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::new();
    for (start, end) in ranges {
        if let Some((_, current_end)) = merged.last_mut()
            && start <= *current_end
        {
            if end > *current_end {
                *current_end = end;
            }
            continue;
        }
        merged.push((start, end));
    }

    let mut free = Vec::new();
    let mut cursor = window_start_utc;
    for (start, end) in merged {
        if start > cursor {
            free.push(FreeInterval {
                start_utc: cursor,
                end_utc: start,
            });
        }
        if end > cursor {
            cursor = end;
        }
    }
    if cursor < window_end_utc {
        free.push(FreeInterval {
            start_utc: cursor,
            end_utc: window_end_utc,
        });
    }
    free
}

fn busy_kind(status: EventStatus) -> Option<BusyKind> {
    match status {
        EventStatus::Cancelled | EventStatus::Postponed | EventStatus::Superseded => None,
        EventStatus::Tentative
        | EventStatus::Announced
        | EventStatus::Estimated
        | EventStatus::Projected
        | EventStatus::Disputed
        | EventStatus::Unknown => Some(BusyKind::Tentative),
        EventStatus::Scheduled
        | EventStatus::Confirmed
        | EventStatus::Rescheduled
        | EventStatus::Completed
        | EventStatus::Observed => Some(BusyKind::Busy),
    }
}

fn occurrence_interval_utc(
    occurrence: &EventOccurrence,
    display_timezone: Tz,
) -> Result<Option<UtcInterval>, String> {
    if occurrence.cancelled_by_override {
        return Ok(None);
    }

    match &occurrence.time {
        TimeSpec::Instant {
            start_utc, end_utc, ..
        } => {
            let Some(end_utc) = end_utc else {
                return Err(
                    "exact instant has no end, so free/busy duration is undefined".to_string(),
                );
            };
            if end_utc <= start_utc {
                return Err("exact instant has a non-positive duration".to_string());
            }
            Ok(Some((*start_utc, *end_utc)))
        }
        TimeSpec::Floating {
            start,
            end,
            source_timezone,
        } => {
            let Some(end) = end else {
                return Err(
                    "floating event has no end, so free/busy duration is undefined".to_string(),
                );
            };
            if end <= start {
                return Err("floating event has a non-positive duration".to_string());
            }
            let timezone = match source_timezone.as_deref() {
                Some(raw) => raw
                    .parse::<Tz>()
                    .map_err(|_| format!("floating event has invalid source timezone {raw:?}"))?,
                None => display_timezone,
            };
            let start_utc = resolve_local(timezone, *start).ok_or_else(|| {
                "floating start falls in a nonexistent local interval".to_string()
            })?;
            let end_utc = resolve_local(timezone, *end)
                .ok_or_else(|| "floating end falls in a nonexistent local interval".to_string())?;
            if end_utc <= start_utc {
                return Err("floating event resolves to a non-positive UTC duration".to_string());
            }
            Ok(Some((start_utc, end_utc)))
        }
        TimeSpec::AllDay {
            start,
            end_exclusive,
        } => {
            let end_date = match end_exclusive {
                Some(value) => *value,
                None => start.checked_add_days(Days::new(1)).ok_or_else(|| {
                    "all-day event end overflows supported date range".to_string()
                })?,
            };
            if end_date <= *start {
                return Err("all-day event has a non-positive duration".to_string());
            }
            let start_local = start
                .and_hms_opt(0, 0, 0)
                .ok_or_else(|| "invalid all-day start".to_string())?;
            let end_local = end_date
                .and_hms_opt(0, 0, 0)
                .ok_or_else(|| "invalid all-day end".to_string())?;
            let start_utc = resolve_local(display_timezone, start_local).ok_or_else(|| {
                "all-day start midnight is not representable in display timezone".to_string()
            })?;
            let end_utc = resolve_local(display_timezone, end_local).ok_or_else(|| {
                "all-day end midnight is not representable in display timezone".to_string()
            })?;
            if end_utc <= start_utc {
                return Err("all-day event resolves to a non-positive UTC duration".to_string());
            }
            Ok(Some((start_utc, end_utc)))
        }
        TimeSpec::DateOnly { .. } => Err(
            "date-only value is not an all-day commitment and cannot block free/busy".to_string(),
        ),
        TimeSpec::Month { .. } | TimeSpec::Year { .. } | TimeSpec::Unknown { .. } => Err(format!(
            "{} precision cannot block free/busy without a concrete interval",
            occurrence.time.kind_name()
        )),
    }
}

fn resolve_local(timezone: Tz, local: NaiveDateTime) -> Option<DateTime<Utc>> {
    match timezone.from_local_datetime(&local) {
        LocalResult::Single(value) => Some(value.with_timezone(&Utc)),
        LocalResult::Ambiguous(first, second) => Some(first.min(second).with_timezone(&Utc)),
        LocalResult::None => None,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, NaiveDate};

    use super::*;
    use crate::domain::{RecurrenceFrequency, RecurrenceOverride, RecurrenceRule};

    #[test]
    fn uncertainty_window_reports_possible_overlap_beyond_representative_start() {
        let anchor = Utc
            .with_ymd_and_hms(2026, 10, 8, 10, 0, 0)
            .single()
            .expect("anchor");
        let mut event = TemporalEvent::new(
            "Variable appointment",
            TimeSpec::Instant {
                start_utc: anchor,
                end_utc: Some(anchor + Duration::hours(1)),
                source_timezone: None,
            },
        );
        event.time_uncertainty = Some(crate::domain::TimeUncertainty::InstantWindow {
            earliest_utc: anchor - Duration::hours(2),
            latest_utc: anchor + Duration::hours(2),
        });

        let early = availability_for_events(
            &[event.clone()],
            chrono_tz::UTC,
            anchor - Duration::hours(2),
            anchor - Duration::hours(1),
        )
        .expect("possible early placement");
        assert!(early.busy.is_empty());
        assert_eq!(early.skipped.len(), 1);

        let distant = availability_for_events(
            &[event],
            chrono_tz::UTC,
            anchor + Duration::days(20),
            anchor + Duration::days(21),
        )
        .expect("distant window");
        assert!(distant.skipped.is_empty());
    }

    #[test]
    fn uncertain_event_is_skipped_instead_of_asserting_definite_busy_time() {
        let start = Utc
            .with_ymd_and_hms(2026, 10, 8, 9, 0, 0)
            .single()
            .expect("start");
        let mut uncertain = TemporalEvent::new(
            "Possible appointment",
            TimeSpec::Instant {
                start_utc: start,
                end_utc: Some(start + Duration::hours(1)),
                source_timezone: None,
            },
        );
        uncertain.time_uncertainty = Some(crate::domain::TimeUncertainty::InstantWindow {
            earliest_utc: start - Duration::minutes(30),
            latest_utc: start + Duration::minutes(30),
        });

        let full = availability_for_events(
            &[uncertain.clone()],
            chrono_tz::UTC,
            start - Duration::hours(2),
            start + Duration::hours(3),
        )
        .expect("full availability");
        assert!(full.busy.is_empty());
        assert_eq!(full.skipped.len(), 1);
        assert!(full.skipped[0].reason.contains("uncertainty"));

        let materialized = availability_for_materialized_events(
            &[uncertain],
            chrono_tz::UTC,
            start - Duration::hours(2),
            start + Duration::hours(3),
        )
        .expect("materialized availability");
        assert!(materialized.busy.is_empty());
        assert_eq!(materialized.skipped.len(), 1);
    }

    #[test]
    fn alternative_slots_exclude_hidden_blockers_and_preserve_duration() {
        let timezone = chrono_tz::UTC;
        let start = timezone
            .with_ymd_and_hms(2026, 10, 8, 9, 0, 0)
            .single()
            .expect("start")
            .with_timezone(&Utc);
        let candidate = TemporalEvent::new(
            "Candidate",
            TimeSpec::Instant {
                start_utc: start,
                end_utc: Some(start + Duration::minutes(30)),
                source_timezone: None,
            },
        );
        let first_block = TemporalEvent::new(
            "Already booked",
            TimeSpec::Instant {
                start_utc: start,
                end_utc: Some(start + Duration::hours(1)),
                source_timezone: None,
            },
        );
        let later_block = TemporalEvent::new(
            "Hidden by saved view",
            TimeSpec::Instant {
                start_utc: start + Duration::hours(1),
                end_utc: Some(start + Duration::hours(2)),
                source_timezone: None,
            },
        );
        let preferences = SlotSearch {
            duration_minutes: 5, // candidate duration must take precedence
            step_minutes: 30,
            day_start: NaiveTime::from_hms_opt(9, 0, 0).expect("start"),
            day_end: NaiveTime::from_hms_opt(13, 0, 0).expect("end"),
            workdays: [true; 7],
        };
        let alternatives = alternative_slots_for_candidate(
            &[first_block, later_block],
            &candidate,
            timezone,
            None,
            preferences,
            3,
        )
        .expect("alternatives");

        assert_eq!(alternatives.slots.len(), 3);
        assert_eq!(alternatives.slots[0].start_utc, start + Duration::hours(2));
        assert_eq!(
            alternatives.slots[0].end_utc - alternatives.slots[0].start_utc,
            Duration::minutes(30)
        );
        assert_eq!(
            alternatives.slots[1].start_utc,
            start + Duration::minutes(150)
        );
        assert!(alternatives.skipped.is_empty());
    }

    #[test]
    fn alternative_slots_exclude_event_being_rescheduled() {
        let start = Utc
            .with_ymd_and_hms(2026, 10, 8, 9, 0, 0)
            .single()
            .expect("start");
        let mut event = TemporalEvent::new(
            "Rescheduled",
            TimeSpec::Instant {
                start_utc: start,
                end_utc: Some(start + Duration::hours(1)),
                source_timezone: None,
            },
        );
        let candidate = event.clone();
        event.time = TimeSpec::Instant {
            start_utc: start + Duration::hours(1),
            end_utc: Some(start + Duration::hours(2)),
            source_timezone: None,
        };
        let blocker = TemporalEvent::new(
            "Conflict",
            TimeSpec::Instant {
                start_utc: start + Duration::hours(1),
                end_utc: Some(start + Duration::hours(2)),
                source_timezone: None,
            },
        );
        let preferences = SlotSearch {
            duration_minutes: 60,
            step_minutes: 30,
            day_start: NaiveTime::from_hms_opt(9, 0, 0).expect("start"),
            day_end: NaiveTime::from_hms_opt(13, 0, 0).expect("end"),
            workdays: [true; 7],
        };
        let results = alternative_slots_for_candidate(
            &[event, blocker],
            &candidate,
            chrono_tz::UTC,
            Some(candidate.id),
            preferences,
            2,
        )
        .expect("alternatives");
        assert_eq!(results.slots[0].start_utc, start + Duration::hours(2));
    }

    #[test]
    fn alternative_slots_refuse_imprecise_candidates() {
        let event = TemporalEvent::new(
            "Date-only",
            TimeSpec::DateOnly {
                start: NaiveDate::from_ymd_opt(2026, 10, 8).expect("date"),
                end_exclusive: None,
            },
        );
        let search = SlotSearch {
            duration_minutes: 30,
            step_minutes: 30,
            day_start: NaiveTime::from_hms_opt(9, 0, 0).expect("start"),
            day_end: NaiveTime::from_hms_opt(17, 0, 0).expect("end"),
            workdays: [true; 7],
        };
        assert!(
            alternative_slots_for_candidate(&[], &event, chrono_tz::UTC, None, search, 3).is_err()
        );
    }

    #[test]
    fn conflict_check_finds_overlapping_busy_events_and_ignores_free_or_excluded() {
        let start = Utc
            .with_ymd_and_hms(2026, 10, 8, 15, 0, 0)
            .single()
            .expect("start");
        let mut candidate = TemporalEvent::new(
            "Candidate",
            TimeSpec::Instant {
                start_utc: start,
                end_utc: Some(start + Duration::hours(1)),
                source_timezone: None,
            },
        );

        let busy = TemporalEvent::new(
            "Existing busy",
            TimeSpec::Instant {
                start_utc: start + Duration::minutes(30),
                end_utc: Some(start + Duration::minutes(90)),
                source_timezone: None,
            },
        );
        let mut free = TemporalEvent::new(
            "Existing free",
            TimeSpec::Instant {
                start_utc: start + Duration::minutes(15),
                end_utc: Some(start + Duration::minutes(45)),
                source_timezone: None,
            },
        );
        free.availability = crate::domain::AvailabilityBehavior::Free;
        let excluded = TemporalEvent::new(
            "Edited event",
            TimeSpec::Instant {
                start_utc: start,
                end_utc: Some(start + Duration::hours(1)),
                source_timezone: None,
            },
        );

        let result = conflicts_for_candidate_event(
            &[busy.clone(), free, excluded.clone()],
            &candidate,
            chrono_tz::UTC,
            Some(excluded.id),
        )
        .expect("conflicts");
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].event_id, busy.id);
        assert!(result.skipped.is_empty());

        candidate.availability = crate::domain::AvailabilityBehavior::Free;
        assert!(
            conflicts_for_candidate_event(&[busy], &candidate, chrono_tz::UTC, Some(excluded.id))
                .expect("free candidate")
                .conflicts
                .is_empty()
        );
    }

    #[test]
    fn conflict_check_expands_recurring_existing_events() {
        let start = Utc
            .with_ymd_and_hms(2026, 10, 8, 15, 0, 0)
            .single()
            .expect("start");
        let mut recurring = TemporalEvent::new(
            "Daily busy",
            TimeSpec::Instant {
                start_utc: start,
                end_utc: Some(start + Duration::hours(1)),
                source_timezone: None,
            },
        );
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.count = Some(2);
        recurring.recurrence = Some(rule);

        let candidate = TemporalEvent::new(
            "Tomorrow conflict",
            TimeSpec::Instant {
                start_utc: start + Duration::days(1) + Duration::minutes(15),
                end_utc: Some(start + Duration::days(1) + Duration::minutes(45)),
                source_timezone: None,
            },
        );

        let result = conflicts_for_candidate_event(&[recurring], &candidate, chrono_tz::UTC, None)
            .expect("conflicts");
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].event_title, "Daily busy");
    }

    #[test]
    fn conflict_check_rejects_candidate_without_concrete_busy_interval() {
        let candidate = TemporalEvent::new(
            "Date only",
            TimeSpec::DateOnly {
                start: NaiveDate::from_ymd_opt(2026, 10, 8).expect("date"),
                end_exclusive: None,
            },
        );
        assert!(conflicts_for_candidate_event(&[], &candidate, chrono_tz::UTC, None).is_err());
    }

    #[test]
    fn materialized_events_do_not_reexpand_attached_recurrence() {
        let start = Utc
            .with_ymd_and_hms(2026, 10, 8, 15, 0, 0)
            .single()
            .expect("start");
        let mut event = TemporalEvent::new(
            "Materialized occurrence",
            TimeSpec::Instant {
                start_utc: start,
                end_utc: Some(start + Duration::hours(1)),
                source_timezone: None,
            },
        );
        let mut recurrence = RecurrenceRule::new(RecurrenceFrequency::Daily);
        recurrence.count = Some(3);
        event.recurrence = Some(recurrence);

        let result = availability_for_materialized_events(
            &[event],
            chrono_tz::UTC,
            start - Duration::hours(1),
            start + Duration::days(4),
        )
        .expect("availability");

        assert_eq!(result.busy.len(), 1);
        assert_eq!(result.busy[0].start_utc, start);
    }

    #[test]
    fn materialized_date_window_uses_display_timezone_midnights() {
        let timezone = chrono_tz::America::Mexico_City;
        let day = NaiveDate::from_ymd_opt(2026, 10, 8).expect("day");
        let next_day = day.checked_add_days(Days::new(1)).expect("next day");
        let event = TemporalEvent::new(
            "All day",
            TimeSpec::AllDay {
                start: day,
                end_exclusive: None,
            },
        );

        let result = availability_for_materialized_date_window(&[event], timezone, day, next_day)
            .expect("availability");

        assert_eq!(result.busy.len(), 1);
        assert!(result.free.is_empty());
    }

    #[test]
    fn slot_search_aligns_candidates_and_respects_busy_gaps() {
        let timezone = chrono_tz::UTC;
        let day = NaiveDate::from_ymd_opt(2026, 10, 8).expect("day");
        let work_start = Utc
            .with_ymd_and_hms(2026, 10, 8, 9, 0, 0)
            .single()
            .expect("work start");
        let free = vec![
            FreeInterval {
                start_utc: work_start,
                end_utc: work_start + Duration::hours(1),
            },
            FreeInterval {
                start_utc: work_start + Duration::hours(2),
                end_utc: work_start + Duration::hours(4),
            },
        ];
        let slots = suggest_slots(
            &free,
            timezone,
            day,
            day.checked_add_days(Days::new(1)).expect("next day"),
            SlotSearch {
                duration_minutes: 60,
                step_minutes: 30,
                day_start: NaiveTime::from_hms_opt(9, 0, 0).expect("start"),
                day_end: NaiveTime::from_hms_opt(17, 0, 0).expect("end"),
                workdays: [true; 7],
            },
        )
        .expect("slots");

        assert_eq!(
            slots,
            vec![
                FreeInterval {
                    start_utc: work_start,
                    end_utc: work_start + Duration::hours(1),
                },
                FreeInterval {
                    start_utc: work_start + Duration::hours(2),
                    end_utc: work_start + Duration::hours(3),
                },
                FreeInterval {
                    start_utc: work_start + Duration::hours(2) + Duration::minutes(30),
                    end_utc: work_start + Duration::hours(3) + Duration::minutes(30),
                },
                FreeInterval {
                    start_utc: work_start + Duration::hours(3),
                    end_utc: work_start + Duration::hours(4),
                },
            ]
        );
    }

    #[test]
    fn slot_search_rounds_forward_from_partial_step() {
        let timezone = chrono_tz::UTC;
        let day = NaiveDate::from_ymd_opt(2026, 10, 8).expect("day");
        let free_start = Utc
            .with_ymd_and_hms(2026, 10, 8, 9, 10, 0)
            .single()
            .expect("free start");
        let slots = suggest_slots(
            &[FreeInterval {
                start_utc: free_start,
                end_utc: free_start + Duration::hours(2),
            }],
            timezone,
            day,
            day.checked_add_days(Days::new(1)).expect("next day"),
            SlotSearch {
                duration_minutes: 30,
                step_minutes: 30,
                day_start: NaiveTime::from_hms_opt(9, 0, 0).expect("start"),
                day_end: NaiveTime::from_hms_opt(17, 0, 0).expect("end"),
                workdays: [true; 7],
            },
        )
        .expect("slots");

        assert_eq!(
            slots.first().map(|slot| slot.start_utc),
            Some(
                Utc.with_ymd_and_hms(2026, 10, 8, 9, 30, 0)
                    .single()
                    .expect("first slot")
            )
        );
    }

    #[test]
    fn slot_search_skips_disabled_weekdays() {
        let timezone = chrono_tz::UTC;
        let friday = NaiveDate::from_ymd_opt(2026, 10, 9).expect("Friday");
        let monday = NaiveDate::from_ymd_opt(2026, 10, 12).expect("Monday");
        let start = Utc
            .with_ymd_and_hms(2026, 10, 9, 9, 0, 0)
            .single()
            .expect("start");
        let end = Utc
            .with_ymd_and_hms(2026, 10, 13, 17, 0, 0)
            .single()
            .expect("end");
        let slots = suggest_slots(
            &[FreeInterval {
                start_utc: start,
                end_utc: end,
            }],
            timezone,
            friday,
            NaiveDate::from_ymd_opt(2026, 10, 14).expect("end date"),
            SlotSearch {
                duration_minutes: 60,
                step_minutes: 60,
                day_start: NaiveTime::from_hms_opt(9, 0, 0).expect("start"),
                day_end: NaiveTime::from_hms_opt(10, 0, 0).expect("end"),
                workdays: [true, true, true, true, true, false, false],
            },
        )
        .expect("slots");

        assert_eq!(slots.len(), 3);
        assert_eq!(slots[0].start_utc.date_naive(), friday);
        assert_eq!(slots[1].start_utc.date_naive(), monday);
        assert_eq!(
            slots[2].start_utc.date_naive(),
            monday.checked_add_days(Days::new(1)).expect("Tuesday")
        );
    }

    #[test]
    fn slot_search_rejects_invalid_constraints() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 8).expect("day");
        let next = day.checked_add_days(Days::new(1)).expect("next day");
        let nine = NaiveTime::from_hms_opt(9, 0, 0).expect("nine");
        let five = NaiveTime::from_hms_opt(17, 0, 0).expect("five");

        assert!(
            suggest_slots(
                &[],
                chrono_tz::UTC,
                day,
                next,
                SlotSearch {
                    duration_minutes: 0,
                    step_minutes: 30,
                    day_start: nine,
                    day_end: five,
                    workdays: [true; 7],
                },
            )
            .is_err()
        );
        assert!(
            suggest_slots(
                &[],
                chrono_tz::UTC,
                day,
                next,
                SlotSearch {
                    duration_minutes: 30,
                    step_minutes: 0,
                    day_start: nine,
                    day_end: five,
                    workdays: [true; 7],
                },
            )
            .is_err()
        );
        assert!(
            suggest_slots(
                &[],
                chrono_tz::UTC,
                day,
                next,
                SlotSearch {
                    duration_minutes: 30,
                    step_minutes: 30,
                    day_start: five,
                    day_end: nine,
                    workdays: [true; 7],
                },
            )
            .is_err()
        );
    }

    #[test]
    fn free_availability_events_do_not_block_time_even_when_tentative() {
        let start = Utc
            .with_ymd_and_hms(2026, 10, 8, 15, 0, 0)
            .single()
            .expect("start");
        let mut event = TemporalEvent::new(
            "FYI event",
            TimeSpec::Instant {
                start_utc: start,
                end_utc: Some(start + Duration::hours(1)),
                source_timezone: None,
            },
        );
        event.status = EventStatus::Tentative;
        event.availability = crate::domain::AvailabilityBehavior::Free;

        let result = availability_for_events(
            &[event],
            chrono_tz::UTC,
            start - Duration::hours(1),
            start + Duration::hours(2),
        )
        .expect("availability");

        assert!(result.busy.is_empty());
        assert_eq!(result.free.len(), 1);
        assert_eq!(result.free[0].start_utc, start - Duration::hours(1));
        assert_eq!(result.free[0].end_utc, start + Duration::hours(2));
    }

    #[test]
    fn exact_intervals_merge_into_free_gaps() {
        let window_start = Utc
            .with_ymd_and_hms(2026, 10, 8, 14, 0, 0)
            .single()
            .expect("window start");
        let window_end = window_start + Duration::hours(6);

        let mut first = TemporalEvent::new(
            "First",
            TimeSpec::Instant {
                start_utc: window_start + Duration::hours(1),
                end_utc: Some(window_start + Duration::hours(2)),
                source_timezone: None,
            },
        );
        first.status = EventStatus::Confirmed;
        let second = TemporalEvent::new(
            "Second",
            TimeSpec::Instant {
                start_utc: window_start + Duration::minutes(90),
                end_utc: Some(window_start + Duration::hours(3)),
                source_timezone: None,
            },
        );

        let result = availability_for_events(
            &[first, second],
            chrono_tz::America::Mexico_City,
            window_start,
            window_end,
        )
        .expect("availability");

        assert_eq!(result.busy.len(), 2);
        assert_eq!(
            result.free,
            vec![
                FreeInterval {
                    start_utc: window_start,
                    end_utc: window_start + Duration::hours(1),
                },
                FreeInterval {
                    start_utc: window_start + Duration::hours(3),
                    end_utc: window_end,
                },
            ]
        );
    }

    #[test]
    fn recurrence_and_cancelled_override_affect_busy_time() {
        let start = Utc
            .with_ymd_and_hms(2026, 10, 8, 15, 0, 0)
            .single()
            .expect("start");
        let mut event = TemporalEvent::new(
            "Daily",
            TimeSpec::Instant {
                start_utc: start,
                end_utc: Some(start + Duration::hours(1)),
                source_timezone: None,
            },
        );
        let mut recurrence = RecurrenceRule::new(RecurrenceFrequency::Daily);
        recurrence.count = Some(2);
        recurrence.overrides.push(RecurrenceOverride {
            original: TimeSpec::Instant {
                start_utc: start + Duration::days(1),
                end_utc: Some(start + Duration::days(1) + Duration::hours(1)),
                source_timezone: None,
            },
            replacement: None,
            cancelled: true,
        });
        event.recurrence = Some(recurrence);

        let result = availability_for_events(
            &[event],
            chrono_tz::UTC,
            start - Duration::hours(1),
            start + Duration::days(2),
        )
        .expect("availability");
        assert_eq!(result.busy.len(), 1);
        assert_eq!(result.busy[0].start_utc, start);
    }

    #[test]
    fn floating_time_uses_source_timezone_and_tentative_status() {
        let local = NaiveDate::from_ymd_opt(2026, 10, 8)
            .expect("date")
            .and_hms_opt(9, 0, 0)
            .expect("local");
        let mut event = TemporalEvent::new(
            "Tentative",
            TimeSpec::Floating {
                start: local,
                end: Some(local + Duration::hours(1)),
                source_timezone: Some("America/New_York".to_string()),
            },
        );
        event.status = EventStatus::Tentative;
        let expected_start = chrono_tz::America::New_York
            .from_local_datetime(&local)
            .single()
            .expect("resolved")
            .with_timezone(&Utc);

        let result = availability_for_events(
            &[event],
            chrono_tz::America::Mexico_City,
            expected_start - Duration::hours(1),
            expected_start + Duration::hours(2),
        )
        .expect("availability");
        assert_eq!(result.busy.len(), 1);
        assert_eq!(result.busy[0].kind, BusyKind::Tentative);
        assert_eq!(result.busy[0].start_utc, expected_start);
    }

    #[test]
    fn invalid_floating_source_timezone_is_skipped_without_inventing_clock_context() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 8)
            .expect("date")
            .and_hms_opt(9, 0, 0)
            .expect("time");
        let event = TemporalEvent::new(
            "Invalid timezone",
            TimeSpec::Floating {
                start,
                end: Some(start + Duration::hours(1)),
                source_timezone: Some("Not/A_Zone".to_string()),
            },
        );
        let window_start = Utc
            .with_ymd_and_hms(2026, 10, 8, 0, 0, 0)
            .single()
            .expect("window");
        let result = availability_for_events(
            std::slice::from_ref(&event),
            chrono_tz::UTC,
            window_start,
            window_start + Duration::days(1),
        )
        .expect("availability");
        assert!(result.busy.is_empty());
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].event_id, event.id);
        assert!(result.skipped[0].reason.contains("invalid source timezone"));
    }

    #[test]
    fn all_day_blocks_display_timezone_day_but_date_only_does_not() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 8).expect("day");
        let all_day = TemporalEvent::new(
            "All day",
            TimeSpec::AllDay {
                start: day,
                end_exclusive: None,
            },
        );
        let date_only = TemporalEvent::new(
            "Date only",
            TimeSpec::DateOnly {
                start: day,
                end_exclusive: None,
            },
        );
        let timezone = chrono_tz::America::Mexico_City;
        let local_start = timezone
            .from_local_datetime(&day.and_hms_opt(0, 0, 0).expect("midnight"))
            .single()
            .expect("resolved")
            .with_timezone(&Utc);
        let local_end = timezone
            .from_local_datetime(
                &day.checked_add_days(Days::new(1))
                    .expect("next day")
                    .and_hms_opt(0, 0, 0)
                    .expect("midnight"),
            )
            .single()
            .expect("resolved")
            .with_timezone(&Utc);

        let result =
            availability_for_events(&[all_day, date_only], timezone, local_start, local_end)
                .expect("availability");
        assert_eq!(result.busy.len(), 1);
        assert_eq!(result.busy[0].start_utc, local_start);
        assert_eq!(result.busy[0].end_utc, local_end);
        assert_eq!(result.skipped.len(), 1);
        assert!(result.skipped[0].reason.contains("date-only"));
        assert!(result.free.is_empty());
    }

    #[test]
    fn point_event_without_end_is_reported_instead_of_inventing_duration() {
        let start = Utc
            .with_ymd_and_hms(2026, 10, 8, 15, 0, 0)
            .single()
            .expect("start");
        let event = TemporalEvent::new(
            "Point",
            TimeSpec::Instant {
                start_utc: start,
                end_utc: None,
                source_timezone: None,
            },
        );
        let result = availability_for_events(
            &[event],
            chrono_tz::UTC,
            start - Duration::hours(1),
            start + Duration::hours(1),
        )
        .expect("availability");
        assert!(result.busy.is_empty());
        assert_eq!(result.skipped.len(), 1);
        assert!(result.skipped[0].reason.contains("no end"));
    }

    #[test]
    fn cancelled_postponed_and_superseded_events_do_not_block() {
        let start = Utc
            .with_ymd_and_hms(2026, 10, 8, 15, 0, 0)
            .single()
            .expect("start");
        let mut events = Vec::new();
        for status in [
            EventStatus::Cancelled,
            EventStatus::Postponed,
            EventStatus::Superseded,
        ] {
            let mut event = TemporalEvent::new(
                status.as_str(),
                TimeSpec::Instant {
                    start_utc: start,
                    end_utc: Some(start + Duration::hours(1)),
                    source_timezone: None,
                },
            );
            event.status = status;
            events.push(event);
        }
        let result = availability_for_events(
            &events,
            chrono_tz::UTC,
            start - Duration::hours(1),
            start + Duration::hours(2),
        )
        .expect("availability");
        assert!(result.busy.is_empty());
        assert_eq!(
            result.free,
            vec![FreeInterval {
                start_utc: start - Duration::hours(1),
                end_utc: start + Duration::hours(2),
            }]
        );
    }
}
