//! Conservative placement transformations for calendar authoring.
//!
//! An uncertain start is a bounded set of possible starts, not a definite
//! appointment at its representative start. A move must translate the entire
//! set without collapsing it to that representative.

use anyhow::{Context, anyhow, bail};
use chrono::{Duration, NaiveDate, NaiveDateTime, TimeZone};
use chrono_tz::Tz;

use crate::domain::{TemporalEvent, TimeSpec, TimeUncertainty};

/// Shift a non-recurring event's representative placement and its entire
/// bounded start uncertainty by the same displacement.
///
/// The caller supplies a fully specified new time with unchanged duration,
/// precision, and source clock context. This is a pure transformation: the
/// returned event retains canonical identity, metadata, and revision times;
/// a later confirmed save owns persistence and updated_at.
pub fn move_uncertain_placement(
    event: &TemporalEvent,
    proposed_time: &TimeSpec,
) -> anyhow::Result<TemporalEvent> {
    if event.recurrence.is_some() {
        bail!("uncertain recurring events are not supported");
    }
    let uncertainty = event
        .time_uncertainty
        .as_ref()
        .ok_or_else(|| anyhow!("event has no bounded placement uncertainty"))?;

    let shifted = match (&event.time, proposed_time, uncertainty) {
        (
            TimeSpec::Instant {
                start_utc: old_start,
                end_utc: Some(old_end),
                source_timezone: old_zone,
            },
            TimeSpec::Instant {
                start_utc: new_start,
                end_utc: Some(new_end),
                source_timezone: new_zone,
            },
            TimeUncertainty::InstantWindow {
                earliest_utc,
                latest_utc,
            },
        ) => {
            require_clock_context(old_zone, new_zone)?;
            require_duration(*old_end - *old_start, *new_end - *new_start)?;
            let delta = *new_start - *old_start;
            TimeUncertainty::InstantWindow {
                earliest_utc: earliest_utc.checked_add_signed(delta).ok_or_else(|| {
                    anyhow!("shifted earliest instant is outside supported range")
                })?,
                latest_utc: latest_utc
                    .checked_add_signed(delta)
                    .ok_or_else(|| anyhow!("shifted latest instant is outside supported range"))?,
            }
        }
        (
            TimeSpec::Floating {
                start: old_start,
                end: Some(old_end),
                source_timezone: old_zone,
            },
            TimeSpec::Floating {
                start: new_start,
                end: Some(new_end),
                source_timezone: new_zone,
            },
            TimeUncertainty::FloatingWindow { earliest, latest },
        ) => {
            require_clock_context(old_zone, new_zone)?;
            require_duration(*old_end - *old_start, *new_end - *new_start)?;
            ensure_unambiguous_wall_clock(*new_start, *new_end, new_zone.as_deref())?;
            let delta = *new_start - *old_start;
            TimeUncertainty::FloatingWindow {
                earliest: earliest
                    .checked_add_signed(delta)
                    .ok_or_else(|| anyhow!("shifted earliest floating time overflows"))?,
                latest: latest
                    .checked_add_signed(delta)
                    .ok_or_else(|| anyhow!("shifted latest floating time overflows"))?,
            }
        }
        (
            TimeSpec::AllDay {
                start: old_start,
                end_exclusive: old_end,
            },
            TimeSpec::AllDay {
                start: new_start,
                end_exclusive: new_end,
            },
            TimeUncertainty::DateWindow { earliest, latest },
        )
        | (
            TimeSpec::DateOnly {
                start: old_start,
                end_exclusive: old_end,
            },
            TimeSpec::DateOnly {
                start: new_start,
                end_exclusive: new_end,
            },
            TimeUncertainty::DateWindow { earliest, latest },
        ) => {
            require_civil_span(*old_start, *old_end, *new_start, *new_end)?;
            let delta = *new_start - *old_start;
            TimeUncertainty::DateWindow {
                earliest: earliest
                    .checked_add_signed(delta)
                    .ok_or_else(|| anyhow!("shifted earliest civil date overflows"))?,
                latest: latest
                    .checked_add_signed(delta)
                    .ok_or_else(|| anyhow!("shifted latest civil date overflows"))?,
            }
        }
        _ => bail!(
            "uncertain placement move must preserve the existing time kind,              defined duration, and matching uncertainty coordinates"
        ),
    };

    let mut moved = event.clone();
    moved.time = proposed_time.clone();
    moved.time_uncertainty = Some(shifted);
    moved
        .validate_time_uncertainty()
        .context("shift would invalidate bounded placement uncertainty")?;
    Ok(moved)
}

