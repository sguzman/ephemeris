use std::{collections::HashSet, fmt};

use chrono::{
    DateTime, Datelike, Days, Duration, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc,
};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    Announced,
    Tentative,
    #[default]
    Scheduled,
    Confirmed,
    Rescheduled,
    Postponed,
    Cancelled,
    Completed,
    Observed,
    Superseded,
    Estimated,
    Projected,
    Disputed,
    Unknown,
}

impl EventStatus {
    pub const ALL: [Self; 14] = [
        Self::Announced,
        Self::Tentative,
        Self::Scheduled,
        Self::Confirmed,
        Self::Rescheduled,
        Self::Postponed,
        Self::Cancelled,
        Self::Completed,
        Self::Observed,
        Self::Superseded,
        Self::Estimated,
        Self::Projected,
        Self::Disputed,
        Self::Unknown,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Announced => "announced",
            Self::Tentative => "tentative",
            Self::Scheduled => "scheduled",
            Self::Confirmed => "confirmed",
            Self::Rescheduled => "rescheduled",
            Self::Postponed => "postponed",
            Self::Cancelled => "cancelled",
            Self::Completed => "completed",
            Self::Observed => "observed",
            Self::Superseded => "superseded",
            Self::Estimated => "estimated",
            Self::Projected => "projected",
            Self::Disputed => "disputed",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let normalized = raw.trim().to_ascii_lowercase().replace('-', "_");
        Self::ALL
            .into_iter()
            .find(|status| status.as_str() == normalized)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceAuthority {
    Official,
    FirstParty,
    Secondary,
    Aggregator,
    Manual,
    Derived,
    Unknown,
}

impl SourceAuthority {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Official => "official",
            Self::FirstParty => "first_party",
            Self::Secondary => "secondary",
            Self::Aggregator => "aggregator",
            Self::Manual => "manual",
            Self::Derived => "derived",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "official" => Self::Official,
            "first_party" => Self::FirstParty,
            "secondary" => Self::Secondary,
            "aggregator" => Self::Aggregator,
            "manual" => Self::Manual,
            "derived" | "resourcearium_derived" => Self::Derived,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Taria,
    Ics,
    Webcal,
    Api,
    CalDav,
    Json,
    Csv,
    Manual,
    Derived,
    Unknown,
}

impl SourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Taria => "taria",
            Self::Ics => "ics",
            Self::Webcal => "webcal",
            Self::Api => "api",
            Self::CalDav => "caldav",
            Self::Json => "json",
            Self::Csv => "csv",
            Self::Manual => "manual",
            Self::Derived => "derived",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "taria" | "resourcearium" => Self::Taria,
            "ics" | "ical" | "icalendar" => Self::Ics,
            "webcal" | "webcals" => Self::Webcal,
            "api" => Self::Api,
            "caldav" => Self::CalDav,
            "json" | "jscalendar" | "jcal" => Self::Json,
            "csv" => Self::Csv,
            "manual" => Self::Manual,
            "derived" => Self::Derived,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TimeSpec {
    /// A source supplied a civil date, but did not assert that it occupied the full day.
    DateOnly {
        start: NaiveDate,
        end_exclusive: Option<NaiveDate>,
    },
    /// A source explicitly supplied all-day semantics.
    AllDay {
        start: NaiveDate,
        end_exclusive: Option<NaiveDate>,
    },
    /// An exact global instant, optionally with source timezone context retained.
    Instant {
        start_utc: DateTime<Utc>,
        end_utc: Option<DateTime<Utc>>,
        source_timezone: Option<String>,
    },
    /// A local wall-clock value intentionally not converted to an instant.
    Floating {
        start: NaiveDateTime,
        end: Option<NaiveDateTime>,
        source_timezone: Option<String>,
    },
    /// The event is known only to a calendar month.
    Month { year: i32, month: u32 },
    /// The event is known only to a calendar year.
    Year { year: i32 },
    /// Taria retained the event, but no renderable temporal placement is available.
    Unknown { original_value: Option<String> },
}

impl TimeSpec {
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::DateOnly { .. } => "date_only",
            Self::AllDay { .. } => "all_day",
            Self::Instant { .. } => "instant",
            Self::Floating { .. } => "floating",
            Self::Month { .. } => "month",
            Self::Year { .. } => "year",
            Self::Unknown { .. } => "unknown",
        }
    }

    /// Returns an exact display date only when the source precision supports one.
    pub fn display_date(&self, timezone: Tz) -> Option<NaiveDate> {
        match self {
            Self::DateOnly { start, .. } => Some(*start),
            Self::AllDay { start, .. } => Some(*start),
            Self::Instant { start_utc, .. } => {
                Some(start_utc.with_timezone(&timezone).date_naive())
            }
            Self::Floating { start, .. } => Some(start.date()),
            Self::Month { .. } | Self::Year { .. } | Self::Unknown { .. } => None,
        }
    }

