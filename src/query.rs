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
    #[serde(default)]
    pub expression: Option<QueryExpr>,
}

impl EventQuery {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
            && self.domain.is_none()
            && self.jurisdiction.is_none()
            && self.status.is_none()
            && self.expression.is_none()
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

        let query = self.text.trim().to_lowercase();
        if !query.is_empty() {
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
            .any(|value| value.to_lowercase().contains(&query));

            let matches_collection = event
                .tags
                .iter()
                .chain(event.source_refs.iter())
                .chain(event.assertion_refs.iter())
                .chain(event.provenance_refs.iter())
                .any(|value| value.to_lowercase().contains(&query));

            let matches_properties = event
                .properties
                .to_string()
                .to_lowercase()
                .contains(&query);

            if !(matches_text || matches_collection || matches_properties) {
                return false;
            }
        }

        self.expression
            .as_ref()
            .is_none_or(|expression| expression.matches(event))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", content = "args", rename_all = "snake_case")]
pub enum QueryExpr {
    All(Vec<QueryExpr>),
    Any(Vec<QueryExpr>),
    Not(Box<QueryExpr>),
    Predicate(QueryPredicate),
}

impl QueryExpr {
    pub fn matches(&self, event: &TemporalEvent) -> bool {
        match self {
            Self::All(expressions) => expressions
                .iter()
                .all(|expression| expression.matches(event)),
            Self::Any(expressions) => expressions
                .iter()
                .any(|expression| expression.matches(event)),
            Self::Not(expression) => !expression.matches(event),
            Self::Predicate(predicate) => predicate.matches(event),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueryPredicate {
    Text {
        field: TextField,
        operator: TextOperator,
        value: String,
        #[serde(default)]
        case_sensitive: bool,
    },
    TextAnyOf {
        field: TextField,
        values: Vec<String>,
        #[serde(default)]
        case_sensitive: bool,
    },
    StatusAnyOf {
        values: Vec<EventStatus>,
    },
    Integer {
        field: IntegerField,
        operator: IntegerOperator,
        value: i32,
    },
    Exists {
        field: PresenceField,
        exists: bool,
    },
}

impl QueryPredicate {
    pub fn matches(&self, event: &TemporalEvent) -> bool {
        match self {
            Self::Text {
                field,
                operator,
                value,
                case_sensitive,
            } => {
                let values = text_values(event, *field);
                if *operator == TextOperator::NotEquals {
                    !values.is_empty()
                        && values.into_iter().all(|candidate| {
                            !text_matches(
                                candidate,
                                value,
                                TextOperator::Equals,
                                *case_sensitive,
                            )
                        })
                } else {
                    values.into_iter().any(|candidate| {
                        text_matches(candidate, value, *operator, *case_sensitive)
                    })
                }
            },
            Self::TextAnyOf {
                field,
                values,
                case_sensitive,
            } => text_values(event, *field).into_iter().any(|candidate| {
                values
                    .iter()
                    .any(|value| text_matches(candidate, value, TextOperator::Equals, *case_sensitive))
            }),
            Self::StatusAnyOf { values } => values.contains(&event.status),
            Self::Integer {
                field,
                operator,
                value,
            } => integer_value(event, *field)
                .is_some_and(|candidate| integer_matches(candidate, *value, *operator)),
            Self::Exists { field, exists } => field_exists(event, *field) == *exists,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextField {
    Title,
    RawTitle,
    Description,
    EventType,
    Domain,
    Jurisdiction,
    Institution,
    Renderability,
    UpstreamEventRef,
    UpstreamReconciledKey,
    SourceRecordKey,
    Tags,
    SourceRefs,
    AssertionRefs,
    ProvenanceRefs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextOperator {
    Equals,
    NotEquals,
    Contains,
    StartsWith,
    EndsWith,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegerField {
    Importance,
    PersonalRelevance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegerOperator {
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceField {
    Source,
    RawTitle,
    Description,
    EventType,
    Domain,
    Jurisdiction,
    Institution,
    Renderability,
    Confidence,
    Importance,
    PersonalRelevance,
    UpstreamEventRef,
    UpstreamReconciledKey,
    SourceRecordKey,
    Tags,
    SourceRefs,
    AssertionRefs,
    ProvenanceRefs,
}

fn text_values(event: &TemporalEvent, field: TextField) -> Vec<&str> {
    match field {
        TextField::Title => vec![event.normalized_title.as_str()],
        TextField::RawTitle => event.raw_title.iter().map(String::as_str).collect(),
        TextField::Description => event.description.iter().map(String::as_str).collect(),
        TextField::EventType => event.event_type.iter().map(String::as_str).collect(),
        TextField::Domain => event.domain.iter().map(String::as_str).collect(),
        TextField::Jurisdiction => event.jurisdiction.iter().map(String::as_str).collect(),
        TextField::Institution => event.institution.iter().map(String::as_str).collect(),
        TextField::Renderability => event.renderability.iter().map(String::as_str).collect(),
        TextField::UpstreamEventRef => event
            .upstream_event_ref
            .iter()
            .map(String::as_str)
            .collect(),
        TextField::UpstreamReconciledKey => event
            .upstream_reconciled_key
            .iter()
            .map(String::as_str)
            .collect(),
        TextField::SourceRecordKey => event.source_record_key.iter().map(String::as_str).collect(),
        TextField::Tags => event.tags.iter().map(String::as_str).collect(),
        TextField::SourceRefs => event.source_refs.iter().map(String::as_str).collect(),
        TextField::AssertionRefs => event.assertion_refs.iter().map(String::as_str).collect(),
        TextField::ProvenanceRefs => event.provenance_refs.iter().map(String::as_str).collect(),
    }
}

fn text_matches(
    candidate: &str,
    expected: &str,
    operator: TextOperator,
    case_sensitive: bool,
) -> bool {
    if case_sensitive {
        return match operator {
            TextOperator::Equals => candidate == expected,
            TextOperator::NotEquals => candidate != expected,
            TextOperator::Contains => candidate.contains(expected),
            TextOperator::StartsWith => candidate.starts_with(expected),
            TextOperator::EndsWith => candidate.ends_with(expected),
        };
    }

    let candidate = candidate.to_lowercase();
    let expected = expected.to_lowercase();
    match operator {
        TextOperator::Equals => candidate == expected,
        TextOperator::NotEquals => candidate != expected,
        TextOperator::Contains => candidate.contains(&expected),
        TextOperator::StartsWith => candidate.starts_with(&expected),
        TextOperator::EndsWith => candidate.ends_with(&expected),
    }
}

fn integer_value(event: &TemporalEvent, field: IntegerField) -> Option<i32> {
    match field {
        IntegerField::Importance => event.importance,
        IntegerField::PersonalRelevance => event.personal_relevance,
    }
}

fn integer_matches(candidate: i32, expected: i32, operator: IntegerOperator) -> bool {
    match operator {
        IntegerOperator::Equal => candidate == expected,
        IntegerOperator::NotEqual => candidate != expected,
        IntegerOperator::LessThan => candidate < expected,
        IntegerOperator::LessThanOrEqual => candidate <= expected,
        IntegerOperator::GreaterThan => candidate > expected,
        IntegerOperator::GreaterThanOrEqual => candidate >= expected,
    }
}

fn field_exists(event: &TemporalEvent, field: PresenceField) -> bool {
    match field {
        PresenceField::Source => event.source_id.is_some() || !event.source_refs.is_empty(),
        PresenceField::RawTitle => event.raw_title.is_some(),
        PresenceField::Description => event.description.is_some(),
        PresenceField::EventType => event.event_type.is_some(),
        PresenceField::Domain => event.domain.is_some(),
        PresenceField::Jurisdiction => event.jurisdiction.is_some(),
        PresenceField::Institution => event.institution.is_some(),
        PresenceField::Renderability => event.renderability.is_some(),
        PresenceField::Confidence => event.confidence.is_some(),
        PresenceField::Importance => event.importance.is_some(),
        PresenceField::PersonalRelevance => event.personal_relevance.is_some(),
        PresenceField::UpstreamEventRef => event.upstream_event_ref.is_some(),
        PresenceField::UpstreamReconciledKey => event.upstream_reconciled_key.is_some(),
        PresenceField::SourceRecordKey => event.source_record_key.is_some(),
        PresenceField::Tags => !event.tags.is_empty(),
        PresenceField::SourceRefs => !event.source_refs.is_empty(),
        PresenceField::AssertionRefs => !event.assertion_refs.is_empty(),
        PresenceField::ProvenanceRefs => !event.provenance_refs.is_empty(),
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
        event.institution = Some("California Secretary of State".to_string());
        event.status = EventStatus::Confirmed;
        event.importance = Some(90);
        event.tags = vec!["state".to_string(), "general".to_string()];
        event.source_refs = vec!["ca-sos".to_string()];
        event
    }

    #[test]
    fn query_dimensions_compose() {
        let query = EventQuery {
            text: "general".to_string(),
            domain: Some("elections".to_string()),
            jurisdiction: Some("US-CA".to_string()),
            status: Some(EventStatus::Confirmed),
            expression: None,
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
    fn nested_boolean_expression_matches() {
        let query = EventQuery {
            expression: Some(QueryExpr::All(vec![
                QueryExpr::Any(vec![
                    QueryExpr::Predicate(QueryPredicate::Text {
                        field: TextField::Institution,
                        operator: TextOperator::Contains,
                        value: "secretary".to_string(),
                        case_sensitive: false,
                    }),
                    QueryExpr::Predicate(QueryPredicate::Text {
                        field: TextField::Domain,
                        operator: TextOperator::Equals,
                        value: "economics".to_string(),
                        case_sensitive: false,
                    }),
                ]),
                QueryExpr::Not(Box::new(QueryExpr::Predicate(
                    QueryPredicate::StatusAnyOf {
                        values: vec![EventStatus::Cancelled, EventStatus::Postponed],
                    },
                ))),
                QueryExpr::Predicate(QueryPredicate::Integer {
                    field: IntegerField::Importance,
                    operator: IntegerOperator::GreaterThanOrEqual,
                    value: 80,
                }),
            ])),
            ..EventQuery::default()
        };

        assert!(query.matches(&event()));
    }

    #[test]
    fn not_equals_on_collection_requires_no_equal_member() {
        let query = EventQuery {
            expression: Some(QueryExpr::Predicate(QueryPredicate::Text {
                field: TextField::Tags,
                operator: TextOperator::NotEquals,
                value: "general".to_string(),
                case_sensitive: false,
            })),
            ..EventQuery::default()
        };

        assert!(!query.matches(&event()));
    }

    #[test]
    fn text_sets_and_existence_checks_work_on_collections() {
        let query = EventQuery {
            expression: Some(QueryExpr::All(vec![
                QueryExpr::Predicate(QueryPredicate::TextAnyOf {
                    field: TextField::Tags,
                    values: vec!["general".to_string(), "federal".to_string()],
                    case_sensitive: false,
                }),
                QueryExpr::Predicate(QueryPredicate::Text {
                    field: TextField::SourceRefs,
                    operator: TextOperator::Equals,
                    value: "CA-SOS".to_string(),
                    case_sensitive: false,
                }),
                QueryExpr::Predicate(QueryPredicate::Exists {
                    field: PresenceField::ProvenanceRefs,
                    exists: false,
                }),
            ])),
            ..EventQuery::default()
        };

        assert!(query.matches(&event()));
    }

    #[test]
    fn saved_view_keeps_independent_presentation_dimensions() {
        let view = SavedView {
            id: Uuid::new_v4(),
            name: "California Elections".to_string(),
            query: EventQuery {
                domain: Some("elections".to_string()),
                jurisdiction: Some("US-CA".to_string()),
                ..EventQuery::default()
            },
            hidden_source_ids: BTreeSet::new(),
            calendar_view: CalendarView::Month,
            calendar_layout: CalendarLayout::Agenda,
            group_by: GroupBy::Jurisdiction,
            sort_rules: vec![SortRule {
                field: SortField::Importance,
                direction: SortDirection::Descending,
            }],
            color_by: ColorBy::EventType,
            display_timezone: "America/Mexico_City".to_string(),
            week_start_monday: false,
        };

        assert_eq!(view.name, "California Elections");
        assert_eq!(view.calendar_layout, CalendarLayout::Agenda);
        assert_eq!(view.group_by, GroupBy::Jurisdiction);
        assert_eq!(view.color_by, ColorBy::EventType);
        assert_eq!(view.sort_rules.len(), 1);
        assert!(!view.id.is_nil());
    }
}
