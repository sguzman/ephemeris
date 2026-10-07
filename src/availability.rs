use chrono::{DateTime, Days, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
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

    let mut result = AvailabilityResult::default();
    for event in events {
        let occurrences = event
            .occurrences_in_window(start_date, end_exclusive, display_timezone)
            .map_err(|error| {
                anyhow::anyhow!("failed to expand availability event {}: {error}", event.id)
            })?;

        for occurrence in occurrences {
            let Some(kind) = busy_kind(occurrence.status) else {
                continue;
            };
            match occurrence_interval_utc(&occurrence, display_timezone) {
                Ok(Some((start_utc, end_utc))) => {
                    let clipped_start = start_utc.max(window_start_utc);
                    let clipped_end = end_utc.min(window_end_utc);
                    if clipped_end > clipped_start {
                        result.busy.push(BusyInterval {
                            event_id: event.id,
                            occurrence_id: occurrence.id,
                            event_title: event.normalized_title.clone(),
                            start_utc: clipped_start,
                            end_utc: clipped_end,
                            kind,
                        });
                    }
                }
                Ok(None) => {}
                Err(reason) => result.skipped.push(AvailabilitySkip {
                    event_id: event.id,
                    reason,
                }),
            }
        }
    }

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
            let timezone = source_timezone
                .as_deref()
                .and_then(|raw| raw.parse::<Tz>().ok())
                .unwrap_or(display_timezone);
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