    /// Day-grid membership. Imprecise month/year values intentionally return false.
    pub fn occurs_on(&self, day: NaiveDate, timezone: Tz) -> bool {
        match self {
            Self::DateOnly {
                start,
                end_exclusive,
            } => {
                let end = end_exclusive.unwrap_or_else(|| start.succ_opt().unwrap_or(*start));
                day >= *start && day < end
            }
            Self::AllDay {
                start,
                end_exclusive,
            } => {
                let end = end_exclusive.unwrap_or_else(|| start.succ_opt().unwrap_or(*start));
                day >= *start && day < end
            }
            Self::Instant {
                start_utc, end_utc, ..
            } => {
                let start_day = start_utc.with_timezone(&timezone).date_naive();
                let end_day = end_utc
                    .map(|end| end.with_timezone(&timezone).date_naive())
                    .unwrap_or(start_day);
                day >= start_day && day <= end_day
            }
            Self::Floating { start, end, .. } => {
                let start_day = start.date();
                let end_day = end.map(|value| value.date()).unwrap_or(start_day);
                day >= start_day && day <= end_day
            }
            Self::Month { .. } | Self::Year { .. } | Self::Unknown { .. } => false,
        }
    }

    pub fn belongs_to_month(&self, month: NaiveDate, timezone: Tz) -> bool {
        match self {
            Self::Month {
                year,
                month: event_month,
            } => *year == month.year() && *event_month == month.month(),
            Self::Year { .. } | Self::Unknown { .. } => false,
            _ => self
                .display_date(timezone)
                .is_some_and(|date| date.year() == month.year() && date.month() == month.month()),
        }
    }

    pub fn belongs_to_year(&self, year: i32, timezone: Tz) -> bool {
        match self {
            Self::Year { year: event_year } => *event_year == year,
            Self::Month {
                year: event_year, ..
            } => *event_year == year,
            Self::Unknown { .. } => false,
            _ => self
                .display_date(timezone)
                .is_some_and(|date| date.year() == year),
        }
    }

    pub const fn is_imprecise(&self) -> bool {
        matches!(
            self,
            Self::Month { .. } | Self::Year { .. } | Self::Unknown { .. }
        )
    }

    pub fn overlaps_date_window(
        &self,
        start: NaiveDate,
        end_exclusive: NaiveDate,
        timezone: Tz,
    ) -> bool {
        if end_exclusive <= start {
            return false;
        }

        match self {
            Self::DateOnly {
                start: event_start,
                end_exclusive: event_end,
            }
            | Self::AllDay {
                start: event_start,
                end_exclusive: event_end,
            } => {
                let event_end =
                    event_end.unwrap_or_else(|| event_start.succ_opt().unwrap_or(*event_start));
                *event_start < end_exclusive && event_end > start
            }
            Self::Instant {
                start_utc, end_utc, ..
            } => {
                let event_start = start_utc.with_timezone(&timezone).naive_local();
                let event_end = end_utc
                    .map(|end| end.with_timezone(&timezone).naive_local())
                    .unwrap_or(event_start);
                let window_start = start.and_hms_opt(0, 0, 0).unwrap_or(event_start);
                let window_end = end_exclusive.and_hms_opt(0, 0, 0).unwrap_or(event_start);
                if end_utc.is_some() {
                    event_start < window_end && event_end > window_start
                } else {
                    event_start >= window_start && event_start < window_end
                }
            }
            Self::Floating {
                start: event_start,
                end: event_end,
                ..
            } => {
                let event_end = event_end.unwrap_or(*event_start);
                let window_start = start.and_hms_opt(0, 0, 0).unwrap_or(*event_start);
                let window_end = end_exclusive.and_hms_opt(0, 0, 0).unwrap_or(*event_start);
                if event_end != *event_start {
                    *event_start < window_end && event_end > window_start
                } else {
                    *event_start >= window_start && *event_start < window_end
                }
            }
            Self::Month { year, month } => {
                let Some(event_start) = NaiveDate::from_ymd_opt(*year, *month, 1) else {
                    return false;
                };
                let Some(event_end) = next_month_start(event_start) else {
                    return false;
                };
                event_start < end_exclusive && event_end > start
            }
            Self::Year { year } => {
                let Some(event_start) = NaiveDate::from_ymd_opt(*year, 1, 1) else {
                    return false;
                };
                let Some(event_end) = NaiveDate::from_ymd_opt(year.saturating_add(1), 1, 1) else {
                    return false;
                };
                event_start < end_exclusive && event_end > start
            }
            Self::Unknown { .. } => false,
        }
    }

