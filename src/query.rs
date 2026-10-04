use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::calendar::{CalendarLayout, CalendarView};
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupBy {
    None,
    #[default]
    Date,
    Week,
    Month,
    Source,
    Domain,
    Jurisdiction,
    Institution,
    EventType,
    Status,
}

impl GroupBy {
    pub const ALL: [Self; 10] = [
        Self::None,
        Self::Date,
        Self::Week,
        Self::Month,
        Self::Source,
        Self::Domain,
        Self::Jurisdiction,
        Self::Institution,
        Self::EventType,
        Self::Status,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "No grouping",
            Self::Date => "Date",
            Self::Week => "Week",
            Self::Month => "Month",
            Self::Source => "Source",
            Self::Domain => "Domain",
            Self::Jurisdiction => "Jurisdiction",
            Self::Institution => "Institution",
            Self::EventType => "Event type",
            Self::Status => "Status",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortField {
    #[default]
    Time,
    Title,
    Importance,
    PersonalRelevance,
    Source,
    Domain,
    Jurisdiction,
    Institution,
    EventType,
    Status,
}

impl SortField {
    pub const ALL: [Self; 10] = [
        Self::Time,
        Self::Title,
        Self::Importance,
        Self::PersonalRelevance,
        Self::Source,
        Self::Domain,
        Self::Jurisdiction,
        Self::Institution,
        Self::EventType,
        Self::Status,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Time => "Time",
            Self::Title => "Title",
            Self::Importance => "Importance",
            Self::PersonalRelevance => "Personal relevance",
            Self::Source => "Source",
            Self::Domain => "Domain",
            Self::Jurisdiction => "Jurisdiction",
            Self::Institution => "Institution",
            Self::EventType => "Event type",
            Self::Status => "Status",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    #[default]
    Ascending,
    Descending,
}

impl SortDirection {
    pub const ALL: [Self; 2] = [Self::Ascending, Self::Descending];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Ascending => "Ascending",
            Self::Descending => "Descending",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortRule {
    pub field: SortField,
    pub direction: SortDirection,
}

impl Default for SortRule {
    fn default() -> Self {
        Self {
            field: SortField::Time,
            direction: SortDirection::Ascending,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorBy {
    None,
    Source,
    Domain,
    Jurisdiction,
    Institution,
    EventType,
    #[default]
    Status,
}

impl ColorBy {
    pub const ALL: [Self; 7] = [
        Self::None,
        Self::Source,
        Self::Domain,
        Self::Jurisdiction,
        Self::Institution,
        Self::EventType,
        Self::Status,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "No semantic color",
            Self::Source => "Source",
            Self::Domain => "Domain",
            Self::Jurisdiction => "Jurisdiction",
            Self::Institution => "Institution",
            Self::EventType => "Event type",
            Self::Status => "Status",
        }
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
    #[serde(default)]
    pub calendar_layout: CalendarLayout,
    #[serde(default)]
    pub group_by: GroupBy,
    #[serde(default)]
    pub sort_rules: Vec<SortRule>,
    #[serde(default)]
    pub color_by: ColorBy,
    pub display_timezone: String,
    pub week_start_monday: bool,
}

impl SavedView {
    pub fn new(
        name: impl Into<String>,
        query: EventQuery,
        hidden_source_ids: BTreeSet<Uuid>,
        calendar_view: CalendarView,
        calendar_layout: CalendarLayout,
        group_by: GroupBy,
        sort_rules: Vec<SortRule>,
        color_by: ColorBy,
        display_timezone: impl Into<String>,
        week_start_monday: bool,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            query,
            hidden_source_ids,
            calendar_view,
            calendar_layout,
            group_by,
            sort_rules,
            color_by,
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
    fn saved_view_keeps_independent_presentation_dimensions() {
        let view = SavedView::new(
            "California Elections",
            EventQuery {
                domain: Some("elections".to_string()),
                jurisdiction: Some("US-CA".to_string()),
                ..EventQuery::default()
            },
            BTreeSet::new(),
            CalendarView::Month,
            CalendarLayout::Agenda,
            GroupBy::Jurisdiction,
            vec![SortRule {
                field: SortField::Importance,
                direction: SortDirection::Descending,
            }],
            ColorBy::EventType,
            "America/Mexico_City",
            false,
        );

        assert_eq!(view.name, "California Elections");
        assert_eq!(view.calendar_layout, CalendarLayout::Agenda);
        assert_eq!(view.group_by, GroupBy::Jurisdiction);
        assert_eq!(view.color_by, ColorBy::EventType);
        assert_eq!(view.sort_rules.len(), 1);
        assert!(!view.id.is_nil());
    }
}