fn require_clock_context(old: &Option<String>, new: &Option<String>) -> anyhow::Result<()> {
    if old != new {
        bail!("uncertain placement move cannot change source clock context");
    }
    Ok(())
}

fn require_duration(old: Duration, new: Duration) -> anyhow::Result<()> {
    if old <= Duration::zero() || old != new {
        bail!("uncertain placement move must preserve a positive exact duration");
    }
    Ok(())
}

fn require_civil_span(
    old_start: NaiveDate,
    old_end: Option<NaiveDate>,
    new_start: NaiveDate,
    new_end: Option<NaiveDate>,
) -> anyhow::Result<()> {
    if old_end.is_some() != new_end.is_some() {
        bail!("uncertain civil move must preserve explicit exclusive-end shape");
    }
    let old_span = old_end.map_or(1, |end| (end - old_start).num_days());
    let new_span = new_end.map_or(1, |end| (end - new_start).num_days());
    if old_span <= 0 || old_span != new_span {
        bail!("uncertain civil move must preserve its positive day span");
    }
    Ok(())
}

fn ensure_unambiguous_wall_clock(
    start: NaiveDateTime,
    end: NaiveDateTime,
    source_timezone: Option<&str>,
) -> anyhow::Result<()> {
    let Some(raw) = source_timezone else {
        // A floating time without an attached timezone has deliberately
        // unresolved wall-clock interpretation; do not assign it one.
        return Ok(());
    };
    let timezone = raw
        .parse::<Tz>()
        .with_context(|| format!("invalid source timezone {raw:?}"))?;
    if timezone.from_local_datetime(&start).single().is_none()
        || timezone.from_local_datetime(&end).single().is_none()
    {
        bail!("proposed floating start/end is ambiguous or nonexistent in {timezone}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::{Days, NaiveDate, TimeZone, Utc};

    use super::*;
    use crate::domain::{RecurrenceFrequency, RecurrenceRule};

    #[test]
    fn instant_move_shifts_the_whole_window_not_only_representative() {
        let anchor = Utc
            .with_ymd_and_hms(2026, 10, 8, 9, 0, 0)
            .single()
            .expect("anchor");
        let mut event = TemporalEvent::new(
            "Flexible appointment",
            TimeSpec::Instant {
                start_utc: anchor,
                end_utc: Some(anchor + Duration::hours(1)),
                source_timezone: Some("America/Mexico_City".to_string()),
            },
        );
        event.time_uncertainty = Some(TimeUncertainty::InstantWindow {
            earliest_utc: anchor - Duration::hours(2),
            latest_utc: anchor + Duration::hours(3),
        });
        let before = event.clone();
        let proposed = TimeSpec::Instant {
            start_utc: anchor + Duration::days(2),
            end_utc: Some(anchor + Duration::days(2) + Duration::hours(1)),
            source_timezone: Some("America/Mexico_City".to_string()),
        };
        let moved = move_uncertain_placement(&event, &proposed).expect("move");

        assert_eq!(moved.id, event.id);
        assert_eq!(moved.created_at, event.created_at);
        assert_eq!(moved.updated_at, event.updated_at);
        assert_eq!(moved.time, proposed);
        assert_eq!(
            moved.time_uncertainty,
            Some(TimeUncertainty::InstantWindow {
                earliest_utc: anchor + Duration::days(2) - Duration::hours(2),
                latest_utc: anchor + Duration::days(2) + Duration::hours(3),
            })
        );
        assert_eq!(event, before);
    }

    #[test]
    fn civil_move_preserves_two_day_span_across_dst_transition() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 30).expect("date");
        let mut event = TemporalEvent::new(
            "Possible retreat",
            TimeSpec::AllDay {
                start,
                end_exclusive: Some(start + Days::new(2)),
            },
        );
        event.time_uncertainty = Some(TimeUncertainty::DateWindow {
            earliest: start - Days::new(1),
            latest: start + Days::new(2),
        });
        let new_start = NaiveDate::from_ymd_opt(2026, 11, 1).expect("new date");
        let moved = move_uncertain_placement(
            &event,
            &TimeSpec::AllDay {
                start: new_start,
                end_exclusive: Some(new_start + Days::new(2)),
            },
        )
        .expect("civil shift");
        assert_eq!(
            moved.time_uncertainty,
            Some(TimeUncertainty::DateWindow {
                earliest: new_start - Days::new(1),
                latest: new_start + Days::new(2),
            })
        );
        assert!(matches!(moved.time, TimeSpec::AllDay { .. }));
    }

    #[test]
    fn floating_move_keeps_wall_coordinates_but_rejects_dst_gap() {
        let tz = "America/New_York".to_string();
        let date = NaiveDate::from_ymd_opt(2026, 3, 6).expect("date");
        let original = date.and_hms_opt(9, 0, 0).expect("time");
        let mut event = TemporalEvent::new(
            "Possible call",
            TimeSpec::Floating {
                start: original,
                end: Some(original + Duration::hours(1)),
                source_timezone: Some(tz.clone()),
            },
        );
        event.time_uncertainty = Some(TimeUncertainty::FloatingWindow {
            earliest: original - Duration::minutes(30),
            latest: original + Duration::minutes(30),
        });

        let valid_start = NaiveDate::from_ymd_opt(2026, 3, 9)
            .expect("date")
            .and_hms_opt(9, 0, 0)
            .expect("time");
        let moved = move_uncertain_placement(
            &event,
            &TimeSpec::Floating {
                start: valid_start,
                end: Some(valid_start + Duration::hours(1)),
                source_timezone: Some(tz.clone()),
            },
        )
        .expect("wall move");
        assert_eq!(
            moved.time_uncertainty,
            Some(TimeUncertainty::FloatingWindow {
                earliest: valid_start - Duration::minutes(30),
                latest: valid_start + Duration::minutes(30),
            })
        );

        let gap_start = NaiveDate::from_ymd_opt(2026, 3, 8)
            .expect("gap date")
            .and_hms_opt(2, 30, 0)
            .expect("gap time");
        assert!(
            move_uncertain_placement(
                &event,
                &TimeSpec::Floating {
                    start: gap_start,
                    end: Some(gap_start + Duration::hours(1)),
                    source_timezone: Some(tz),
                },
            )
            .is_err()
        );
    }

    #[test]
    fn uncertain_moves_refuse_duration_precision_zone_and_recurrence_changes() {
        let start = Utc
            .with_ymd_and_hms(2026, 10, 8, 9, 0, 0)
            .single()
            .expect("start");
        let original = TimeSpec::Instant {
            start_utc: start,
            end_utc: Some(start + Duration::hours(1)),
            source_timezone: None,
        };
        let mut event = TemporalEvent::new("Uncertain", original.clone());
        let proposed = TimeSpec::Instant {
            start_utc: start + Duration::days(1),
            end_utc: Some(start + Duration::days(1) + Duration::hours(1)),
            source_timezone: None,
        };
        assert!(move_uncertain_placement(&event, &proposed).is_err());
        event.time_uncertainty = Some(TimeUncertainty::InstantWindow {
            earliest_utc: start - Duration::hours(1),
            latest_utc: start + Duration::hours(1),
        });

        let changed_duration = TimeSpec::Instant {
            start_utc: start + Duration::days(1),
            end_utc: Some(start + Duration::days(1) + Duration::hours(2)),
            source_timezone: None,
        };
        assert!(move_uncertain_placement(&event, &changed_duration).is_err());

        let changed_zone = TimeSpec::Instant {
            start_utc: start + Duration::days(1),
            end_utc: Some(start + Duration::days(1) + Duration::hours(1)),
            source_timezone: Some("UTC".to_string()),
        };
        assert!(move_uncertain_placement(&event, &changed_zone).is_err());
        assert!(
            move_uncertain_placement(
                &event,
                &TimeSpec::DateOnly {
                    start: NaiveDate::from_ymd_opt(2026, 10, 9).expect("date"),
                    end_exclusive: None,
                },
            )
            .is_err()
        );

        event.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Daily));
        assert!(move_uncertain_placement(&event, &proposed).is_err());
    }
}