    pub fn display_time_label(&self, timezone: Tz) -> String {
        match self {
            Self::DateOnly {
                end_exclusive: Some(_),
                ..
            } => "Date range".to_string(),
            Self::DateOnly { .. } => "Date only".to_string(),
            Self::AllDay { .. } => "All day".to_string(),
            Self::Instant {
                start_utc,
                source_timezone,
                ..
            } => {
                let local = start_utc.with_timezone(&timezone);
                match source_timezone {
                    Some(source_timezone) => {
                        format!("{} · source {}", local.format("%H:%M"), source_timezone)
                    }
                    None => local.format("%H:%M").to_string(),
                }
            }
            Self::Floating {
                start,
                source_timezone,
                ..
            } => match source_timezone {
                Some(source_timezone) => {
                    format!("{} · local {}", start.format("%H:%M"), source_timezone)
                }
                None => format!("{} · floating", start.format("%H:%M")),
            },
            Self::Month { year, month } => NaiveDate::from_ymd_opt(*year, *month, 1)
                .map(|date| format!("{} · month precision", date.format("%B %Y")))
                .unwrap_or_else(|| format!("{year}-{month:02} · month precision")),
            Self::Year { year } => format!("{year} · year precision"),
            Self::Unknown { .. } => "Time unresolved".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemporalSource {
    pub id: Uuid,
    pub external_ref: Option<String>,
    pub name: String,
    pub publisher: Option<String>,
    pub authority: SourceAuthority,
    pub kind: SourceKind,
    pub locator: Option<String>,
    pub enabled: bool,
    pub read_only: bool,
    pub properties: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TemporalSource {
    pub fn new(name: impl Into<String>, kind: SourceKind, authority: SourceAuthority) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            external_ref: None,
            name: name.into(),
            publisher: None,
            authority,
            kind,
            locator: None,
            enabled: true,
            read_only: true,
            properties: Value::Object(Default::default()),
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecurrenceFrequency {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl RecurrenceFrequency {
    pub const ALL: [Self; 4] = [Self::Daily, Self::Weekly, Self::Monthly, Self::Yearly];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
            Self::Yearly => "yearly",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let normalized = raw.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|frequency| frequency.as_str() == normalized)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecurrenceRule {
    pub frequency: RecurrenceFrequency,
    #[serde(default = "default_recurrence_interval")]
    pub interval: u32,
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub until: Option<NaiveDate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rdates: Vec<TimeSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exdates: Vec<TimeSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<RecurrenceOverride>,
}

impl RecurrenceRule {
    pub const fn new(frequency: RecurrenceFrequency) -> Self {
        Self {
            frequency,
            interval: 1,
            count: None,
            until: None,
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), RecurrenceError> {
        if self.interval == 0 {
            return Err(RecurrenceError::ZeroInterval);
        }
        if self.count == Some(0) {
            return Err(RecurrenceError::ZeroCount);
        }
        Ok(())
    }
}

const fn default_recurrence_interval() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecurrenceOverride {
    pub original: TimeSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement: Option<TimeSpec>,
    #[serde(default)]
    pub cancelled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecurrenceOccurrenceOrigin {
    Single,
    Rule,
    RDate,
    DetachedOverride,
}

impl RecurrenceOccurrenceOrigin {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Rule => "rule",
            Self::RDate => "rdate",
            Self::DetachedOverride => "detached override",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventOccurrence {
    pub id: Uuid,
    pub event_id: Uuid,
    pub recurrence_index: Option<u32>,
    pub origin: RecurrenceOccurrenceOrigin,
    pub original_time: TimeSpec,
    pub time: TimeSpec,
    pub status: EventStatus,
    pub override_applied: bool,
    pub cancelled_by_override: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecurrenceError {
    ZeroInterval,
    ZeroCount,
    UnsupportedTimeKind(&'static str),
    InvalidSourceTimezone(String),
    MismatchedExceptionTimeKind {
        expected: &'static str,
        actual: &'static str,
    },
    DuplicateOverride(String),
    ConflictingException(String),
    ArithmeticOverflow,
}

impl fmt::Display for RecurrenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroInterval => formatter.write_str("recurrence interval must be at least 1"),
            Self::ZeroCount => formatter.write_str("recurrence count must be at least 1"),
            Self::UnsupportedTimeKind(kind) => {
                write!(
                    formatter,
                    "recurrence is not supported for {kind} precision"
                )
            }
            Self::InvalidSourceTimezone(zone) => {
                write!(formatter, "invalid recurrence source timezone {zone}")
            }
            Self::MismatchedExceptionTimeKind { expected, actual } => {
                write!(
                    formatter,
                    "recurrence exception uses {actual} time for a {expected} series"
                )
            }
            Self::DuplicateOverride(key) => {
                write!(formatter, "duplicate recurrence override for {key}")
            }
            Self::ConflictingException(key) => {
                write!(
                    formatter,
                    "recurrence occurrence {key} cannot be both excluded and overridden"
                )
            }
            Self::ArithmeticOverflow => formatter.write_str("recurrence arithmetic overflow"),
        }
    }
}

impl std::error::Error for RecurrenceError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemporalEvent {
    pub id: Uuid,
    pub source_id: Option<Uuid>,
    pub source_record_key: Option<String>,

    pub upstream_event_ref: Option<String>,
    pub upstream_reconciled_key: Option<String>,
    pub assertion_refs: Vec<String>,
    pub source_refs: Vec<String>,
    pub provenance_refs: Vec<String>,
    pub renderability: Option<String>,

    pub normalized_title: String,
    pub raw_title: Option<String>,
    pub description: Option<String>,
    pub event_type: Option<String>,
    pub domain: Option<String>,
    pub jurisdiction: Option<String>,
    pub institution: Option<String>,
    pub status: EventStatus,
    pub confidence: Option<f32>,
    pub importance: Option<i32>,
    pub personal_relevance: Option<i32>,
    pub time: TimeSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurrence: Option<RecurrenceRule>,
    pub tags: Vec<String>,
    pub properties: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TemporalEvent {
    pub fn new(normalized_title: impl Into<String>, time: TimeSpec) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            source_id: None,
            source_record_key: None,
            upstream_event_ref: None,
            upstream_reconciled_key: None,
            assertion_refs: Vec::new(),
            source_refs: Vec::new(),
            provenance_refs: Vec::new(),
            renderability: None,
            normalized_title: normalized_title.into(),
            raw_title: None,
            description: None,
            event_type: None,
            domain: None,
            jurisdiction: None,
            institution: None,
            status: EventStatus::Scheduled,
            confidence: None,
            importance: None,
            personal_relevance: None,
            time,
            recurrence: None,
            tags: Vec::new(),
            properties: Value::Object(Default::default()),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn display_date(&self, timezone: Tz) -> Option<NaiveDate> {
        self.time.display_date(timezone)
    }

    pub fn display_time_label(&self, timezone: Tz) -> String {
        self.time.display_time_label(timezone)
    }

    pub fn validate_recurrence(&self) -> Result<(), RecurrenceError> {
        let Some(rule) = self.recurrence.as_ref() else {
            return Ok(());
        };

        rule.validate()?;
        validate_recurrence_time(&self.time)?;
        let expected_kind = self.time.kind_name();

        for time in rule.rdates.iter().chain(rule.exdates.iter()) {
            validate_recurrence_time(time)?;
            if time.kind_name() != expected_kind {
                return Err(RecurrenceError::MismatchedExceptionTimeKind {
                    expected: expected_kind,
                    actual: time.kind_name(),
                });
            }
        }

        let exdate_keys = rule
            .exdates
            .iter()
            .map(recurrence_key)
            .collect::<Result<HashSet<_>, _>>()?;
        let mut override_keys = HashSet::new();
        for occurrence_override in &rule.overrides {
            validate_recurrence_time(&occurrence_override.original)?;
            if occurrence_override.original.kind_name() != expected_kind {
                return Err(RecurrenceError::MismatchedExceptionTimeKind {
                    expected: expected_kind,
                    actual: occurrence_override.original.kind_name(),
                });
            }
            if let Some(replacement) = occurrence_override.replacement.as_ref() {
                validate_recurrence_time(replacement)?;
                if replacement.kind_name() != expected_kind {
                    return Err(RecurrenceError::MismatchedExceptionTimeKind {
                        expected: expected_kind,
                        actual: replacement.kind_name(),
                    });
                }
            }

            let key = recurrence_key(&occurrence_override.original)?;
            if !override_keys.insert(key.clone()) {
                return Err(RecurrenceError::DuplicateOverride(key));
            }
            if exdate_keys.contains(&key) {
                return Err(RecurrenceError::ConflictingException(key));
            }
        }

        Ok(())
    }

    pub fn occurrences_in_window(
        &self,
        start: NaiveDate,
        end_exclusive: NaiveDate,
        display_timezone: Tz,
    ) -> Result<Vec<EventOccurrence>, RecurrenceError> {
        if end_exclusive <= start {
            return Ok(Vec::new());
        }

        let Some(rule) = self.recurrence.as_ref() else {
            if !self
                .time
                .overlaps_date_window(start, end_exclusive, display_timezone)
            {
                return Ok(Vec::new());
            }
            return Ok(vec![EventOccurrence {
                id: self.id,
                event_id: self.id,
                recurrence_index: None,
                origin: RecurrenceOccurrenceOrigin::Single,
                original_time: self.time.clone(),
                time: self.time.clone(),
                status: self.status,
                override_applied: false,
                cancelled_by_override: false,
            }]);
        };

        self.validate_recurrence()?;

        let mut occurrences = Vec::new();
        let mut seen_occurrence_ids = HashSet::new();
        let mut generated_keys = HashSet::new();
        let mut recurrence_period = 0_u32;
        let mut emitted = 0_u32;

        loop {
            if rule.count.is_some_and(|count| emitted >= count) {
                break;
            }

            let shifted = shift_recurrence_time(&self.time, rule, recurrence_period)?;
            recurrence_period = recurrence_period
                .checked_add(1)
                .ok_or(RecurrenceError::ArithmeticOverflow)?;

            let Some(time) = shifted else {
                continue;
            };
            let occurrence_date = recurrence_rule_date(&time)?;

            if rule.until.is_some_and(|until| occurrence_date > until) {
                break;
            }

            let recurrence_index = emitted;
            emitted = emitted
                .checked_add(1)
                .ok_or(RecurrenceError::ArithmeticOverflow)?;
            generated_keys.insert(recurrence_key(&time)?);

            if let Some(occurrence) = materialize_recurrence_occurrence(
                self.id,
                self.status,
                rule,
                time,
                Some(recurrence_index),
                RecurrenceOccurrenceOrigin::Rule,
                start,
                end_exclusive,
                display_timezone,
            )? && seen_occurrence_ids.insert(occurrence.id)
            {
                occurrences.push(occurrence);
            }

            if occurrence_date >= end_exclusive {
                break;
            }
        }

        for rdate in &rule.rdates {
            generated_keys.insert(recurrence_key(rdate)?);
            if let Some(occurrence) = materialize_recurrence_occurrence(
                self.id,
                self.status,
                rule,
                rdate.clone(),
                None,
                RecurrenceOccurrenceOrigin::RDate,
                start,
                end_exclusive,
                display_timezone,
            )? && seen_occurrence_ids.insert(occurrence.id)
            {
                occurrences.push(occurrence);
            }
        }

        for occurrence_override in &rule.overrides {
            let key = recurrence_key(&occurrence_override.original)?;
            if generated_keys.contains(&key) {
                continue;
            }
            if let Some(occurrence) = materialize_recurrence_occurrence(
                self.id,
                self.status,
                rule,
                occurrence_override.original.clone(),
                None,
                RecurrenceOccurrenceOrigin::DetachedOverride,
                start,
                end_exclusive,
                display_timezone,
            )? && seen_occurrence_ids.insert(occurrence.id)
            {
                occurrences.push(occurrence);
            }
        }

        Ok(occurrences)
    }
}

fn occurrence_identity(event_id: Uuid, original_time: &TimeSpec) -> Result<Uuid, RecurrenceError> {
    let key = recurrence_key(original_time)?;
    let mut hasher = Sha256::new();
    hasher.update(event_id.as_bytes());
    hasher.update(key.as_bytes());
    let digest = hasher.finalize();

    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(Uuid::from_bytes(bytes))
}

fn recurrence_key(time: &TimeSpec) -> Result<String, RecurrenceError> {
    match time {
        TimeSpec::DateOnly { start, .. } => Ok(format!("date_only:{start}")),
        TimeSpec::AllDay { start, .. } => Ok(format!("all_day:{start}")),
        TimeSpec::Instant { start_utc, .. } => Ok(format!("instant:{}", start_utc.to_rfc3339())),
        TimeSpec::Floating { start, .. } => Ok(format!("floating:{start}")),
        TimeSpec::Month { .. } | TimeSpec::Year { .. } | TimeSpec::Unknown { .. } => {
            Err(RecurrenceError::UnsupportedTimeKind(time.kind_name()))
        }
    }
}

fn validate_recurrence_time(time: &TimeSpec) -> Result<(), RecurrenceError> {
    if matches!(
        time,
        TimeSpec::Month { .. } | TimeSpec::Year { .. } | TimeSpec::Unknown { .. }
    ) {
        return Err(RecurrenceError::UnsupportedTimeKind(time.kind_name()));
    }

    if let TimeSpec::Instant {
        source_timezone: Some(raw),
        ..
    } = time
    {
        raw.parse::<Tz>()
            .map_err(|_| RecurrenceError::InvalidSourceTimezone(raw.clone()))?;
    }

    Ok(())
}

fn materialize_recurrence_occurrence(
    event_id: Uuid,
    base_status: EventStatus,
    rule: &RecurrenceRule,
    original_time: TimeSpec,
    recurrence_index: Option<u32>,
    origin: RecurrenceOccurrenceOrigin,
    start: NaiveDate,
    end_exclusive: NaiveDate,
    display_timezone: Tz,
) -> Result<Option<EventOccurrence>, RecurrenceError> {
    let key = recurrence_key(&original_time)?;

    for exdate in &rule.exdates {
        if recurrence_key(exdate)? == key {
            return Ok(None);
        }
    }

    let mut time = original_time.clone();
    let mut status = base_status;
    let mut override_applied = false;
    let mut cancelled_by_override = false;

    for occurrence_override in &rule.overrides {
        if recurrence_key(&occurrence_override.original)? != key {
            continue;
        }

        override_applied = true;
        if let Some(replacement) = occurrence_override.replacement.as_ref() {
            time.clone_from(replacement);
        }
        if occurrence_override.cancelled {
            status = EventStatus::Cancelled;
            cancelled_by_override = true;
        }
        break;
    }

    if !time.overlaps_date_window(start, end_exclusive, display_timezone) {
        return Ok(None);
    }

    Ok(Some(EventOccurrence {
        id: occurrence_identity(event_id, &original_time)?,
        event_id,
        recurrence_index,
        origin,
        original_time,
        time,
        status,
        override_applied,
        cancelled_by_override,
    }))
}

fn shift_recurrence_time(
    time: &TimeSpec,
    rule: &RecurrenceRule,
    period: u32,
) -> Result<Option<TimeSpec>, RecurrenceError> {
    let steps = rule
        .interval
        .checked_mul(period)
        .ok_or(RecurrenceError::ArithmeticOverflow)?;

    match time {
        TimeSpec::DateOnly {
            start,
            end_exclusive,
        } => {
            let Some(shifted_start) = shift_date(*start, rule.frequency, steps)? else {
                return Ok(None);
            };
            let shifted_end = match end_exclusive {
                Some(end) => Some(
                    shifted_start
                        .checked_add_signed(Duration::days((*end - *start).num_days()))
                        .ok_or(RecurrenceError::ArithmeticOverflow)?,
                ),
                None => None,
            };
            Ok(Some(TimeSpec::DateOnly {
                start: shifted_start,
                end_exclusive: shifted_end,
            }))
        }
        TimeSpec::AllDay {
            start,
            end_exclusive,
        } => {
            let Some(shifted_start) = shift_date(*start, rule.frequency, steps)? else {
                return Ok(None);
            };
            let shifted_end = match end_exclusive {
                Some(end) => Some(
                    shifted_start
                        .checked_add_signed(Duration::days((*end - *start).num_days()))
                        .ok_or(RecurrenceError::ArithmeticOverflow)?,
                ),
                None => None,
            };
            Ok(Some(TimeSpec::AllDay {
                start: shifted_start,
                end_exclusive: shifted_end,
            }))
        }
        TimeSpec::Floating {
            start,
            end,
            source_timezone,
        } => {
            let Some(shifted_start) = shift_naive_datetime(*start, rule.frequency, steps)? else {
                return Ok(None);
            };
            let shifted_end = match end {
                Some(end) => Some(
                    shifted_start
                        .checked_add_signed(*end - *start)
                        .ok_or(RecurrenceError::ArithmeticOverflow)?,
                ),
                None => None,
            };
            Ok(Some(TimeSpec::Floating {
                start: shifted_start,
                end: shifted_end,
                source_timezone: source_timezone.clone(),
            }))
        }
        TimeSpec::Instant {
            start_utc,
            end_utc,
            source_timezone,
        } => {
            let timezone = match source_timezone.as_deref() {
                Some(raw) => raw
                    .parse::<Tz>()
                    .map_err(|_| RecurrenceError::InvalidSourceTimezone(raw.to_string()))?,
                None => chrono_tz::UTC,
            };
            let local_start = start_utc.with_timezone(&timezone).naive_local();
            let Some(shifted_local) = shift_naive_datetime(local_start, rule.frequency, steps)?
            else {
                return Ok(None);
            };
            let Some(shifted_start) = resolve_local_datetime(timezone, shifted_local) else {
                return Ok(None);
            };
            let shifted_end = match end_utc {
                Some(end) => Some(
                    shifted_start
                        .checked_add_signed(*end - *start_utc)
                        .ok_or(RecurrenceError::ArithmeticOverflow)?,
                ),
                None => None,
            };
            Ok(Some(TimeSpec::Instant {
                start_utc: shifted_start,
                end_utc: shifted_end,
                source_timezone: source_timezone.clone(),
            }))
        }
        TimeSpec::Month { .. } | TimeSpec::Year { .. } | TimeSpec::Unknown { .. } => {
            Err(RecurrenceError::UnsupportedTimeKind(time.kind_name()))
        }
    }
}

fn recurrence_rule_date(time: &TimeSpec) -> Result<NaiveDate, RecurrenceError> {
    match time {
        TimeSpec::DateOnly { start, .. } | TimeSpec::AllDay { start, .. } => Ok(*start),
        TimeSpec::Floating { start, .. } => Ok(start.date()),
        TimeSpec::Instant {
            start_utc,
            source_timezone,
            ..
        } => {
            let timezone = match source_timezone.as_deref() {
                Some(raw) => raw
                    .parse::<Tz>()
                    .map_err(|_| RecurrenceError::InvalidSourceTimezone(raw.to_string()))?,
                None => chrono_tz::UTC,
            };
            Ok(start_utc.with_timezone(&timezone).date_naive())
        }
        TimeSpec::Month { .. } | TimeSpec::Year { .. } | TimeSpec::Unknown { .. } => {
            Err(RecurrenceError::UnsupportedTimeKind(time.kind_name()))
        }
    }
}

fn shift_naive_datetime(
    value: NaiveDateTime,
    frequency: RecurrenceFrequency,
    steps: u32,
) -> Result<Option<NaiveDateTime>, RecurrenceError> {
    let Some(date) = shift_date(value.date(), frequency, steps)? else {
        return Ok(None);
    };
    Ok(Some(NaiveDateTime::new(date, value.time())))
}

fn shift_date(
    value: NaiveDate,
    frequency: RecurrenceFrequency,
    steps: u32,
) -> Result<Option<NaiveDate>, RecurrenceError> {
    match frequency {
        RecurrenceFrequency::Daily => Ok(value.checked_add_days(Days::new(u64::from(steps)))),
        RecurrenceFrequency::Weekly => {
            let days = u64::from(steps)
                .checked_mul(7)
                .ok_or(RecurrenceError::ArithmeticOverflow)?;
            Ok(value.checked_add_days(Days::new(days)))
        }
        RecurrenceFrequency::Monthly => add_months_preserving_day(value, steps),
        RecurrenceFrequency::Yearly => {
            let step_years =
                i32::try_from(steps).map_err(|_| RecurrenceError::ArithmeticOverflow)?;
            let year = value
                .year()
                .checked_add(step_years)
                .ok_or(RecurrenceError::ArithmeticOverflow)?;
            Ok(NaiveDate::from_ymd_opt(year, value.month(), value.day()))
        }
    }
}

fn add_months_preserving_day(
    value: NaiveDate,
    months: u32,
) -> Result<Option<NaiveDate>, RecurrenceError> {
    let month_zero = i64::from(value.month0());
    let base = i64::from(value.year())
        .checked_mul(12)
        .and_then(|year_months| year_months.checked_add(month_zero))
        .ok_or(RecurrenceError::ArithmeticOverflow)?;
    let target = base
        .checked_add(i64::from(months))
        .ok_or(RecurrenceError::ArithmeticOverflow)?;
    let year =
        i32::try_from(target.div_euclid(12)).map_err(|_| RecurrenceError::ArithmeticOverflow)?;
    let month = u32::try_from(target.rem_euclid(12) + 1)
        .map_err(|_| RecurrenceError::ArithmeticOverflow)?;
    Ok(NaiveDate::from_ymd_opt(year, month, value.day()))
}

fn next_month_start(value: NaiveDate) -> Option<NaiveDate> {
    if value.month() == 12 {
        NaiveDate::from_ymd_opt(value.year().checked_add(1)?, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(value.year(), value.month() + 1, 1)
    }
}

fn resolve_local_datetime(timezone: Tz, value: NaiveDateTime) -> Option<DateTime<Utc>> {
    match timezone.from_local_datetime(&value) {
        LocalResult::Single(value) => Some(value.with_timezone(&Utc)),
        LocalResult::Ambiguous(first, second) => Some(first.min(second).with_timezone(&Utc)),
        LocalResult::None => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn daily_recurrence_expands_with_stable_occurrence_identity() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Daily",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Daily,
            interval: 2,
            count: Some(4),
            until: None,
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let first = event
            .occurrences_in_window(start, start + Duration::days(10), chrono_tz::UTC)
            .expect("expand");
        let second = event
            .occurrences_in_window(start, start + Duration::days(10), chrono_tz::UTC)
            .expect("expand again");

        assert_eq!(first.len(), 4);
        assert_eq!(
            first
                .iter()
                .map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                Some(start),
                Some(start + Duration::days(2)),
                Some(start + Duration::days(4)),
                Some(start + Duration::days(6)),
            ]
        );
        assert_eq!(
            first
                .iter()
                .map(|occurrence| occurrence.id)
                .collect::<Vec<_>>(),
            second
                .iter()
                .map(|occurrence| occurrence.id)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn rdate_and_exdate_adjust_generated_occurrence_set() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Adjusted daily",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Daily,
            interval: 1,
            count: Some(3),
            until: None,
            rdates: vec![TimeSpec::DateOnly {
                start: start + Duration::days(4),
                end_exclusive: None,
            }],
            exdates: vec![TimeSpec::DateOnly {
                start: start + Duration::days(1),
                end_exclusive: None,
            }],
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(start, start + Duration::days(7), chrono_tz::UTC)
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![start, start + Duration::days(2), start + Duration::days(4),]
        );
        assert!(
            occurrences
                .iter()
                .any(|occurrence| occurrence.origin == RecurrenceOccurrenceOrigin::RDate)
        );
    }

    #[test]
    fn moved_override_keeps_original_occurrence_identity() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 1).expect("start");
        let original = TimeSpec::DateOnly {
            start: start + Duration::days(7),
            end_exclusive: None,
        };
        let replacement = TimeSpec::DateOnly {
            start: start + Duration::days(9),
            end_exclusive: None,
        };
        let mut event = TemporalEvent::new(
            "Weekly move",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Weekly,
            interval: 1,
            count: Some(2),
            until: None,
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let baseline = event
            .occurrences_in_window(start, start + Duration::days(14), chrono_tz::UTC)
            .expect("baseline");
        let baseline_id = baseline[1].id;

        event
            .recurrence
            .as_mut()
            .expect("recurrence")
            .overrides
            .push(RecurrenceOverride {
                original: original.clone(),
                replacement: Some(replacement.clone()),
                cancelled: false,
            });

        let moved = event
            .occurrences_in_window(start, start + Duration::days(14), chrono_tz::UTC)
            .expect("moved");
        let occurrence = moved
            .iter()
            .find(|occurrence| occurrence.original_time == original)
            .expect("moved occurrence");

        assert_eq!(occurrence.id, baseline_id);
        assert_eq!(occurrence.time, replacement);
        assert!(occurrence.override_applied);
        assert!(!occurrence.cancelled_by_override);
    }

    #[test]
    fn cancelled_override_remains_materialized_as_cancelled() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 1).expect("start");
        let cancelled_time = TimeSpec::DateOnly {
            start: start + Duration::days(1),
            end_exclusive: None,
        };
        let mut event = TemporalEvent::new(
            "Daily cancellation",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Daily,
            interval: 1,
            count: Some(2),
            until: None,
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: vec![RecurrenceOverride {
                original: cancelled_time.clone(),
                replacement: None,
                cancelled: true,
            }],
        });

        let occurrences = event
            .occurrences_in_window(start, start + Duration::days(3), chrono_tz::UTC)
            .expect("expand");
        let cancelled = occurrences
            .iter()
            .find(|occurrence| occurrence.original_time == cancelled_time)
            .expect("cancelled occurrence");

        assert_eq!(cancelled.status, EventStatus::Cancelled);
        assert_eq!(cancelled.time, cancelled_time);
        assert!(cancelled.cancelled_by_override);
    }

    #[test]
    fn detached_moved_override_can_enter_the_active_window() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 1).expect("start");
        let original = TimeSpec::DateOnly {
            start: start + Duration::days(19),
            end_exclusive: None,
        };
        let replacement = TimeSpec::DateOnly {
            start: start + Duration::days(4),
            end_exclusive: None,
        };
        let mut event = TemporalEvent::new(
            "Moved into window",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Daily,
            interval: 1,
            count: None,
            until: None,
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: vec![RecurrenceOverride {
                original: original.clone(),
                replacement: Some(replacement),
                cancelled: false,
            }],
        });

        let occurrences = event
            .occurrences_in_window(
                start + Duration::days(4),
                start + Duration::days(5),
                chrono_tz::UTC,
            )
            .expect("expand");
        let moved = occurrences
            .iter()
            .find(|occurrence| occurrence.original_time == original)
            .expect("detached moved occurrence");

        assert_eq!(moved.origin, RecurrenceOccurrenceOrigin::DetachedOverride);
        assert!(moved.override_applied);
    }

    #[test]
    fn exdate_and_override_conflict_is_rejected() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 1).expect("start");
        let excluded = TimeSpec::DateOnly {
            start: start + Duration::days(1),
            end_exclusive: None,
        };
        let mut event = TemporalEvent::new(
            "Conflicting exception",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Daily,
            interval: 1,
            count: None,
            until: None,
            rdates: Vec::new(),
            exdates: vec![excluded.clone()],
            overrides: vec![RecurrenceOverride {
                original: excluded,
                replacement: None,
                cancelled: true,
            }],
        });

