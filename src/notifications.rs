use std::collections::HashMap;

use anyhow::Context;
use chrono::{DateTime, Days, Duration, LocalResult, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::domain::{
    EventOccurrence, EventStatus, NotificationRule, NotificationTarget, NotificationTrigger,
    TemporalEvent, TimeSpec,
};
use crate::query::{
    EventMembership, QueryContext, SavedView,
    matches_composed_or_overlay_with_saved_views_and_membership,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationOccurrence {
    pub id: Uuid,
    pub rule_id: Uuid,
    pub rule_name: String,
    pub event_id: Uuid,
    pub occurrence_id: Uuid,
    pub event_title: String,
    pub trigger_at_utc: DateTime<Utc>,
    pub starts_at_utc: DateTime<Utc>,
    pub lead_minutes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationSkip {
    pub rule_id: Uuid,
    pub event_id: Uuid,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotificationEvaluation {
    pub occurrences: Vec<NotificationOccurrence>,
    pub skipped: Vec<NotificationSkip>,
}

pub fn evaluate_notification_rules(
    rules: &[NotificationRule],
    events: &[TemporalEvent],
    saved_views: &[SavedView],
    memberships: &HashMap<Uuid, EventMembership>,
    context: &QueryContext,
    window_start_utc: DateTime<Utc>,
    window_end_utc: DateTime<Utc>,
) -> anyhow::Result<NotificationEvaluation> {
    if window_end_utc <= window_start_utc {
        return Ok(NotificationEvaluation::default());
    }

    let mut evaluation = NotificationEvaluation::default();

    for rule in rules.iter().filter(|rule| rule.enabled) {
        rule.validate().context("invalid notification rule")?;
        match rule.target {
            NotificationTarget::Event { event_id } => {
                if let Some(event) = events.iter().find(|event| event.id == event_id) {
                    evaluate_rule_for_event(
                        rule,
                        event,
                        context,
                        window_start_utc,
                        window_end_utc,
                        &mut evaluation,
                    )?;
                }
            }
            NotificationTarget::SavedView { saved_view_id } => {
                let Some(view) = saved_views.iter().find(|view| view.id == saved_view_id) else {
                    continue;
                };
                for event in events {
                    if event
                        .source_id
                        .is_some_and(|source_id| view.hidden_source_ids.contains(&source_id))
                    {
                        continue;
                    }
                    let membership = memberships.get(&event.id);
                    if matches_composed_or_overlay_with_saved_views_and_membership(
                        &view.query,
                        &view.composition_layers,
                        &view.overlays,
                        saved_views,
                        event,
                        context,
                        membership,
                    ) {
                        evaluate_rule_for_event(
                            rule,
                            event,
                            context,
                            window_start_utc,
                            window_end_utc,
                            &mut evaluation,
                        )?;
                    }
                }
            }
        }
    }

    evaluation.occurrences.sort_by_key(|occurrence| {
        (
            occurrence.trigger_at_utc,
            occurrence.event_title.clone(),
            occurrence.rule_id,
            occurrence.occurrence_id,
        )
    });
    evaluation
        .skipped
        .sort_by_key(|skip| (skip.rule_id, skip.event_id, skip.reason.clone()));
    evaluation.skipped.dedup();

    Ok(evaluation)
}

fn evaluate_rule_for_event(
    rule: &NotificationRule,
    event: &TemporalEvent,
    context: &QueryContext,
    window_start_utc: DateTime<Utc>,
    window_end_utc: DateTime<Utc>,
    evaluation: &mut NotificationEvaluation,
) -> anyhow::Result<()> {
    let lead_minutes = match rule.trigger {
        NotificationTrigger::BeforeStart { minutes } => minutes,
    };
    let lead = Duration::minutes(i64::from(lead_minutes));
    let occurrence_window_start = window_start_utc
        .checked_add_signed(lead)
        .context("notification occurrence window overflow")?;
    let occurrence_window_end = window_end_utc
        .checked_add_signed(lead)
        .context("notification occurrence window overflow")?;

    let start_date = occurrence_window_start
        .with_timezone(&context.display_timezone)
        .date_naive()
        .checked_sub_days(Days::new(1))
        .unwrap_or(NaiveDate::MIN);
    let end_exclusive = occurrence_window_end
        .with_timezone(&context.display_timezone)
        .date_naive()
        .checked_add_days(Days::new(2))
        .unwrap_or(NaiveDate::MAX);

    let occurrences = event
        .occurrences_in_window(start_date, end_exclusive, context.display_timezone)
        .with_context(|| format!("failed to expand reminder target event {}", event.id))?;

    for occurrence in occurrences {
        if occurrence.status == EventStatus::Cancelled || occurrence.cancelled_by_override {
            continue;
        }
        let Some(starts_at_utc) = notification_start_utc(&occurrence.time, context.display_timezone)
        else {
            evaluation.skipped.push(NotificationSkip {
                rule_id: rule.id,
                event_id: event.id,
                reason: unsupported_time_reason(&occurrence.time),
            });
            continue;
        };
        let trigger_at_utc = starts_at_utc
            .checked_sub_signed(lead)
            .context("notification trigger time overflow")?;
        if trigger_at_utc < window_start_utc || trigger_at_utc >= window_end_utc {
            continue;
        }

        evaluation.occurrences.push(NotificationOccurrence {
            id: notification_occurrence_id(rule.id, occurrence.id),
            rule_id: rule.id,
            rule_name: rule.name.clone(),
            event_id: event.id,
            occurrence_id: occurrence.id,
            event_title: event.normalized_title.clone(),
            trigger_at_utc,
            starts_at_utc,
            lead_minutes,
        });
    }

    Ok(())
}

fn notification_start_utc(time: &TimeSpec, display_timezone: Tz) -> Option<DateTime<Utc>> {
    match time {
        TimeSpec::Instant { start_utc, .. } => Some(*start_utc),
        TimeSpec::Floating {
            start,
            source_timezone,
            ..
        } => {
            let timezone = source_timezone
                .as_deref()
                .and_then(|raw| raw.parse::<Tz>().ok())
                .unwrap_or(display_timezone);
            match timezone.from_local_datetime(start) {
                LocalResult::Single(value) => Some(value.with_timezone(&Utc)),
                LocalResult::Ambiguous(first, second) => {
                    Some(first.min(second).with_timezone(&Utc))
                }
                LocalResult::None => None,
            }
        }
        TimeSpec::DateOnly { .. }
        | TimeSpec::AllDay { .. }
        | TimeSpec::Month { .. }
        | TimeSpec::Year { .. }
        | TimeSpec::Unknown { .. } => None,
    }
}

fn unsupported_time_reason(time: &TimeSpec) -> String {
    match time {
        TimeSpec::Floating { .. } => {
            "floating start falls in a nonexistent local wall-clock interval".to_string()
        }
        _ => format!(
            "before-start notifications require a timed instant or floating DATE-TIME; got {}",
            time.kind_name()
        ),
    }
}

fn notification_occurrence_id(rule_id: Uuid, occurrence_id: Uuid) -> Uuid {
    let mut hasher = Sha256::new();
    hasher.update(rule_id.as_bytes());
    hasher.update(occurrence_id.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDateTime;

    use super::*;
    use crate::calendar::{CalendarLayout, CalendarView};
    use crate::domain::{
        RecurrenceFrequency, RecurrenceOverride, RecurrenceRule,
    };
    use crate::query::{
        ColorBy, EventQuery, GroupBy, SortRule, default_table_columns,
    };

    fn context() -> QueryContext {
        QueryContext::new(
            chrono_tz::America::Mexico_City,
            NaiveDate::from_ymd_opt(2026, 10, 7).expect("today"),
        )
    }

    fn saved_view(id: Uuid, query: EventQuery) -> SavedView {
        SavedView {
            id,
            name: "Reminder view".to_string(),
            query,
            hidden_source_ids: Default::default(),
            calendar_view: CalendarView::Month,
            calendar_layout: CalendarLayout::Grid,
            group_by: GroupBy::Date,
            sort_rules: vec![SortRule::default()],
            color_by: ColorBy::Status,
            color_rules: Vec::new(),
            composition_layers: Vec::new(),
            overlays: Vec::new(),
            table_columns: default_table_columns(),
            display_timezone: "America/Mexico_City".to_string(),
            week_start_monday: false,
        }
    }

    #[test]
    fn exact_event_notification_is_scheduled_before_start() {
        let start = Utc
            .with_ymd_and_hms(2026, 10, 7, 15, 0, 0)
            .single()
            .expect("start");
        let event = TemporalEvent::new(
            "Call",
            TimeSpec::Instant {
                start_utc: start,
                end_utc: None,
                source_timezone: None,
            },
        );
        let rule = NotificationRule::for_event(event.id, "15 minutes before", 15);
        let evaluation = evaluate_notification_rules(
            &[rule.clone()],
            &[event.clone()],
            &[],
            &HashMap::new(),
            &context(),
            start - Duration::minutes(20),
            start,
        )
        .expect("evaluate");

        assert_eq!(evaluation.skipped, Vec::new());
        assert_eq!(evaluation.occurrences.len(), 1);
        let occurrence = &evaluation.occurrences[0];
        assert_eq!(occurrence.rule_id, rule.id);
        assert_eq!(occurrence.event_id, event.id);
        assert_eq!(occurrence.starts_at_utc, start);
        assert_eq!(occurrence.trigger_at_utc, start - Duration::minutes(15));
        assert_eq!(
            occurrence.id,
            notification_occurrence_id(rule.id, event.id)
        );
    }

    #[test]
    fn floating_event_uses_source_timezone_or_display_timezone() {
        let wall = NaiveDateTime::parse_from_str(
            "2026-10-07T09:00:00",
            "%Y-%m-%dT%H:%M:%S",
        )
        .expect("wall");
        let mut source_tz = TemporalEvent::new(
            "Source zone",
            TimeSpec::Floating {
                start: wall,
                end: None,
                source_timezone: Some("America/New_York".to_string()),
            },
        );
        let source_rule = NotificationRule::for_event(source_tz.id, "At start", 0);
        let expected_source = chrono_tz::America::New_York
            .from_local_datetime(&wall)
            .single()
            .expect("local")
            .with_timezone(&Utc);

        let source_eval = evaluate_notification_rules(
            &[source_rule],
            &[source_tz.clone()],
            &[],
            &HashMap::new(),
            &context(),
            expected_source - Duration::minutes(1),
            expected_source + Duration::minutes(1),
        )
        .expect("source zone");
        assert_eq!(source_eval.occurrences[0].starts_at_utc, expected_source);

        source_tz.time = TimeSpec::Floating {
            start: wall,
            end: None,
            source_timezone: None,
        };
        let display_rule = NotificationRule::for_event(source_tz.id, "Display zone", 0);
        let expected_display = chrono_tz::America::Mexico_City
            .from_local_datetime(&wall)
            .single()
            .expect("display local")
            .with_timezone(&Utc);
        let display_eval = evaluate_notification_rules(
            &[display_rule],
            &[source_tz],
            &[],
            &HashMap::new(),
            &context(),
            expected_display - Duration::minutes(1),
            expected_display + Duration::minutes(1),
        )
        .expect("display zone");
        assert_eq!(
            display_eval.occurrences[0].starts_at_utc,
            expected_display
        );
    }

    #[test]
    fn recurrence_evaluation_uses_occurrence_identity_and_skips_cancelled_override() {
        let start = Utc
            .with_ymd_and_hms(2026, 10, 7, 15, 0, 0)
            .single()
            .expect("start");
        let second_start = start + Duration::days(1);
        let mut event = TemporalEvent::new(
            "Daily standup",
            TimeSpec::Instant {
                start_utc: start,
                end_utc: None,
                source_timezone: None,
            },
        );
        let mut recurrence = RecurrenceRule::new(RecurrenceFrequency::Daily);
        recurrence.count = Some(2);
        recurrence.overrides.push(RecurrenceOverride {
            original: TimeSpec::Instant {
                start_utc: second_start,
                end_utc: None,
                source_timezone: None,
            },
            replacement: None,
            cancelled: true,
        });
        event.recurrence = Some(recurrence);
        let rule = NotificationRule::for_event(event.id, "10 minutes before", 10);

        let evaluation = evaluate_notification_rules(
            &[rule],
            &[event],
            &[],
            &HashMap::new(),
            &context(),
            start - Duration::minutes(15),
            second_start + Duration::minutes(1),
        )
        .expect("evaluate");
        assert_eq!(evaluation.occurrences.len(), 1);
        assert_eq!(
            evaluation.occurrences[0].trigger_at_utc,
            start - Duration::minutes(10)
        );
    }

    #[test]
    fn saved_view_rule_reuses_query_semantics() {
        let start = Utc
            .with_ymd_and_hms(2026, 10, 7, 15, 0, 0)
            .single()
            .expect("start");
        let mut matching = TemporalEvent::new(
            "Matching",
            TimeSpec::Instant {
                start_utc: start,
                end_utc: None,
                source_timezone: None,
            },
        );
        matching.domain = Some("personal".to_string());
        let mut other = matching.clone();
        other.id = Uuid::new_v4();
        other.normalized_title = "Other".to_string();
        other.domain = Some("work".to_string());

        let view_id = Uuid::new_v4();
        let view = saved_view(
            view_id,
            EventQuery {
                domain: Some("personal".to_string()),
                ..EventQuery::default()
            },
        );
        let rule = NotificationRule::for_saved_view(view_id, "Personal events", 30);
        let evaluation = evaluate_notification_rules(
            &[rule],
            &[matching.clone(), other],
            &[view],
            &HashMap::new(),
            &context(),
            start - Duration::minutes(31),
            start - Duration::minutes(29),
        )
        .expect("evaluate");

        assert_eq!(evaluation.occurrences.len(), 1);
        assert_eq!(evaluation.occurrences[0].event_id, matching.id);
    }

    #[test]
    fn all_day_event_is_reported_as_unsupported_instead_of_inventing_a_clock() {
        let event = TemporalEvent::new(
            "All day",
            TimeSpec::AllDay {
                start: NaiveDate::from_ymd_opt(2026, 10, 7).expect("date"),
                end_exclusive: None,
            },
        );
        let rule = NotificationRule::for_event(event.id, "Reminder", 60);
        let evaluation = evaluate_notification_rules(
            &[rule],
            &[event],
            &[],
            &HashMap::new(),
            &context(),
            Utc.with_ymd_and_hms(2026, 10, 6, 0, 0, 0)
                .single()
                .expect("start"),
            Utc.with_ymd_and_hms(2026, 10, 9, 0, 0, 0)
                .single()
                .expect("end"),
        )
        .expect("evaluate");

        assert!(evaluation.occurrences.is_empty());
        assert_eq!(evaluation.skipped.len(), 1);
        assert!(evaluation.skipped[0].reason.contains("require a timed"));
    }
}
