use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::calendar::CalendarView;
use crate::domain::{EventStatus, TemporalEvent};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventQuery {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub jurisdiction: Option<String>,
    #[serde(default)]
    pub status: Option<EventStatus>,
}

impl EventQuery {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
            && self.domain.is_none()
            && self.jurisdiction.is_none()
            && self.status.is_none()
    }

    pub fn matches(&self, event: &TemporalEvent) -> bool {
        if let Some(domain) = self.domain.as_deref()
            && event.domain.as_deref() != Some(domain)
        {
            return false;
        }

        if let Some(jurisdiction) = self.jurisdiction.as_deref()
            && event.jurisdiction.as_deref() != Some(jurisdiction)
        {
            return false;
        }

        if let Some(status) = self.status
            && event.status != status
        {
            return false;
        }

        let query = self.text.trim().to_ascii_lowercase();
        if query.is_empty() {
            return true;
        }

        let matches_text = [
            Some(event.normalized_title.as_str()),
            event.raw_title.as_deref(),
            event.description.as_deref(),
            event.event_type.as_deref(),
            event.domain.as_deref(),
            event.jurisdiction.as_deref(),
            event.institution.as_deref(),
            event.upstream_event_ref.as_deref(),
            event.upstream_reconciled_key.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|value| value.to_ascii_lowercase().contains(&query));

        matches_text
            || event
                .tags
                .iter()
                .chain(event.source_refs.iter())
                .chain(event.assertion_refs.iter())
                .chain(event.provenance_refs.iter())
                .any(|value| value.to_ascii_lowercase().contains(&query))
            || event
                .properties
                .to_string()
                .to_ascii_lowercase()
                .contains(&query)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedView {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub query: EventQuery,
    #[serde(default)]
    pub hidden_source_ids: BTreeSet<Uuid>,
    pub calendar_view: CalendarView,
    pub display_timezone: String,
    pub week_start_monday: bool,
}

impl SavedView {
    pub fn new(
        name: impl Into<String>,
        query: EventQuery,
        hidden_source_ids: BTreeSet<Uuid>,
        calendar_view: CalendarView,
        display_timezone: impl Into<String>,
        week_start_monday: bool,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            query,
            hidden_source_ids,
            calendar_view,
            display_timezone: display_timezone.into(),
            week_start_monday,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use chrono::NaiveDate;

    use super::*;
    use crate::domain::{TemporalEvent, TimeSpec};

    fn event() -> TemporalEvent {
        let mut event = TemporalEvent::new(
            "California General Election",
            TimeSpec::DateOnly {
                start: NaiveDate::from_ymd_opt(2026, 11, 3).expect("date"),
                end_exclusive: None,
            },
        );
        event.domain = Some("elections".to_string());
        event.jurisdiction = Some("US-CA".to_string());
        event.status = EventStatus::Confirmed;
        event.tags = vec!["state".to_string(), "general".to_string()];
        event
    }

    #[test]
    fn query_dimensions_compose() {
        let query = EventQuery {
            text: "general".to_string(),
            domain: Some("elections".to_string()),
            jurisdiction: Some("US-CA".to_string()),
            status: Some(EventStatus::Confirmed),
        };

        assert!(query.matches(&event()));
    }

    #[test]
    fn wrong_facet_rejects_event() {
        let query = EventQuery {
            domain: Some("economics".to_string()),
            ..EventQuery::default()
        };

        assert!(!query.matches(&event()));
    }

    #[test]
    fn saved_view_has_stable_identity() {
        let view = SavedView::new(
            "California Elections",
            EventQuery {
                domain: Some("elections".to_string()),
                jurisdiction: Some("US-CA".to_string()),
                ..EventQuery::default()
            },
            BTreeSet::new(),
            CalendarView::Month,
            "America/Mexico_City",
            false,
        );

        assert_eq!(view.name, "California Elections");
        assert!(!view.id.is_nil());
    }
}