        assert!(matches!(
            event.validate_recurrence(),
            Err(RecurrenceError::ConflictingException(_))
        ));
    }

    #[test]
    fn monthly_recurrence_skips_invalid_calendar_dates() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 31).expect("start");
        let mut event = TemporalEvent::new(
            "Month end",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: Some(3),
            until: None,
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2026, 6, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");
        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 31).expect("jan"),
                NaiveDate::from_ymd_opt(2026, 3, 31).expect("mar"),
                NaiveDate::from_ymd_opt(2026, 5, 31).expect("may"),
            ]
        );
    }

    #[test]
    fn recurrence_preserves_all_day_range_duration() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Two days",
            TimeSpec::AllDay {
                start,
                end_exclusive: Some(start + Duration::days(2)),
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Weekly,
            interval: 1,
            count: Some(2),
            until: None,
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(start, start + Duration::days(14), chrono_tz::UTC)
            .expect("expand");

        assert_eq!(occurrences.len(), 2);
        assert_eq!(
            occurrences[1].time,
            TimeSpec::AllDay {
                start: start + Duration::days(7),
                end_exclusive: Some(start + Duration::days(9)),
            }
        );
    }

    #[test]
    fn exact_recurrence_preserves_source_wall_clock_across_dst() {
        let start_utc = DateTime::parse_from_rfc3339("2026-03-01T14:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        let mut event = TemporalEvent::new(
            "Weekly at nine",
            TimeSpec::Instant {
                start_utc,
                end_utc: None,
                source_timezone: Some("America/New_York".to_string()),
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Weekly,
            interval: 1,
            count: Some(2),
            until: None,
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                NaiveDate::from_ymd_opt(2026, 3, 1).expect("start"),
                NaiveDate::from_ymd_opt(2026, 3, 10).expect("end"),
                chrono_tz::America::New_York,
            )
            .expect("expand");

        assert_eq!(occurrences.len(), 2);
        let TimeSpec::Instant {
            start_utc: second, ..
        } = occurrences[1].time
        else {
            panic!("instant occurrence");
        };
        assert_eq!(second.format("%H:%M").to_string(), "13:00");
        assert_eq!(
            second
                .with_timezone(&chrono_tz::America::New_York)
                .format("%H:%M")
                .to_string(),
            "09:00"
        );
    }

    #[test]
    fn recurrence_validation_rejects_coarse_precision_before_persistence() {
        for time in [
            TimeSpec::Month {
                year: 2026,
                month: 10,
            },
            TimeSpec::Year { year: 2026 },
            TimeSpec::Unknown {
                original_value: Some("someday".to_string()),
            },
        ] {
            let mut event = TemporalEvent::new("Unsupported recurrence", time);
            event.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Monthly));

            assert!(matches!(
                event.validate_recurrence(),
                Err(RecurrenceError::UnsupportedTimeKind(_))
            ));
        }
    }

    #[test]
    fn recurrence_rejects_coarse_precision_base_time() {
        let mut event = TemporalEvent::new("Annual unknown month", TimeSpec::Year { year: 2026 });
        event.recurrence = Some(RecurrenceRule::new(RecurrenceFrequency::Yearly));

        let error = event
            .occurrences_in_window(
                NaiveDate::from_ymd_opt(2026, 1, 1).expect("start"),
                NaiveDate::from_ymd_opt(2027, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect_err("coarse recurrence must be rejected");
        assert!(matches!(
            error,
            RecurrenceError::UnsupportedTimeKind("year")
        ));
    }

    #[test]
    fn all_day_event_does_not_shift_with_timezone() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 4).expect("valid date");
        let time = TimeSpec::AllDay {
            start,
            end_exclusive: None,
        };

        assert_eq!(time.display_date(chrono_tz::UTC), Some(start));
        assert_eq!(time.display_date(chrono_tz::Pacific::Honolulu), Some(start));
        assert_eq!(time.display_date(chrono_tz::Asia::Tokyo), Some(start));
    }

    #[test]
    fn date_only_is_not_all_day() {
        let date = NaiveDate::from_ymd_opt(2027, 10, 24).expect("valid date");
        let time = TimeSpec::DateOnly {
            start: date,
            end_exclusive: None,
        };

        assert_eq!(time.kind_name(), "date_only");
        assert_eq!(time.display_time_label(chrono_tz::UTC), "Date only");
        assert!(time.occurs_on(date, chrono_tz::UTC));
    }

    #[test]
    fn instant_event_uses_display_timezone() {
        let start_utc = DateTime::parse_from_rfc3339("2026-10-05T01:00:00Z")
            .expect("valid timestamp")
            .with_timezone(&Utc);
        let time = TimeSpec::Instant {
            start_utc,
            end_utc: None,
            source_timezone: Some("UTC".to_string()),
        };

        let mexico = time.display_date(chrono_tz::America::Mexico_City);
        let tokyo = time.display_date(chrono_tz::Asia::Tokyo);

        assert_ne!(mexico, tokyo);
    }

    #[test]
    fn month_and_year_precision_do_not_invent_day() {
        let month = TimeSpec::Month {
            year: 2027,
            month: 11,
        };
        let year = TimeSpec::Year { year: 2027 };

        assert_eq!(month.display_date(chrono_tz::UTC), None);
        assert_eq!(year.display_date(chrono_tz::UTC), None);
        assert!(month.belongs_to_month(
            NaiveDate::from_ymd_opt(2027, 11, 1).expect("month"),
            chrono_tz::UTC
        ));
        assert!(year.belongs_to_year(2027, chrono_tz::UTC));
    }
}
