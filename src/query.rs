use std::collections::BTreeSet;

use chrono::{Datelike, Duration, NaiveDate, NaiveTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::calendar::{CalendarLayout, CalendarView};
use crate::domain::{EventIdentityState, EventStatus, TemporalEvent, TimeSpec, TimeUncertainty};

#[derive(Debug, Clone, Copy)]
pub struct QueryContext {
    pub display_timezone: Tz,
    pub today: NaiveDate,
}

impl QueryContext {
    pub fn new(display_timezone: Tz, today: NaiveDate) -> Self {
        Self {
            display_timezone,
            today,
        }
    }

    pub fn for_timezone(display_timezone: Tz) -> Self {
        Self {
            display_timezone,
            today: Utc::now().with_timezone(&display_timezone).date_naive(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventMembership {
    pub bundle_refs: BTreeSet<String>,
    pub calendar_refs: BTreeSet<String>,
    pub collection_ids: BTreeSet<Uuid>,
    pub outgoing_relation_types: BTreeSet<String>,
    pub incoming_relation_types: BTreeSet<String>,
    pub identity_states: BTreeSet<String>,
    pub annotation_kinds: BTreeSet<String>,
}

impl EventMembership {
    pub fn belongs_to_bundle(&self, bundle_ref: &str) -> bool {
        self.bundle_refs.contains(bundle_ref)
    }

    pub fn belongs_to_calendar(&self, calendar_id: &str) -> bool {
        self.calendar_refs.contains(calendar_id)
    }

    pub fn belongs_to_collection(&self, collection_id: Uuid) -> bool {
        self.collection_ids.contains(&collection_id)
    }

    pub fn has_relation_type(&self, relation_type: &str, direction: RelationDirection) -> bool {
        match direction {
            RelationDirection::Outgoing => self.outgoing_relation_types.contains(relation_type),
            RelationDirection::Incoming => self.incoming_relation_types.contains(relation_type),
            RelationDirection::Either => {
                self.outgoing_relation_types.contains(relation_type)
                    || self.incoming_relation_types.contains(relation_type)
            }
        }
    }

    pub fn has_identity_state(&self, state: EventIdentityState) -> bool {
        self.identity_states.contains(state.as_str())
    }

    pub fn has_annotation_kind(&self, kind: &str) -> bool {
        self.annotation_kinds.contains(kind)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationDirection {
    Outgoing,
    Incoming,
    #[default]
    Either,
}

impl RelationDirection {
    pub const ALL: [Self; 3] = [Self::Outgoing, Self::Incoming, Self::Either];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Outgoing => "Outgoing",
            Self::Incoming => "Incoming",
            Self::Either => "Either direction",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventQuery {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub jurisdiction: Option<String>,
    #[serde(default)]
    pub event_type: Option<String>,
    #[serde(default)]
    pub institution: Option<String>,
    #[serde(default)]
    pub renderability: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
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
            && self.event_type.is_none()
            && self.institution.is_none()
            && self.renderability.is_none()
            && self.tag.is_none()
            && self.status.is_none()
            && self.expression.is_none()
    }

    pub fn matches(&self, event: &TemporalEvent, context: &QueryContext) -> bool {
        self.matches_with_membership(event, context, None)
    }

    pub fn matches_with_membership(
        &self,
        event: &TemporalEvent,
        context: &QueryContext,
        membership: Option<&EventMembership>,
    ) -> bool {
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

        if let Some(event_type) = self.event_type.as_deref()
            && event.event_type.as_deref() != Some(event_type)
        {
            return false;
        }

        if let Some(institution) = self.institution.as_deref()
            && event.institution.as_deref() != Some(institution)
        {
            return false;
        }

        if let Some(renderability) = self.renderability.as_deref()
            && event.renderability.as_deref() != Some(renderability)
        {
            return false;
        }

        if let Some(tag) = self.tag.as_deref()
            && !event.tags.iter().any(|event_tag| event_tag == tag)
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

            let matches_properties = event.properties.to_string().to_lowercase().contains(&query);

            if !(matches_text || matches_collection || matches_properties) {
                return false;
            }
        }

        self.expression
            .as_ref()
            .is_none_or(|expression| expression.matches_with_membership(event, context, membership))
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
    pub fn matches(&self, event: &TemporalEvent, context: &QueryContext) -> bool {
        self.matches_with_membership(event, context, None)
    }

    pub fn matches_with_membership(
        &self,
        event: &TemporalEvent,
        context: &QueryContext,
        membership: Option<&EventMembership>,
    ) -> bool {
        match self {
            Self::All(expressions) => expressions
                .iter()
                .all(|expression| expression.matches_with_membership(event, context, membership)),
            Self::Any(expressions) => expressions
                .iter()
                .any(|expression| expression.matches_with_membership(event, context, membership)),
            Self::Not(expression) => {
                !expression.matches_with_membership(event, context, membership)
            }
            Self::Predicate(predicate) => {
                predicate.matches_with_membership(event, context, membership)
            }
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
    TemporalKindAnyOf {
        values: Vec<TemporalKind>,
    },
    DateOverlaps {
        start: Option<NaiveDate>,
        end_exclusive: Option<NaiveDate>,
        #[serde(default)]
        include_imprecise: bool,
    },
    RelativeDateOverlaps {
        start_offset_days: i32,
        end_offset_days_exclusive: i32,
        #[serde(default)]
        include_imprecise: bool,
    },
    UncertainStartOverlaps {
        start: Option<NaiveDate>,
        end_exclusive: Option<NaiveDate>,
    },
    BundleMembership {
        bundle_ref: String,
    },
    ProjectedCalendarMembership {
        calendar_id: String,
    },
    CollectionMembership {
        collection_id: Uuid,
    },
    RelationType {
        relation_type: String,
        #[serde(default)]
        direction: RelationDirection,
    },
    IdentityStateAnyOf {
        values: Vec<EventIdentityState>,
    },
    AnnotationKind {
        annotation_kind: String,
    },
}

impl QueryPredicate {
    pub fn matches(&self, event: &TemporalEvent, context: &QueryContext) -> bool {
        self.matches_with_membership(event, context, None)
    }

    pub fn matches_with_membership(
        &self,
        event: &TemporalEvent,
        context: &QueryContext,
        membership: Option<&EventMembership>,
    ) -> bool {
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
                            !text_matches(candidate, value, TextOperator::Equals, *case_sensitive)
                        })
                } else {
                    values
                        .into_iter()
                        .any(|candidate| text_matches(candidate, value, *operator, *case_sensitive))
                }
            }
            Self::TextAnyOf {
                field,
                values,
                case_sensitive,
            } => text_values(event, *field).into_iter().any(|candidate| {
                values.iter().any(|value| {
                    text_matches(candidate, value, TextOperator::Equals, *case_sensitive)
                })
            }),
            Self::StatusAnyOf { values } => values.contains(&event.status),
            Self::Integer {
                field,
                operator,
                value,
            } => integer_value(event, *field)
                .is_some_and(|candidate| integer_matches(candidate, *value, *operator)),
            Self::Exists { field, exists } => field_exists(event, *field) == *exists,
            Self::TemporalKindAnyOf { values } => values.contains(&TemporalKind::of(&event.time)),
            Self::DateOverlaps {
                start,
                end_exclusive,
                include_imprecise,
            } => event_date_span(&event.time, context.display_timezone, *include_imprecise)
                .is_some_and(|(event_start, event_end_exclusive)| {
                    start.is_none_or(|query_start| event_end_exclusive > query_start)
                        && end_exclusive
                            .is_none_or(|query_end_exclusive| event_start < query_end_exclusive)
                }),
            Self::RelativeDateOverlaps {
                start_offset_days,
                end_offset_days_exclusive,
                include_imprecise,
            } => {
                let start = context
                    .today
                    .checked_add_signed(Duration::days(i64::from(*start_offset_days)));
                let end_exclusive = context
                    .today
                    .checked_add_signed(Duration::days(i64::from(*end_offset_days_exclusive)));

                match (start, end_exclusive) {
                    (Some(start), Some(end_exclusive)) if end_exclusive > start => {
                        event_date_span(&event.time, context.display_timezone, *include_imprecise)
                            .is_some_and(|(event_start, event_end_exclusive)| {
                                event_end_exclusive > start && event_start < end_exclusive
                            })
                    }
                    _ => false,
                }
            }
            Self::UncertainStartOverlaps {
                start,
                end_exclusive,
            } => event
                .time_uncertainty
                .as_ref()
                .and_then(|uncertainty| {
                    uncertainty_start_date_window(uncertainty, context.display_timezone)
                })
                .is_some_and(|(uncertain_earliest, uncertain_latest)| {
                    start.is_none_or(|query_start| uncertain_latest >= query_start)
                        && end_exclusive.is_none_or(|query_end_exclusive| {
                            uncertain_earliest < query_end_exclusive
                        })
                }),
            Self::BundleMembership { bundle_ref } => {
                membership.is_some_and(|membership| membership.belongs_to_bundle(bundle_ref))
            }
            Self::ProjectedCalendarMembership { calendar_id } => {
                membership.is_some_and(|membership| membership.belongs_to_calendar(calendar_id))
            }
            Self::CollectionMembership { collection_id } => membership
                .is_some_and(|membership| membership.belongs_to_collection(*collection_id)),
            Self::RelationType {
                relation_type,
                direction,
            } => membership
                .is_some_and(|membership| membership.has_relation_type(relation_type, *direction)),
            Self::IdentityStateAnyOf { values } => membership.is_some_and(|membership| {
                values
                    .iter()
                    .any(|state| membership.has_identity_state(*state))
            }),
            Self::AnnotationKind { annotation_kind } => {
                membership.is_some_and(|membership| membership.has_annotation_kind(annotation_kind))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemporalKind {
    DateOnly,
    AllDay,
    Instant,
    Floating,
    Month,
    Year,
    Unknown,
}

impl TemporalKind {
    pub const ALL: [Self; 7] = [
        Self::DateOnly,
        Self::AllDay,
        Self::Instant,
        Self::Floating,
        Self::Month,
        Self::Year,
        Self::Unknown,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::DateOnly => "Date only",
            Self::AllDay => "All day",
            Self::Instant => "Exact instant",
            Self::Floating => "Floating / local time",
            Self::Month => "Month precision",
            Self::Year => "Year precision",
            Self::Unknown => "Unresolved",
        }
    }

    pub const fn of(time: &TimeSpec) -> Self {
        match time {
            TimeSpec::DateOnly { .. } => Self::DateOnly,
            TimeSpec::AllDay { .. } => Self::AllDay,
            TimeSpec::Instant { .. } => Self::Instant,
            TimeSpec::Floating { .. } => Self::Floating,
            TimeSpec::Month { .. } => Self::Month,
            TimeSpec::Year { .. } => Self::Year,
            TimeSpec::Unknown { .. } => Self::Unknown,
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

impl TextField {
    pub const ALL: [Self; 15] = [
        Self::Title,
        Self::RawTitle,
        Self::Description,
        Self::EventType,
        Self::Domain,
        Self::Jurisdiction,
        Self::Institution,
        Self::Renderability,
        Self::UpstreamEventRef,
        Self::UpstreamReconciledKey,
        Self::SourceRecordKey,
        Self::Tags,
        Self::SourceRefs,
        Self::AssertionRefs,
        Self::ProvenanceRefs,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Title => "Title",
            Self::RawTitle => "Raw title",
            Self::Description => "Description",
            Self::EventType => "Event type",
            Self::Domain => "Domain",
            Self::Jurisdiction => "Jurisdiction",
            Self::Institution => "Institution",
            Self::Renderability => "Renderability",
            Self::UpstreamEventRef => "Upstream event ref",
            Self::UpstreamReconciledKey => "Reconciled key",
            Self::SourceRecordKey => "Source record key",
            Self::Tags => "Tags",
            Self::SourceRefs => "Source refs",
            Self::AssertionRefs => "Assertion refs",
            Self::ProvenanceRefs => "Provenance refs",
        }
    }
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

impl TextOperator {
    pub const ALL: [Self; 5] = [
        Self::Equals,
        Self::NotEquals,
        Self::Contains,
        Self::StartsWith,
        Self::EndsWith,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Equals => "equals",
            Self::NotEquals => "does not equal",
            Self::Contains => "contains",
            Self::StartsWith => "starts with",
            Self::EndsWith => "ends with",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegerField {
    Importance,
    PersonalRelevance,
}

impl IntegerField {
    pub const ALL: [Self; 2] = [Self::Importance, Self::PersonalRelevance];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Importance => "Importance",
            Self::PersonalRelevance => "Personal relevance",
        }
    }
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

impl IntegerOperator {
    pub const ALL: [Self; 6] = [
        Self::Equal,
        Self::NotEqual,
        Self::LessThan,
        Self::LessThanOrEqual,
        Self::GreaterThan,
        Self::GreaterThanOrEqual,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Equal => "=",
            Self::NotEqual => "≠",
            Self::LessThan => "<",
            Self::LessThanOrEqual => "≤",
            Self::GreaterThan => ">",
            Self::GreaterThanOrEqual => "≥",
        }
    }
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
    TimeUncertainty,
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

impl PresenceField {
    pub const ALL: [Self; 19] = [
        Self::Source,
        Self::RawTitle,
        Self::Description,
        Self::EventType,
        Self::Domain,
        Self::Jurisdiction,
        Self::Institution,
        Self::Renderability,
        Self::Confidence,
        Self::TimeUncertainty,
        Self::Importance,
        Self::PersonalRelevance,
        Self::UpstreamEventRef,
        Self::UpstreamReconciledKey,
        Self::SourceRecordKey,
        Self::Tags,
        Self::SourceRefs,
        Self::AssertionRefs,
        Self::ProvenanceRefs,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Source => "Source",
            Self::RawTitle => "Raw title",
            Self::Description => "Description",
            Self::EventType => "Event type",
            Self::Domain => "Domain",
            Self::Jurisdiction => "Jurisdiction",
            Self::Institution => "Institution",
            Self::Renderability => "Renderability",
            Self::Confidence => "Confidence",
            Self::TimeUncertainty => "Temporal uncertainty",
            Self::Importance => "Importance",
            Self::PersonalRelevance => "Personal relevance",
            Self::UpstreamEventRef => "Upstream event ref",
            Self::UpstreamReconciledKey => "Reconciled key",
            Self::SourceRecordKey => "Source record key",
            Self::Tags => "Tags",
            Self::SourceRefs => "Source refs",
            Self::AssertionRefs => "Assertion refs",
            Self::ProvenanceRefs => "Provenance refs",
        }
    }
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

fn uncertainty_start_date_window(
    uncertainty: &TimeUncertainty,
    display_timezone: Tz,
) -> Option<(NaiveDate, NaiveDate)> {
    match uncertainty {
        TimeUncertainty::DateWindow { earliest, latest } => Some((*earliest, *latest)),
        TimeUncertainty::FloatingWindow { earliest, latest } => {
            Some((earliest.date(), latest.date()))
        }
        TimeUncertainty::InstantWindow {
            earliest_utc,
            latest_utc,
        } => Some((
            earliest_utc.with_timezone(&display_timezone).date_naive(),
            latest_utc.with_timezone(&display_timezone).date_naive(),
        )),
    }
}

fn event_date_span(
    time: &TimeSpec,
    timezone: Tz,
    include_imprecise: bool,
) -> Option<(NaiveDate, NaiveDate)> {
    match time {
        TimeSpec::DateOnly {
            start,
            end_exclusive,
        }
        | TimeSpec::AllDay {
            start,
            end_exclusive,
        } => Some((
            *start,
            end_exclusive.unwrap_or_else(|| start.succ_opt().unwrap_or(*start)),
        )),
        TimeSpec::Instant {
            start_utc, end_utc, ..
        } => {
            let start = start_utc.with_timezone(&timezone);
            let end_exclusive = end_utc
                .map(|end| {
                    let end = end.with_timezone(&timezone);
                    if end.time() == NaiveTime::default() {
                        end.date_naive()
                    } else {
                        end.date_naive()
                            .succ_opt()
                            .unwrap_or_else(|| end.date_naive())
                    }
                })
                .unwrap_or_else(|| {
                    start
                        .date_naive()
                        .succ_opt()
                        .unwrap_or_else(|| start.date_naive())
                });
            Some((start.date_naive(), end_exclusive))
        }
        TimeSpec::Floating { start, end, .. } => {
            let end_exclusive = end
                .map(|end| {
                    if end.time() == NaiveTime::default() {
                        end.date()
                    } else {
                        end.date().succ_opt().unwrap_or_else(|| end.date())
                    }
                })
                .unwrap_or_else(|| start.date().succ_opt().unwrap_or_else(|| start.date()));
            Some((start.date(), end_exclusive))
        }
        TimeSpec::Month { year, month } if include_imprecise => {
            let start = NaiveDate::from_ymd_opt(*year, *month, 1)?;
            Some((start, next_month(start)?))
        }
        TimeSpec::Year { year } if include_imprecise => {
            let start = NaiveDate::from_ymd_opt(*year, 1, 1)?;
            let end = NaiveDate::from_ymd_opt(year.checked_add(1)?, 1, 1)?;
            Some((start, end))
        }
        TimeSpec::Month { .. } | TimeSpec::Year { .. } | TimeSpec::Unknown { .. } => None,
    }
}

fn next_month(start: NaiveDate) -> Option<NaiveDate> {
    if start.month() == 12 {
        NaiveDate::from_ymd_opt(start.year().checked_add(1)?, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(start.year(), start.month() + 1, 1)
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
        PresenceField::TimeUncertainty => event.time_uncertainty.is_some(),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl RgbColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

impl Default for RgbColor {
    fn default() -> Self {
        Self::new(116, 185, 255)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorRule {
    pub id: Uuid,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub when: QueryExpr,
    pub color: RgbColor,
}

impl ColorRule {
    pub fn matches(&self, event: &TemporalEvent, context: &QueryContext) -> bool {
        self.matches_with_membership(event, context, None)
    }

    pub fn matches_with_membership(
        &self,
        event: &TemporalEvent,
        context: &QueryContext,
        membership: Option<&EventMembership>,
    ) -> bool {
        self.enabled
            && self
                .when
                .matches_with_membership(event, context, membership)
    }
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompositionOperator {
    #[default]
    Union,
    Intersect,
    Subtract,
}

impl CompositionOperator {
    pub const ALL: [Self; 3] = [Self::Union, Self::Intersect, Self::Subtract];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Union => "Union",
            Self::Intersect => "Intersect",
            Self::Subtract => "Subtract",
        }
    }

    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Union => "∪",
            Self::Intersect => "∩",
            Self::Subtract => "−",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompositionLayer {
    pub id: Uuid,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub operator: CompositionOperator,
    #[serde(default)]
    pub saved_view_id: Option<Uuid>,
    #[serde(default)]
    pub query: EventQuery,
}

impl CompositionLayer {
    pub fn matches(&self, event: &TemporalEvent, context: &QueryContext) -> bool {
        self.matches_with_membership(event, context, None)
    }

    pub fn matches_with_membership(
        &self,
        event: &TemporalEvent,
        context: &QueryContext,
        membership: Option<&EventMembership>,
    ) -> bool {
        self.enabled
            && self.saved_view_id.is_none()
            && self
                .query
                .matches_with_membership(event, context, membership)
    }
}

pub fn matches_composed_query(
    base: &EventQuery,
    layers: &[CompositionLayer],
    event: &TemporalEvent,
    context: &QueryContext,
) -> bool {
    matches_composed_query_with_membership(base, layers, event, context, None)
}

pub fn matches_composed_query_with_membership(
    base: &EventQuery,
    layers: &[CompositionLayer],
    event: &TemporalEvent,
    context: &QueryContext,
    membership: Option<&EventMembership>,
) -> bool {
    let mut included = base.matches_with_membership(event, context, membership);

    for layer in layers
        .iter()
        .filter(|layer| layer.enabled && layer.saved_view_id.is_none())
    {
        let layer_matches = layer
            .query
            .matches_with_membership(event, context, membership);
        included = match layer.operator {
            CompositionOperator::Union => included || layer_matches,
            CompositionOperator::Intersect => included && layer_matches,
            CompositionOperator::Subtract => included && !layer_matches,
        };
    }

    included
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Overlay {
    pub id: Uuid,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub query: EventQuery,
    #[serde(default)]
    pub color_by: ColorBy,
    #[serde(default)]
    pub color_rules: Vec<ColorRule>,
}

impl Overlay {
    pub fn matches(&self, event: &TemporalEvent, context: &QueryContext) -> bool {
        self.matches_with_membership(event, context, None)
    }

    pub fn matches_with_membership(
        &self,
        event: &TemporalEvent,
        context: &QueryContext,
        membership: Option<&EventMembership>,
    ) -> bool {
        self.enabled
            && self
                .query
                .matches_with_membership(event, context, membership)
    }
}

pub fn matches_composed_or_overlay(
    base: &EventQuery,
    composition_layers: &[CompositionLayer],
    overlays: &[Overlay],
    event: &TemporalEvent,
    context: &QueryContext,
) -> bool {
    matches_composed_or_overlay_with_membership(
        base,
        composition_layers,
        overlays,
        event,
        context,
        None,
    )
}

pub fn matches_composed_or_overlay_with_membership(
    base: &EventQuery,
    composition_layers: &[CompositionLayer],
    overlays: &[Overlay],
    event: &TemporalEvent,
    context: &QueryContext,
    membership: Option<&EventMembership>,
) -> bool {
    matches_composed_query_with_membership(base, composition_layers, event, context, membership)
        || overlays
            .iter()
            .any(|overlay| overlay.matches_with_membership(event, context, membership))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableColumn {
    Date,
    Time,
    Title,
    EventType,
    Domain,
    Jurisdiction,
    Institution,
    Status,
    Importance,
    PersonalRelevance,
    Source,
    Renderability,
    Confidence,
    Tags,
    UpstreamEventRef,
    ReconciledEventRef,
}

impl TableColumn {
    pub const ALL: [Self; 16] = [
        Self::Date,
        Self::Time,
        Self::Title,
        Self::EventType,
        Self::Domain,
        Self::Jurisdiction,
        Self::Institution,
        Self::Status,
        Self::Importance,
        Self::PersonalRelevance,
        Self::Source,
        Self::Renderability,
        Self::Confidence,
        Self::Tags,
        Self::UpstreamEventRef,
        Self::ReconciledEventRef,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Date => "Date",
            Self::Time => "Time / precision",
            Self::Title => "Title",
            Self::EventType => "Type",
            Self::Domain => "Domain",
            Self::Jurisdiction => "Jurisdiction",
            Self::Institution => "Institution",
            Self::Status => "Status",
            Self::Importance => "Importance",
            Self::PersonalRelevance => "Relevance",
            Self::Source => "Source",
            Self::Renderability => "Renderability",
            Self::Confidence => "Confidence",
            Self::Tags => "Tags",
            Self::UpstreamEventRef => "Event ref",
            Self::ReconciledEventRef => "Reconciled ref",
        }
    }
}

pub fn default_table_columns() -> Vec<TableColumn> {
    vec![
        TableColumn::Date,
        TableColumn::Time,
        TableColumn::Title,
        TableColumn::EventType,
        TableColumn::Domain,
        TableColumn::Jurisdiction,
        TableColumn::Institution,
        TableColumn::Status,
        TableColumn::Importance,
        TableColumn::PersonalRelevance,
        TableColumn::Source,
    ]
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
    #[serde(default)]
    pub color_rules: Vec<ColorRule>,
    #[serde(default)]
    pub composition_layers: Vec<CompositionLayer>,
    #[serde(default)]
    pub overlays: Vec<Overlay>,
    #[serde(default = "default_table_columns")]
    pub table_columns: Vec<TableColumn>,
    pub display_timezone: String,
    pub week_start_monday: bool,
}

struct SavedViewMatchContext<'a> {
    saved_views: &'a [SavedView],
    event: &'a TemporalEvent,
    query_context: &'a QueryContext,
    membership: Option<&'a EventMembership>,
}

pub fn matches_composed_or_overlay_with_saved_views_and_membership(
    base: &EventQuery,
    composition_layers: &[CompositionLayer],
    overlays: &[Overlay],
    saved_views: &[SavedView],
    event: &TemporalEvent,
    context: &QueryContext,
    membership: Option<&EventMembership>,
) -> bool {
    let evaluation = SavedViewMatchContext {
        saved_views,
        event,
        query_context: context,
        membership,
    };
    let mut visiting = BTreeSet::new();
    matches_composed_or_overlay_with_references(
        base,
        composition_layers,
        overlays,
        &evaluation,
        &mut visiting,
    )
}

fn matches_composed_or_overlay_with_references(
    base: &EventQuery,
    composition_layers: &[CompositionLayer],
    overlays: &[Overlay],
    evaluation: &SavedViewMatchContext<'_>,
    visiting: &mut BTreeSet<Uuid>,
) -> bool {
    let mut included = base.matches_with_membership(
        evaluation.event,
        evaluation.query_context,
        evaluation.membership,
    );

    for layer in composition_layers.iter().filter(|layer| layer.enabled) {
        let layer_matches = if let Some(saved_view_id) = layer.saved_view_id {
            referenced_saved_view_matches(saved_view_id, evaluation, visiting)
        } else {
            Some(layer.query.matches_with_membership(
                evaluation.event,
                evaluation.query_context,
                evaluation.membership,
            ))
        };

        let Some(layer_matches) = layer_matches else {
            continue;
        };

        included = match layer.operator {
            CompositionOperator::Union => included || layer_matches,
            CompositionOperator::Intersect => included && layer_matches,
            CompositionOperator::Subtract => included && !layer_matches,
        };
    }

    included
        || overlays.iter().any(|overlay| {
            overlay.matches_with_membership(
                evaluation.event,
                evaluation.query_context,
                evaluation.membership,
            )
        })
}

fn referenced_saved_view_matches(
    saved_view_id: Uuid,
    evaluation: &SavedViewMatchContext<'_>,
    visiting: &mut BTreeSet<Uuid>,
) -> Option<bool> {
    let view = evaluation
        .saved_views
        .iter()
        .find(|view| view.id == saved_view_id)?;

    if !visiting.insert(saved_view_id) {
        return None;
    }

    let matches = evaluation
        .event
        .source_id
        .is_none_or(|source_id| !view.hidden_source_ids.contains(&source_id))
        && matches_composed_or_overlay_with_references(
            &view.query,
            &view.composition_layers,
            &view.overlays,
            evaluation,
            visiting,
        );

    visiting.remove(&saved_view_id);
    Some(matches)
}

pub fn saved_view_reference_cycle(saved_views: &[SavedView], root_id: Uuid) -> Option<Vec<Uuid>> {
    fn visit(
        saved_views: &[SavedView],
        id: Uuid,
        stack: &mut Vec<Uuid>,
        visited: &mut BTreeSet<Uuid>,
    ) -> Option<Vec<Uuid>> {
        if let Some(position) = stack.iter().position(|candidate| *candidate == id) {
            let mut cycle = stack[position..].to_vec();
            cycle.push(id);
            return Some(cycle);
        }

        if !visited.insert(id) {
            return None;
        }

        let view = saved_views.iter().find(|view| view.id == id)?;

        stack.push(id);
        for referenced_id in view
            .composition_layers
            .iter()
            .filter_map(|layer| layer.saved_view_id)
        {
            if let Some(cycle) = visit(saved_views, referenced_id, stack, visited) {
                return Some(cycle);
            }
        }
        stack.pop();
        None
    }

    visit(saved_views, root_id, &mut Vec::new(), &mut BTreeSet::new())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use chrono::NaiveDate;

    use super::*;
    use crate::domain::{TemporalEvent, TimeSpec, TimeUncertainty};

    fn test_context() -> QueryContext {
        QueryContext::new(
            chrono_tz::America::Mexico_City,
            NaiveDate::from_ymd_opt(2026, 10, 4).expect("today"),
        )
    }

    fn event_with_time(time: TimeSpec) -> TemporalEvent {
        let mut event = TemporalEvent::new("California General Election", time);
        event.domain = Some("elections".to_string());
        event.jurisdiction = Some("US-CA".to_string());
        event.event_type = Some("election".to_string());
        event.institution = Some("California Secretary of State".to_string());
        event.renderability = Some("ready".to_string());
        event.status = EventStatus::Confirmed;
        event.importance = Some(90);
        event.tags = vec!["state".to_string(), "general".to_string()];
        event.source_refs = vec!["ca-sos".to_string()];
        event
    }

    fn event() -> TemporalEvent {
        event_with_time(TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 11, 3).expect("date"),
            end_exclusive: None,
        })
    }

    #[test]
    fn legacy_query_json_without_expression_still_loads() {
        let raw = r#"{
            "text": "election",
            "domain": "politics",
            "jurisdiction": "US",
            "status": "confirmed"
        }"#;

        let query: EventQuery = serde_json::from_str(raw).expect("legacy query");
        assert_eq!(query.text, "election");
        assert_eq!(query.domain.as_deref(), Some("politics"));
        assert_eq!(query.jurisdiction.as_deref(), Some("US"));
        assert_eq!(query.status, Some(EventStatus::Confirmed));
        assert!(query.expression.is_none());
    }

    #[test]
    fn query_dimensions_compose() {
        let query = EventQuery {
            text: "general".to_string(),
            domain: Some("elections".to_string()),
            jurisdiction: Some("US-CA".to_string()),
            event_type: Some("election".to_string()),
            institution: Some("California Secretary of State".to_string()),
            renderability: Some("ready".to_string()),
            tag: Some("general".to_string()),
            status: Some(EventStatus::Confirmed),
            expression: None,
        };

        assert!(query.matches(&event(), &test_context()));
    }

    #[test]
    fn richer_simple_facets_match_exact_event_dimensions() {
        let matching = EventQuery {
            event_type: Some("election".to_string()),
            institution: Some("California Secretary of State".to_string()),
            renderability: Some("ready".to_string()),
            tag: Some("state".to_string()),
            ..EventQuery::default()
        };
        assert!(matching.matches(&event(), &test_context()));

        let wrong_tag = EventQuery {
            tag: Some("federal".to_string()),
            ..EventQuery::default()
        };
        assert!(!wrong_tag.matches(&event(), &test_context()));
    }

    #[test]
    fn wrong_facet_rejects_event() {
        let query = EventQuery {
            domain: Some("economics".to_string()),
            ..EventQuery::default()
        };

        assert!(!query.matches(&event(), &test_context()));
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

        assert!(query.matches(&event(), &test_context()));
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

        assert!(!query.matches(&event(), &test_context()));
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

        assert!(query.matches(&event(), &test_context()));
    }

    #[test]
    fn temporal_uncertainty_is_queryable_as_independent_presence() {
        let mut event = event_with_time(TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 10, 7).expect("date"),
            end_exclusive: None,
        });
        let context = QueryContext::new(
            chrono_tz::America::Mexico_City,
            NaiveDate::from_ymd_opt(2026, 10, 7).expect("today"),
        );
        let predicate = QueryPredicate::Exists {
            field: PresenceField::TimeUncertainty,
            exists: true,
        };

        assert!(!predicate.matches(&event, &context));
        event.time_uncertainty = Some(crate::domain::TimeUncertainty::DateWindow {
            earliest: NaiveDate::from_ymd_opt(2026, 10, 5).expect("earliest"),
            latest: NaiveDate::from_ymd_opt(2026, 10, 9).expect("latest"),
        });
        assert!(predicate.matches(&event, &context));
        assert_eq!(event.time.kind_name(), "date_only");
    }

    #[test]
    fn uncertain_start_overlap_matches_possible_start_window_not_anchor_only() {
        let canonical_day = NaiveDate::from_ymd_opt(2026, 10, 10).expect("canonical");
        let mut uncertain = event_with_time(TimeSpec::DateOnly {
            start: canonical_day,
            end_exclusive: None,
        });
        uncertain.time_uncertainty = Some(TimeUncertainty::DateWindow {
            earliest: NaiveDate::from_ymd_opt(2026, 10, 5).expect("earliest"),
            latest: NaiveDate::from_ymd_opt(2026, 10, 12).expect("latest"),
        });

        let uncertainty_query = QueryPredicate::UncertainStartOverlaps {
            start: Some(NaiveDate::from_ymd_opt(2026, 10, 5).expect("start")),
            end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 10, 7).expect("end")),
        };
        assert!(uncertainty_query.matches(&uncertain, &test_context()));

        let ordinary_overlap = QueryPredicate::DateOverlaps {
            start: Some(NaiveDate::from_ymd_opt(2026, 10, 5).expect("start")),
            end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 10, 7).expect("end")),
            include_imprecise: false,
        };
        assert!(!ordinary_overlap.matches(&uncertain, &test_context()));
    }

    #[test]
    fn uncertain_instant_start_overlap_uses_display_timezone() {
        let anchor = chrono::DateTime::parse_from_rfc3339("2026-10-05T01:00:00Z")
            .expect("anchor")
            .with_timezone(&Utc);
        let mut uncertain = event_with_time(TimeSpec::Instant {
            start_utc: anchor,
            end_utc: None,
            source_timezone: Some("UTC".to_string()),
        });
        uncertain.time_uncertainty = Some(TimeUncertainty::InstantWindow {
            earliest_utc: chrono::DateTime::parse_from_rfc3339("2026-10-05T00:30:00Z")
                .expect("earliest")
                .with_timezone(&Utc),
            latest_utc: chrono::DateTime::parse_from_rfc3339("2026-10-05T02:00:00Z")
                .expect("latest")
                .with_timezone(&Utc),
        });
        let predicate = QueryPredicate::UncertainStartOverlaps {
            start: Some(NaiveDate::from_ymd_opt(2026, 10, 4).expect("start")),
            end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 10, 5).expect("end")),
        };

        assert!(predicate.matches(&uncertain, &test_context()));

        let tokyo = QueryContext::new(
            chrono_tz::Asia::Tokyo,
            NaiveDate::from_ymd_opt(2026, 10, 4).expect("today"),
        );
        assert!(!predicate.matches(&uncertain, &tokyo));
    }

    #[test]
    fn temporal_kind_predicate_preserves_precision_classes() {
        let query = EventQuery {
            expression: Some(QueryExpr::Predicate(QueryPredicate::TemporalKindAnyOf {
                values: vec![TemporalKind::DateOnly],
            })),
            ..EventQuery::default()
        };

        assert!(query.matches(&event(), &test_context()));

        let mut month_event = event();
        month_event.time = TimeSpec::Month {
            year: 2026,
            month: 11,
        };
        assert!(!query.matches(&month_event, &test_context()));
    }

    #[test]
    fn date_overlap_is_timezone_aware_for_exact_instants() {
        let instant = chrono::DateTime::parse_from_rfc3339("2026-10-05T01:00:00Z")
            .expect("instant")
            .with_timezone(&Utc);
        let mut event = event();
        event.time = TimeSpec::Instant {
            start_utc: instant,
            end_utc: None,
            source_timezone: Some("UTC".to_string()),
        };

        let query = EventQuery {
            expression: Some(QueryExpr::Predicate(QueryPredicate::DateOverlaps {
                start: Some(NaiveDate::from_ymd_opt(2026, 10, 4).expect("start")),
                end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 10, 5).expect("end")),
                include_imprecise: false,
            })),
            ..EventQuery::default()
        };

        assert!(query.matches(&event, &test_context()));
        let tokyo = QueryContext::new(
            chrono_tz::Asia::Tokyo,
            NaiveDate::from_ymd_opt(2026, 10, 4).expect("today"),
        );
        assert!(!query.matches(&event, &tokyo));
    }

    #[test]
    fn date_overlap_does_not_invent_day_for_imprecise_events() {
        let mut month_event = event();
        month_event.time = TimeSpec::Month {
            year: 2026,
            month: 11,
        };

        let strict = EventQuery {
            expression: Some(QueryExpr::Predicate(QueryPredicate::DateOverlaps {
                start: Some(NaiveDate::from_ymd_opt(2026, 11, 1).expect("start")),
                end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 12, 1).expect("end")),
                include_imprecise: false,
            })),
            ..EventQuery::default()
        };
        assert!(!strict.matches(&month_event, &test_context()));

        let inclusive = EventQuery {
            expression: Some(QueryExpr::Predicate(QueryPredicate::DateOverlaps {
                start: Some(NaiveDate::from_ymd_opt(2026, 11, 1).expect("start")),
                end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 12, 1).expect("end")),
                include_imprecise: true,
            })),
            ..EventQuery::default()
        };
        assert!(inclusive.matches(&month_event, &test_context()));
    }

    #[test]
    fn relative_date_window_uses_explicit_today_anchor() {
        let query = EventQuery {
            expression: Some(QueryExpr::Predicate(QueryPredicate::RelativeDateOverlaps {
                start_offset_days: 1,
                end_offset_days_exclusive: 31,
                include_imprecise: false,
            })),
            ..EventQuery::default()
        };

        let mut future = event();
        future.time = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 11, 3).expect("future"),
            end_exclusive: None,
        };
        assert!(query.matches(&future, &test_context()));

        let mut today = event();
        today.time = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 10, 4).expect("today"),
            end_exclusive: None,
        };
        assert!(!query.matches(&today, &test_context()));
    }

    #[test]
    fn invalid_relative_date_window_matches_nothing() {
        let query = EventQuery {
            expression: Some(QueryExpr::Predicate(QueryPredicate::RelativeDateOverlaps {
                start_offset_days: 10,
                end_offset_days_exclusive: 5,
                include_imprecise: true,
            })),
            ..EventQuery::default()
        };

        assert!(!query.matches(&event(), &test_context()));
    }

    #[test]
    fn composition_layers_apply_union_intersection_and_subtraction_in_order() {
        let base = EventQuery {
            domain: Some("economics".to_string()),
            ..EventQuery::default()
        };
        let layers = vec![
            CompositionLayer {
                id: Uuid::new_v4(),
                name: "Add California".to_string(),
                enabled: true,
                operator: CompositionOperator::Union,
                saved_view_id: None,
                query: EventQuery {
                    jurisdiction: Some("US-CA".to_string()),
                    ..EventQuery::default()
                },
            },
            CompositionLayer {
                id: Uuid::new_v4(),
                name: "Require confirmed".to_string(),
                enabled: true,
                operator: CompositionOperator::Intersect,
                saved_view_id: None,
                query: EventQuery {
                    status: Some(EventStatus::Confirmed),
                    ..EventQuery::default()
                },
            },
            CompositionLayer {
                id: Uuid::new_v4(),
                name: "Exclude federal".to_string(),
                enabled: true,
                operator: CompositionOperator::Subtract,
                saved_view_id: None,
                query: EventQuery {
                    expression: Some(QueryExpr::Predicate(QueryPredicate::Text {
                        field: TextField::Tags,
                        operator: TextOperator::Equals,
                        value: "federal".to_string(),
                        case_sensitive: false,
                    })),
                    ..EventQuery::default()
                },
            },
        ];

        assert!(matches_composed_query(
            &base,
            &layers,
            &event(),
            &test_context()
        ));

        let mut federal = event();
        federal.tags.push("federal".to_string());
        assert!(!matches_composed_query(
            &base,
            &layers,
            &federal,
            &test_context()
        ));
    }

    #[test]
    fn disabled_composition_layer_has_no_effect() {
        let base = EventQuery {
            jurisdiction: Some("US-CA".to_string()),
            ..EventQuery::default()
        };
        let layers = vec![CompositionLayer {
            id: Uuid::new_v4(),
            name: "Disabled exclusion".to_string(),
            enabled: false,
            operator: CompositionOperator::Subtract,
            saved_view_id: None,
            query: EventQuery {
                jurisdiction: Some("US-CA".to_string()),
                ..EventQuery::default()
            },
        }];

        assert!(matches_composed_query(
            &base,
            &layers,
            &event(),
            &test_context()
        ));
    }

    #[test]
    fn overlay_can_include_event_excluded_by_base_query() {
        let base = EventQuery {
            domain: Some("economics".to_string()),
            ..EventQuery::default()
        };
        let overlays = vec![Overlay {
            id: Uuid::new_v4(),
            name: "California".to_string(),
            enabled: true,
            query: EventQuery {
                jurisdiction: Some("US-CA".to_string()),
                ..EventQuery::default()
            },
            color_by: ColorBy::Jurisdiction,
            color_rules: Vec::new(),
        }];

        assert!(matches_composed_or_overlay(
            &base,
            &[],
            &overlays,
            &event(),
            &test_context()
        ));
    }

    #[test]
    fn membership_predicates_use_external_query_context() {
        let event = event();
        let mut membership = EventMembership::default();
        membership
            .bundle_refs
            .insert("bundle:temporal/politics-government".to_string());
        membership
            .calendar_refs
            .insert("projected-calendar:us-politics".to_string());
        let collection_id = Uuid::new_v4();
        membership.collection_ids.insert(collection_id);
        membership
            .outgoing_relation_types
            .insert("causes".to_string());
        membership
            .incoming_relation_types
            .insert("follows".to_string());

        let bundle_query = EventQuery {
            expression: Some(QueryExpr::Predicate(QueryPredicate::BundleMembership {
                bundle_ref: "bundle:temporal/politics-government".to_string(),
            })),
            ..EventQuery::default()
        };
        let calendar_query = EventQuery {
            expression: Some(QueryExpr::Predicate(
                QueryPredicate::ProjectedCalendarMembership {
                    calendar_id: "projected-calendar:us-politics".to_string(),
                },
            )),
            ..EventQuery::default()
        };

        let collection_query = EventQuery {
            expression: Some(QueryExpr::Predicate(QueryPredicate::CollectionMembership {
                collection_id,
            })),
            ..EventQuery::default()
        };
        let outgoing_relation_query = EventQuery {
            expression: Some(QueryExpr::Predicate(QueryPredicate::RelationType {
                relation_type: "causes".to_string(),
                direction: RelationDirection::Outgoing,
            })),
            ..EventQuery::default()
        };
        let incoming_relation_query = EventQuery {
            expression: Some(QueryExpr::Predicate(QueryPredicate::RelationType {
                relation_type: "follows".to_string(),
                direction: RelationDirection::Incoming,
            })),
            ..EventQuery::default()
        };
        let either_relation_query = EventQuery {
            expression: Some(QueryExpr::Predicate(QueryPredicate::RelationType {
                relation_type: "causes".to_string(),
                direction: RelationDirection::Either,
            })),
            ..EventQuery::default()
        };

        assert!(!bundle_query.matches(&event, &test_context()));
        assert!(bundle_query.matches_with_membership(&event, &test_context(), Some(&membership)));
        assert!(calendar_query.matches_with_membership(&event, &test_context(), Some(&membership)));
        assert!(collection_query.matches_with_membership(
            &event,
            &test_context(),
            Some(&membership)
        ));
        assert!(outgoing_relation_query.matches_with_membership(
            &event,
            &test_context(),
            Some(&membership)
        ));
        assert!(incoming_relation_query.matches_with_membership(
            &event,
            &test_context(),
            Some(&membership)
        ));
        assert!(either_relation_query.matches_with_membership(
            &event,
            &test_context(),
            Some(&membership)
        ));
    }

    #[test]
    fn annotation_kind_predicate_uses_external_user_metadata_context() {
        let event = event();
        let mut membership = EventMembership::default();
        membership.annotation_kinds.insert("note".to_string());
        membership.annotation_kinds.insert("watched".to_string());

        let note_query = EventQuery {
            expression: Some(QueryExpr::Predicate(QueryPredicate::AnnotationKind {
                annotation_kind: "note".to_string(),
            })),
            ..EventQuery::default()
        };
        let missing_query = EventQuery {
            expression: Some(QueryExpr::Predicate(QueryPredicate::AnnotationKind {
                annotation_kind: "rating".to_string(),
            })),
            ..EventQuery::default()
        };

        assert!(!note_query.matches(&event, &test_context()));
        assert!(note_query.matches_with_membership(&event, &test_context(), Some(&membership)));
        assert!(!missing_query.matches_with_membership(&event, &test_context(), Some(&membership)));
    }

    #[test]
    fn identity_state_predicate_uses_external_assessment_context() {
        let event = event();
        let mut membership = EventMembership::default();
        membership
            .identity_states
            .insert(EventIdentityState::Candidate.as_str().to_string());
        membership
            .identity_states
            .insert(EventIdentityState::Distinct.as_str().to_string());

        let candidate_query = EventQuery {
            expression: Some(QueryExpr::Predicate(QueryPredicate::IdentityStateAnyOf {
                values: vec![EventIdentityState::Candidate],
            })),
            ..EventQuery::default()
        };
        let resolved_query = EventQuery {
            expression: Some(QueryExpr::Predicate(QueryPredicate::IdentityStateAnyOf {
                values: vec![EventIdentityState::SameEvent],
            })),
            ..EventQuery::default()
        };
        let either_query = EventQuery {
            expression: Some(QueryExpr::Predicate(QueryPredicate::IdentityStateAnyOf {
                values: vec![EventIdentityState::SameEvent, EventIdentityState::Distinct],
            })),
            ..EventQuery::default()
        };

        assert!(!candidate_query.matches(&event, &test_context()));
        assert!(candidate_query.matches_with_membership(
            &event,
            &test_context(),
            Some(&membership)
        ));
        assert!(!resolved_query.matches_with_membership(
            &event,
            &test_context(),
            Some(&membership)
        ));
        assert!(either_query.matches_with_membership(&event, &test_context(), Some(&membership)));
    }

    #[test]
    fn composition_layer_can_reference_saved_view_logical_set() {
        let referenced = SavedView {
            id: Uuid::new_v4(),
            name: "California".to_string(),
            query: EventQuery {
                jurisdiction: Some("US-CA".to_string()),
                ..EventQuery::default()
            },
            hidden_source_ids: BTreeSet::new(),
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
        };

        let layer = CompositionLayer {
            id: Uuid::new_v4(),
            name: "Reuse California".to_string(),
            enabled: true,
            operator: CompositionOperator::Union,
            saved_view_id: Some(referenced.id),
            query: EventQuery {
                domain: Some("does-not-match".to_string()),
                ..EventQuery::default()
            },
        };

        assert!(matches_composed_or_overlay_with_saved_views_and_membership(
            &EventQuery {
                domain: Some("does-not-match".to_string()),
                ..EventQuery::default()
            },
            &[layer],
            &[],
            &[referenced],
            &event(),
            &test_context(),
            None,
        ));
    }

    #[test]
    fn saved_view_reference_reuses_overlay_and_source_visibility() {
        let source_id = Uuid::new_v4();
        let mut hidden_sources = BTreeSet::new();
        hidden_sources.insert(source_id);

        let referenced = SavedView {
            id: Uuid::new_v4(),
            name: "Overlay-backed".to_string(),
            query: EventQuery {
                domain: Some("does-not-match".to_string()),
                ..EventQuery::default()
            },
            hidden_source_ids: hidden_sources,
            calendar_view: CalendarView::Month,
            calendar_layout: CalendarLayout::Grid,
            group_by: GroupBy::Date,
            sort_rules: vec![SortRule::default()],
            color_by: ColorBy::Status,
            color_rules: Vec::new(),
            composition_layers: Vec::new(),
            overlays: vec![Overlay {
                id: Uuid::new_v4(),
                name: "California".to_string(),
                enabled: true,
                query: EventQuery {
                    jurisdiction: Some("US-CA".to_string()),
                    ..EventQuery::default()
                },
                color_by: ColorBy::Jurisdiction,
                color_rules: Vec::new(),
            }],
            table_columns: default_table_columns(),
            display_timezone: "America/Mexico_City".to_string(),
            week_start_monday: false,
        };

        let layer = CompositionLayer {
            id: Uuid::new_v4(),
            name: "Reference".to_string(),
            enabled: true,
            operator: CompositionOperator::Union,
            saved_view_id: Some(referenced.id),
            query: EventQuery::default(),
        };

        let mut sourced_event = event();
        sourced_event.source_id = Some(source_id);

        assert!(
            !matches_composed_or_overlay_with_saved_views_and_membership(
                &EventQuery {
                    domain: Some("does-not-match".to_string()),
                    ..EventQuery::default()
                },
                std::slice::from_ref(&layer),
                &[],
                std::slice::from_ref(&referenced),
                &sourced_event,
                &test_context(),
                None,
            )
        );

        let mut visible_reference = referenced;
        visible_reference.hidden_source_ids.clear();
        assert!(matches_composed_or_overlay_with_saved_views_and_membership(
            &EventQuery {
                domain: Some("does-not-match".to_string()),
                ..EventQuery::default()
            },
            &[layer],
            &[],
            &[visible_reference],
            &sourced_event,
            &test_context(),
            None,
        ));
    }

    #[test]
    fn missing_saved_view_reference_is_a_no_op() {
        let layer = CompositionLayer {
            id: Uuid::new_v4(),
            name: "Missing".to_string(),
            enabled: true,
            operator: CompositionOperator::Subtract,
            saved_view_id: Some(Uuid::new_v4()),
            query: EventQuery::default(),
        };

        assert!(matches_composed_or_overlay_with_saved_views_and_membership(
            &EventQuery {
                jurisdiction: Some("US-CA".to_string()),
                ..EventQuery::default()
            },
            &[layer],
            &[],
            &[],
            &event(),
            &test_context(),
            None,
        ));
    }

    #[test]
    fn saved_view_reference_cycles_are_detected_and_runtime_safe() {
        let a_id = Uuid::new_v4();
        let b_id = Uuid::new_v4();

        let make_view = |id, name: &str, referenced_id| SavedView {
            id,
            name: name.to_string(),
            query: EventQuery {
                jurisdiction: Some("US-CA".to_string()),
                ..EventQuery::default()
            },
            hidden_source_ids: BTreeSet::new(),
            calendar_view: CalendarView::Month,
            calendar_layout: CalendarLayout::Grid,
            group_by: GroupBy::Date,
            sort_rules: vec![SortRule::default()],
            color_by: ColorBy::Status,
            color_rules: Vec::new(),
            composition_layers: vec![CompositionLayer {
                id: Uuid::new_v4(),
                name: "Reference".to_string(),
                enabled: true,
                operator: CompositionOperator::Union,
                saved_view_id: Some(referenced_id),
                query: EventQuery::default(),
            }],
            overlays: Vec::new(),
            table_columns: default_table_columns(),
            display_timezone: "America/Mexico_City".to_string(),
            week_start_monday: false,
        };

        let a = make_view(a_id, "A", b_id);
        let b = make_view(b_id, "B", a_id);
        let views = vec![a.clone(), b];

        let cycle = saved_view_reference_cycle(&views, a_id).expect("cycle");
        assert_eq!(cycle.first(), Some(&a_id));
        assert_eq!(cycle.last(), Some(&a_id));

        assert!(matches_composed_or_overlay_with_saved_views_and_membership(
            &EventQuery::default(),
            &[CompositionLayer {
                id: Uuid::new_v4(),
                name: "A".to_string(),
                enabled: true,
                operator: CompositionOperator::Intersect,
                saved_view_id: Some(a.id),
                query: EventQuery::default(),
            }],
            &[],
            &views,
            &event(),
            &test_context(),
            None,
        ));
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
            color_rules: vec![ColorRule {
                id: Uuid::new_v4(),
                name: "High importance".to_string(),
                enabled: true,
                when: QueryExpr::Predicate(QueryPredicate::Integer {
                    field: IntegerField::Importance,
                    operator: IntegerOperator::GreaterThanOrEqual,
                    value: 80,
                }),
                color: RgbColor::new(255, 80, 80),
            }],
            composition_layers: vec![CompositionLayer {
                id: Uuid::new_v4(),
                name: "Exclude cancelled".to_string(),
                enabled: true,
                operator: CompositionOperator::Subtract,
                saved_view_id: None,
                query: EventQuery {
                    status: Some(EventStatus::Cancelled),
                    ..EventQuery::default()
                },
            }],
            table_columns: vec![
                TableColumn::Title,
                TableColumn::Date,
                TableColumn::Jurisdiction,
            ],
            overlays: vec![Overlay {
                id: Uuid::new_v4(),
                name: "Federal".to_string(),
                enabled: true,
                query: EventQuery {
                    jurisdiction: Some("US".to_string()),
                    ..EventQuery::default()
                },
                color_by: ColorBy::Domain,
                color_rules: Vec::new(),
            }],
            display_timezone: "America/Mexico_City".to_string(),
            week_start_monday: false,
        };

        assert_eq!(view.name, "California Elections");
        assert_eq!(view.calendar_layout, CalendarLayout::Agenda);
        assert_eq!(view.group_by, GroupBy::Jurisdiction);
        assert_eq!(view.color_by, ColorBy::EventType);
        assert_eq!(view.color_rules.len(), 1);
        assert!(view.color_rules[0].matches(&event(), &test_context()));
        assert_eq!(view.composition_layers.len(), 1);
        assert_eq!(view.overlays.len(), 1);
        assert_eq!(view.sort_rules.len(), 1);
        assert_eq!(
            view.table_columns,
            vec![
                TableColumn::Title,
                TableColumn::Date,
                TableColumn::Jurisdiction,
            ]
        );
        assert!(!view.id.is_nil());
    }
}
