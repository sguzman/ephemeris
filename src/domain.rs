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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecurrenceWeekday {
    #[default]
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl RecurrenceWeekday {
    pub const ALL: [Self; 7] = [
        Self::Monday,
        Self::Tuesday,
        Self::Wednesday,
        Self::Thursday,
        Self::Friday,
        Self::Saturday,
        Self::Sunday,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Monday => "monday",
            Self::Tuesday => "tuesday",
            Self::Wednesday => "wednesday",
            Self::Thursday => "thursday",
            Self::Friday => "friday",
            Self::Saturday => "saturday",
            Self::Sunday => "sunday",
        }
    }

    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Monday => "Mon",
            Self::Tuesday => "Tue",
            Self::Wednesday => "Wed",
            Self::Thursday => "Thu",
            Self::Friday => "Fri",
            Self::Saturday => "Sat",
            Self::Sunday => "Sun",
        }
    }

    pub const fn offset_from_monday(self) -> u64 {
        match self {
            Self::Monday => 0,
            Self::Tuesday => 1,
            Self::Wednesday => 2,
            Self::Thursday => 3,
            Self::Friday => 4,
            Self::Saturday => 5,
            Self::Sunday => 6,
        }
    }

    pub const fn offset_from(self, week_start: Self) -> u64 {
        (self.offset_from_monday() + 7 - week_start.offset_from_monday()) % 7
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let normalized = raw.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|weekday| weekday.as_str() == normalized)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RecurrenceOrdinalWeekday {
    pub ordinal: i8,
    pub weekday: RecurrenceWeekday,
}

impl RecurrenceOrdinalWeekday {
    pub const fn new(ordinal: i8, weekday: RecurrenceWeekday) -> Self {
        Self { ordinal, weekday }
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
    #[serde(default)]
    pub week_start: RecurrenceWeekday,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by_weekday: Vec<RecurrenceWeekday>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by_month: Vec<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by_week_no: Vec<i8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by_year_day: Vec<i16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by_month_day: Vec<i8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by_month_weekday: Vec<RecurrenceOrdinalWeekday>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by_set_pos: Vec<i16>,
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
            week_start: RecurrenceWeekday::Monday,
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
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
        let yearly_week_context =
            self.frequency == RecurrenceFrequency::Yearly && !self.by_week_no.is_empty();
        if !self.by_weekday.is_empty()
            && self.frequency != RecurrenceFrequency::Weekly
            && self.frequency != RecurrenceFrequency::Monthly
            && self.frequency != RecurrenceFrequency::Yearly
        {
            return Err(RecurrenceError::ByWeekdayRequiresSupportedContext);
        }
        let weekly_week_context =
            self.frequency == RecurrenceFrequency::Weekly && !self.by_weekday.is_empty();
        if self.week_start != RecurrenceWeekday::Monday
            && !weekly_week_context
            && !yearly_week_context
        {
            return Err(RecurrenceError::WeekStartRequiresWeekContext);
        }
        let mut weekdays = HashSet::new();
        for weekday in &self.by_weekday {
            if !weekdays.insert(*weekday) {
                return Err(RecurrenceError::DuplicateByWeekday(weekday.as_str()));
            }
        }
        if !self.by_month.is_empty() && self.frequency != RecurrenceFrequency::Yearly {
            return Err(RecurrenceError::ByMonthRequiresYearly);
        }
        let mut months = HashSet::new();
        for month in &self.by_month {
            if !(1..=12).contains(month) {
                return Err(RecurrenceError::InvalidByMonth(*month));
            }
            if !months.insert(*month) {
                return Err(RecurrenceError::DuplicateByMonth(*month));
            }
        }
        if !self.by_week_no.is_empty() && self.frequency != RecurrenceFrequency::Yearly {
            return Err(RecurrenceError::ByWeekNoRequiresYearly);
        }
        let mut week_numbers = HashSet::new();
        for week_no in &self.by_week_no {
            if *week_no == 0 || !(-53..=53).contains(week_no) {
                return Err(RecurrenceError::InvalidByWeekNo(*week_no));
            }
            if !week_numbers.insert(*week_no) {
                return Err(RecurrenceError::DuplicateByWeekNo(*week_no));
            }
        }
        if !self.by_year_day.is_empty() && self.frequency != RecurrenceFrequency::Yearly {
            return Err(RecurrenceError::ByYearDayRequiresYearly);
        }
        let mut year_days = HashSet::new();
        for day in &self.by_year_day {
            if *day == 0 || !(-366..=366).contains(day) {
                return Err(RecurrenceError::InvalidByYearDay(*day));
            }
            if !year_days.insert(*day) {
                return Err(RecurrenceError::DuplicateByYearDay(*day));
            }
        }
        if !self.by_month_day.is_empty()
            && self.frequency != RecurrenceFrequency::Monthly
            && self.frequency != RecurrenceFrequency::Yearly
        {
            return Err(RecurrenceError::ByMonthDayRequiresMonthlyOrYearly);
        }
        let mut month_days = HashSet::new();
        for day in &self.by_month_day {
            if *day == 0 || !(-31..=31).contains(day) {
                return Err(RecurrenceError::InvalidByMonthDay(*day));
            }
            if !month_days.insert(*day) {
                return Err(RecurrenceError::DuplicateByMonthDay(*day));
            }
        }
        if !self.by_week_no.is_empty() && !self.by_month_weekday.is_empty() {
            return Err(RecurrenceError::OrdinalByWeekdayWithByWeekNo);
        }
        if !self.by_month_weekday.is_empty()
            && self.frequency != RecurrenceFrequency::Monthly
            && self.frequency != RecurrenceFrequency::Yearly
        {
            return Err(RecurrenceError::OrdinalByWeekdayRequiresMonthlyOrYearly);
        }
        let ordinal_limit = if self.frequency == RecurrenceFrequency::Yearly
            && self.by_month.is_empty()
        {
            53
        } else {
            5
        };
        let mut ordinal_weekdays = HashSet::new();
        for selector in &self.by_month_weekday {
            if selector.ordinal == 0
                || !(-ordinal_limit..=ordinal_limit).contains(&selector.ordinal)
            {
                return Err(RecurrenceError::InvalidOrdinalByWeekday(selector.ordinal));
            }
            if !ordinal_weekdays.insert(*selector) {
                return Err(RecurrenceError::DuplicateOrdinalByWeekday(
                    selector.ordinal,
                    selector.weekday.as_str(),
                ));
            }
        }
        if !self.by_set_pos.is_empty() {
            let has_selector = !self.by_weekday.is_empty()
                || !self.by_month.is_empty()
                || !self.by_week_no.is_empty()
                || !self.by_year_day.is_empty()
                || !self.by_month_day.is_empty()
                || !self.by_month_weekday.is_empty();
            if !has_selector {
                return Err(RecurrenceError::BySetPosRequiresSelector);
            }

            let mut positions = HashSet::new();
            for position in &self.by_set_pos {
                if *position == 0 || !(-366..=366).contains(position) {
                    return Err(RecurrenceError::InvalidBySetPos(*position));
                }
                if !positions.insert(*position) {
                    return Err(RecurrenceError::DuplicateBySetPos(*position));
                }
            }
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
    ByWeekdayRequiresSupportedContext,
    WeekStartRequiresWeekContext,
    DuplicateByWeekday(&'static str),
    ByMonthRequiresYearly,
    InvalidByMonth(u8),
    DuplicateByMonth(u8),
    ByWeekNoRequiresYearly,
    InvalidByWeekNo(i8),
    DuplicateByWeekNo(i8),
    ByYearDayRequiresYearly,
    InvalidByYearDay(i16),
    DuplicateByYearDay(i16),
    ByMonthDayRequiresMonthlyOrYearly,
    InvalidByMonthDay(i8),
    DuplicateByMonthDay(i8),
    OrdinalByWeekdayRequiresMonthlyOrYearly,
    InvalidOrdinalByWeekday(i8),
    DuplicateOrdinalByWeekday(i8, &'static str),
    OrdinalByWeekdayWithByWeekNo,
    BySetPosRequiresSelector,
    InvalidBySetPos(i16),
    DuplicateBySetPos(i16),
    UnsupportedTimeKind(&'static str),
    InvalidSourceTimezone(String),
    MismatchedExceptionTimeKind {
        expected: &'static str,
        actual: &'static str,
    },
    DuplicateOverride(String),
    ConflictingException(String),
    UnknownOverrideTarget(String),
    ArithmeticOverflow,
}

impl fmt::Display for RecurrenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroInterval => formatter.write_str("recurrence interval must be at least 1"),
            Self::ZeroCount => formatter.write_str("recurrence count must be at least 1"),
            Self::ByWeekdayRequiresSupportedContext => formatter.write_str(
                "plain BYDAY weekday selection requires weekly, monthly, or yearly recurrence",
            ),
            Self::WeekStartRequiresWeekContext => formatter.write_str(
                "custom WKST requires weekly BYDAY recurrence or yearly BYWEEKNO recurrence",
            ),
            Self::DuplicateByWeekday(weekday) => {
                write!(formatter, "duplicate BYDAY weekday {weekday}")
            }
            Self::ByMonthRequiresYearly => formatter
                .write_str("BYMONTH selection is currently supported only for yearly recurrence"),
            Self::InvalidByMonth(month) => {
                write!(
                    formatter,
                    "BYMONTH value must be between 1 and 12, got {month}"
                )
            }
            Self::DuplicateByMonth(month) => {
                write!(formatter, "duplicate BYMONTH value {month}")
            }
            Self::ByWeekNoRequiresYearly => formatter
                .write_str("BYWEEKNO selection is supported only for yearly recurrence"),
            Self::InvalidByWeekNo(week_no) => {
                write!(
                    formatter,
                    "BYWEEKNO value must be -53..=-1 or 1..=53, got {week_no}"
                )
            }
            Self::DuplicateByWeekNo(week_no) => {
                write!(formatter, "duplicate BYWEEKNO value {week_no}")
            }
            Self::ByYearDayRequiresYearly => formatter
                .write_str("BYYEARDAY selection is supported only for yearly recurrence"),
            Self::InvalidByYearDay(day) => {
                write!(
                    formatter,
                    "BYYEARDAY value must be -366..=-1 or 1..=366, got {day}"
                )
            }
            Self::DuplicateByYearDay(day) => {
                write!(formatter, "duplicate BYYEARDAY value {day}")
            }
            Self::ByMonthDayRequiresMonthlyOrYearly => formatter
                .write_str("BYMONTHDAY selection requires monthly or yearly recurrence"),
            Self::InvalidByMonthDay(day) => {
                write!(
                    formatter,
                    "BYMONTHDAY value must be -31..=-1 or 1..=31, got {day}"
                )
            }
            Self::DuplicateByMonthDay(day) => {
                write!(formatter, "duplicate BYMONTHDAY value {day}")
            }
            Self::OrdinalByWeekdayRequiresMonthlyOrYearly => {
                formatter.write_str("ordinal BYDAY selection requires monthly or yearly recurrence")
            }
            Self::InvalidOrdinalByWeekday(ordinal) => {
                write!(
                    formatter,
                    "ordinal BYDAY is outside the supported recurrence-context range, got {ordinal}"
                )
            }
            Self::DuplicateOrdinalByWeekday(ordinal, weekday) => {
                write!(
                    formatter,
                    "duplicate ordinal BYDAY selector {ordinal} {weekday}"
                )
            }
            Self::OrdinalByWeekdayWithByWeekNo => formatter.write_str(
                "ordinal BYDAY selectors cannot be combined with yearly BYWEEKNO",
            ),
            Self::BySetPosRequiresSelector => {
                formatter.write_str("BYSETPOS requires at least one other BY selector")
            }
            Self::InvalidBySetPos(position) => {
                write!(
                    formatter,
                    "BYSETPOS value must be -366..=-1 or 1..=366, got {position}"
                )
            }
            Self::DuplicateBySetPos(position) => {
                write!(formatter, "duplicate BYSETPOS value {position}")
            }
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
            Self::UnknownOverrideTarget(key) => {
                write!(
                    formatter,
                    "recurrence override does not target an RRULE/RDATE occurrence: {key}"
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
            if !recurrence_generates_original_time(&self.time, rule, &occurrence_override.original)?
            {
                return Err(RecurrenceError::UnknownOverrideTarget(key));
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
        let window = RecurrenceExpansionWindow {
            start,
            end_exclusive,
            display_timezone,
        };

        let mut occurrences = Vec::new();
        let mut seen_occurrence_ids = HashSet::new();
        let mut generated_keys = HashSet::new();
        let mut recurrence_period = 0_u32;
        let mut emitted = 0_u32;
        let mut stop_rule = false;

        while !stop_rule {
            if rule.count.is_some_and(|count| emitted >= count) {
                break;
            }

            let candidates = recurrence_candidates_for_period(&self.time, rule, recurrence_period)?;
            recurrence_period = recurrence_period
                .checked_add(1)
                .ok_or(RecurrenceError::ArithmeticOverflow)?;

            if candidates.is_empty() {
                continue;
            }

            for time in candidates {
                if rule.count.is_some_and(|count| emitted >= count) {
                    stop_rule = true;
                    break;
                }

                let occurrence_date = recurrence_rule_date(&time)?;
                if rule.until.is_some_and(|until| occurrence_date > until) {
                    stop_rule = true;
                    break;
                }

                let recurrence_index = emitted;
                emitted = emitted
                    .checked_add(1)
                    .ok_or(RecurrenceError::ArithmeticOverflow)?;
                generated_keys.insert(recurrence_key(&time)?);

                if let Some(occurrence) = materialize_recurrence_occurrence(
                    self,
                    rule,
                    time,
                    Some(recurrence_index),
                    RecurrenceOccurrenceOrigin::Rule,
                    window,
                )? && seen_occurrence_ids.insert(occurrence.id)
                {
                    occurrences.push(occurrence);
                }

                if occurrence_date >= end_exclusive {
                    stop_rule = true;
                    break;
                }
            }
        }

        for rdate in &rule.rdates {
            generated_keys.insert(recurrence_key(rdate)?);
            if let Some(occurrence) = materialize_recurrence_occurrence(
                self,
                rule,
                rdate.clone(),
                None,
                RecurrenceOccurrenceOrigin::RDate,
                window,
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
                self,
                rule,
                occurrence_override.original.clone(),
                None,
                RecurrenceOccurrenceOrigin::DetachedOverride,
                window,
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

fn recurrence_candidates_for_period(
    base: &TimeSpec,
    rule: &RecurrenceRule,
    period: u32,
) -> Result<Vec<TimeSpec>, RecurrenceError> {
    let candidates = recurrence_candidates_before_set_pos(base, rule, period)?;
    Ok(apply_set_positions(candidates, &rule.by_set_pos))
}

fn recurrence_candidates_before_set_pos(
    base: &TimeSpec,
    rule: &RecurrenceRule,
    period: u32,
) -> Result<Vec<TimeSpec>, RecurrenceError> {
    if rule.frequency == RecurrenceFrequency::Weekly && !rule.by_weekday.is_empty() {
        return weekly_recurrence_candidates(base, rule, period);
    }
    if rule.frequency == RecurrenceFrequency::Monthly && !rule.by_weekday.is_empty() {
        return monthly_plain_weekday_candidates(base, rule, period);
    }
    if rule.frequency == RecurrenceFrequency::Monthly
        && !rule.by_month_day.is_empty()
        && !rule.by_month_weekday.is_empty()
    {
        return monthly_combined_recurrence_candidates(base, rule, period);
    }
    if rule.frequency == RecurrenceFrequency::Monthly && !rule.by_month_day.is_empty() {
        return monthly_recurrence_candidates(base, rule, period);
    }
    if rule.frequency == RecurrenceFrequency::Monthly && !rule.by_month_weekday.is_empty() {
        return monthly_ordinal_weekday_candidates(base, rule, period);
    }
    if rule.frequency == RecurrenceFrequency::Yearly && !rule.by_week_no.is_empty() {
        return yearly_week_number_candidates(base, rule, period);
    }
    if rule.frequency == RecurrenceFrequency::Yearly
        && (!rule.by_month.is_empty()
            || !rule.by_year_day.is_empty()
            || !rule.by_weekday.is_empty()
            || !rule.by_month_day.is_empty()
            || !rule.by_month_weekday.is_empty())
    {
        return yearly_recurrence_candidates(base, rule, period);
    }

    Ok(shift_recurrence_time(base, rule, period)?
        .into_iter()
        .collect())
}

fn apply_set_positions(candidates: Vec<TimeSpec>, positions: &[i16]) -> Vec<TimeSpec> {
    if positions.is_empty() || candidates.is_empty() {
        return candidates;
    }

    let candidate_count = candidates.len();
    let mut indices = positions
        .iter()
        .filter_map(|position| {
            if *position > 0 {
                let index = usize::try_from(i32::from(*position) - 1).ok()?;
                (index < candidate_count).then_some(index)
            } else {
                let from_end = usize::try_from(-i32::from(*position)).ok()?;
                candidate_count.checked_sub(from_end)
            }
        })
        .collect::<Vec<_>>();
    indices.sort_unstable();
    indices.dedup();

    indices
        .into_iter()
        .filter_map(|index| candidates.get(index).cloned())
        .collect()
}

fn weekly_recurrence_candidates(
    base: &TimeSpec,
    rule: &RecurrenceRule,
    period: u32,
) -> Result<Vec<TimeSpec>, RecurrenceError> {
    let base_date = recurrence_rule_date(base)?;
    let base_weekday = u64::from(base_date.weekday().num_days_from_monday());
    let days_since_week_start = (base_weekday + 7 - rule.week_start.offset_from_monday()) % 7;
    let week_start = base_date
        .checked_sub_days(Days::new(days_since_week_start))
        .ok_or(RecurrenceError::ArithmeticOverflow)?;
    let week_offset = u64::from(rule.interval)
        .checked_mul(u64::from(period))
        .and_then(|weeks| weeks.checked_mul(7))
        .ok_or(RecurrenceError::ArithmeticOverflow)?;
    let active_week_start = week_start
        .checked_add_days(Days::new(week_offset))
        .ok_or(RecurrenceError::ArithmeticOverflow)?;

    let mut weekdays = rule.by_weekday.clone();
    weekdays.sort_by_key(|weekday| weekday.offset_from(rule.week_start));

    let mut candidates = Vec::with_capacity(weekdays.len());
    for weekday in weekdays {
        let candidate_date = active_week_start
            .checked_add_days(Days::new(weekday.offset_from(rule.week_start)))
            .ok_or(RecurrenceError::ArithmeticOverflow)?;
        if period == 0 && candidate_date < base_date {
            continue;
        }
        if let Some(candidate) = shift_time_to_date(base, base_date, candidate_date)? {
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

fn monthly_plain_weekday_candidates(
    base: &TimeSpec,
    rule: &RecurrenceRule,
    period: u32,
) -> Result<Vec<TimeSpec>, RecurrenceError> {
    let base_date = recurrence_rule_date(base)?;
    let base_month_start = NaiveDate::from_ymd_opt(base_date.year(), base_date.month(), 1)
        .ok_or(RecurrenceError::ArithmeticOverflow)?;
    let month_offset = rule
        .interval
        .checked_mul(period)
        .ok_or(RecurrenceError::ArithmeticOverflow)?;
    let Some(active_month_start) = add_months_preserving_day(base_month_start, month_offset)?
    else {
        return Ok(Vec::new());
    };

    let active_year = active_month_start.year();
    let active_month = active_month_start.month();
    let month_start_weekday = u64::from(active_month_start.weekday().num_days_from_monday());
    let mut weekdays = rule.by_weekday.clone();
    weekdays.sort_by_key(|weekday| weekday.offset_from_monday());

    let mut candidate_dates = Vec::new();
    for weekday in weekdays {
        let first_offset = (weekday.offset_from_monday() + 7 - month_start_weekday) % 7;
        let mut candidate_date = active_month_start
            .checked_add_days(Days::new(first_offset))
            .ok_or(RecurrenceError::ArithmeticOverflow)?;
        while candidate_date.year() == active_year && candidate_date.month() == active_month {
            candidate_dates.push(candidate_date);
            candidate_date = candidate_date
                .checked_add_days(Days::new(7))
                .ok_or(RecurrenceError::ArithmeticOverflow)?;
        }
    }

    for selector in &rule.by_month_weekday {
        if let Some(candidate_date) = resolve_ordinal_weekday(active_month_start, *selector) {
            candidate_dates.push(candidate_date);
        }
    }

    if !rule.by_month_day.is_empty() {
        let month_day_dates = rule
            .by_month_day
            .iter()
            .filter_map(|selector| resolve_month_day(active_month_start, *selector))
            .collect::<HashSet<_>>();
        candidate_dates.retain(|date| month_day_dates.contains(date));
    }

    candidate_dates.retain(|date| period != 0 || *date >= base_date);
    candidate_dates.sort_unstable();
    candidate_dates.dedup();

    let mut candidates = Vec::with_capacity(candidate_dates.len());
    for candidate_date in candidate_dates {
        if let Some(candidate) = shift_time_to_date(base, base_date, candidate_date)? {
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

fn monthly_recurrence_candidates(
    base: &TimeSpec,
    rule: &RecurrenceRule,
    period: u32,
) -> Result<Vec<TimeSpec>, RecurrenceError> {
    let base_date = recurrence_rule_date(base)?;
    let base_month_start = NaiveDate::from_ymd_opt(base_date.year(), base_date.month(), 1)
        .ok_or(RecurrenceError::ArithmeticOverflow)?;
    let month_offset = rule
        .interval
        .checked_mul(period)
        .ok_or(RecurrenceError::ArithmeticOverflow)?;
    let Some(active_month_start) = add_months_preserving_day(base_month_start, month_offset)?
    else {
        return Ok(Vec::new());
    };

    let mut candidate_dates = Vec::with_capacity(rule.by_month_day.len());
    for day in &rule.by_month_day {
        let Some(candidate_date) = resolve_month_day(active_month_start, *day) else {
            continue;
        };
        if period == 0 && candidate_date < base_date {
            continue;
        }
        candidate_dates.push(candidate_date);
    }
    candidate_dates.sort_unstable();
    candidate_dates.dedup();

    let mut candidates = Vec::with_capacity(candidate_dates.len());
    for candidate_date in candidate_dates {
        if let Some(candidate) = shift_time_to_date(base, base_date, candidate_date)? {
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

fn monthly_combined_recurrence_candidates(
    base: &TimeSpec,
    rule: &RecurrenceRule,
    period: u32,
) -> Result<Vec<TimeSpec>, RecurrenceError> {
    let base_date = recurrence_rule_date(base)?;
    let base_month_start = NaiveDate::from_ymd_opt(base_date.year(), base_date.month(), 1)
        .ok_or(RecurrenceError::ArithmeticOverflow)?;
    let month_offset = rule
        .interval
        .checked_mul(period)
        .ok_or(RecurrenceError::ArithmeticOverflow)?;
    let Some(active_month_start) = add_months_preserving_day(base_month_start, month_offset)?
    else {
        return Ok(Vec::new());
    };

    let month_day_dates = rule
        .by_month_day
        .iter()
        .filter_map(|selector| resolve_month_day(active_month_start, *selector))
        .collect::<HashSet<_>>();

    let mut candidate_dates = rule
        .by_month_weekday
        .iter()
        .filter_map(|selector| resolve_ordinal_weekday(active_month_start, *selector))
        .filter(|date| month_day_dates.contains(date))
        .filter(|date| period != 0 || *date >= base_date)
        .collect::<Vec<_>>();
    candidate_dates.sort_unstable();
    candidate_dates.dedup();

    let mut candidates = Vec::with_capacity(candidate_dates.len());
    for candidate_date in candidate_dates {
        if let Some(candidate) = shift_time_to_date(base, base_date, candidate_date)? {
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

fn monthly_ordinal_weekday_candidates(
    base: &TimeSpec,
    rule: &RecurrenceRule,
    period: u32,
) -> Result<Vec<TimeSpec>, RecurrenceError> {
    let base_date = recurrence_rule_date(base)?;
    let base_month_start = NaiveDate::from_ymd_opt(base_date.year(), base_date.month(), 1)
        .ok_or(RecurrenceError::ArithmeticOverflow)?;
    let month_offset = rule
        .interval
        .checked_mul(period)
        .ok_or(RecurrenceError::ArithmeticOverflow)?;
    let Some(active_month_start) = add_months_preserving_day(base_month_start, month_offset)?
    else {
        return Ok(Vec::new());
    };

    let mut candidate_dates = Vec::with_capacity(rule.by_month_weekday.len());
    for selector in &rule.by_month_weekday {
        let Some(candidate_date) = resolve_ordinal_weekday(active_month_start, *selector) else {
            continue;
        };
        if period == 0 && candidate_date < base_date {
            continue;
        }
        candidate_dates.push(candidate_date);
    }
    candidate_dates.sort_unstable();
    candidate_dates.dedup();

    let mut candidates = Vec::with_capacity(candidate_dates.len());
    for candidate_date in candidate_dates {
        if let Some(candidate) = shift_time_to_date(base, base_date, candidate_date)? {
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

fn resolve_ordinal_weekday(
    month_start: NaiveDate,
    selector: RecurrenceOrdinalWeekday,
) -> Option<NaiveDate> {
    let target = i64::try_from(selector.weekday.offset_from_monday()).ok()?;
    if selector.ordinal > 0 {
        let first = i64::from(month_start.weekday().num_days_from_monday());
        let weekday_delta = u64::try_from((target - first + 7) % 7).ok()?;
        let ordinal_weeks = u64::try_from(i16::from(selector.ordinal) - 1).ok()?;
        let total_days = weekday_delta.checked_add(ordinal_weeks.checked_mul(7)?)?;
        let candidate = month_start.checked_add_days(Days::new(total_days))?;
        return (candidate.month() == month_start.month()).then_some(candidate);
    }

    let next_month = next_month_start(month_start)?;
    let last_day = next_month.checked_sub_days(Days::new(1))?;
    let last = i64::from(last_day.weekday().num_days_from_monday());
    let weekday_delta = u64::try_from((last - target + 7) % 7).ok()?;
    let ordinal_weeks = u64::try_from(-i16::from(selector.ordinal) - 1).ok()?;
    let total_days = weekday_delta.checked_add(ordinal_weeks.checked_mul(7)?)?;
    let candidate = last_day.checked_sub_days(Days::new(total_days))?;
    (candidate.month() == month_start.month()).then_some(candidate)
}

fn resolve_month_day(month_start: NaiveDate, selector: i8) -> Option<NaiveDate> {
    if selector > 0 {
        return NaiveDate::from_ymd_opt(
            month_start.year(),
            month_start.month(),
            u32::try_from(selector).ok()?,
        );
    }

    let next_month = next_month_start(month_start)?;
    let days_from_end = u64::try_from(-i16::from(selector)).ok()?;
    let candidate = next_month.checked_sub_days(Days::new(days_from_end))?;
    (candidate.year() == month_start.year() && candidate.month() == month_start.month())
        .then_some(candidate)
}

fn recurrence_week_one_start(
    year: i32,
    week_start: RecurrenceWeekday,
) -> Result<NaiveDate, RecurrenceError> {
    let january_four =
        NaiveDate::from_ymd_opt(year, 1, 4).ok_or(RecurrenceError::ArithmeticOverflow)?;
    let january_four_weekday = u64::from(january_four.weekday().num_days_from_monday());
    let days_since_week_start = (january_four_weekday + 7 - week_start.offset_from_monday()) % 7;
    january_four
        .checked_sub_days(Days::new(days_since_week_start))
        .ok_or(RecurrenceError::ArithmeticOverflow)
}

fn recurrence_weeks_in_year(
    year: i32,
    week_start: RecurrenceWeekday,
) -> Result<i16, RecurrenceError> {
    let current = recurrence_week_one_start(year, week_start)?;
    let next_year = year
        .checked_add(1)
        .ok_or(RecurrenceError::ArithmeticOverflow)?;
    let next = recurrence_week_one_start(next_year, week_start)?;
    i16::try_from((next - current).num_days() / 7).map_err(|_| RecurrenceError::ArithmeticOverflow)
}

fn resolve_week_number_start(
    year: i32,
    selector: i8,
    week_start: RecurrenceWeekday,
) -> Result<Option<NaiveDate>, RecurrenceError> {
    let week_count = recurrence_weeks_in_year(year, week_start)?;
    let selector = i16::from(selector);
    let ordinal = if selector > 0 {
        selector
    } else {
        week_count
            .checked_add(selector)
            .and_then(|value| value.checked_add(1))
            .ok_or(RecurrenceError::ArithmeticOverflow)?
    };
    if !(1..=week_count).contains(&ordinal) {
        return Ok(None);
    }

    let week_offset = u64::try_from(ordinal - 1)
        .ok()
        .and_then(|weeks| weeks.checked_mul(7))
        .ok_or(RecurrenceError::ArithmeticOverflow)?;
    let week_one = recurrence_week_one_start(year, week_start)?;
    Ok(Some(
        week_one
            .checked_add_days(Days::new(week_offset))
            .ok_or(RecurrenceError::ArithmeticOverflow)?,
    ))
}

fn yearly_week_number_candidates(
    base: &TimeSpec,
    rule: &RecurrenceRule,
    period: u32,
) -> Result<Vec<TimeSpec>, RecurrenceError> {
    let base_date = recurrence_rule_date(base)?;
    let year_offset = rule
        .interval
        .checked_mul(period)
        .ok_or(RecurrenceError::ArithmeticOverflow)?;
    let year_offset =
        i32::try_from(year_offset).map_err(|_| RecurrenceError::ArithmeticOverflow)?;
    let active_year = base_date
        .year()
        .checked_add(year_offset)
        .ok_or(RecurrenceError::ArithmeticOverflow)?;

    let weekday_offsets = if rule.by_weekday.is_empty() {
        let base_weekday = u64::from(base_date.weekday().num_days_from_monday());
        vec![(base_weekday + 7 - rule.week_start.offset_from_monday()) % 7]
    } else {
        let mut weekdays = rule.by_weekday.clone();
        weekdays.sort_by_key(|weekday| weekday.offset_from(rule.week_start));
        weekdays
            .into_iter()
            .map(|weekday| weekday.offset_from(rule.week_start))
            .collect::<Vec<_>>()
    };

    let selected_months =
        (!rule.by_month.is_empty()).then(|| rule.by_month.iter().copied().collect::<HashSet<_>>());
    let selected_year_days = if rule.by_year_day.is_empty() {
        None
    } else {
        Some(
            rule.by_year_day
                .iter()
                .filter_map(|selector| resolve_year_day(active_year, *selector))
                .collect::<HashSet<_>>(),
        )
    };

    let mut candidate_dates = Vec::new();
    for week_no in &rule.by_week_no {
        let Some(week_start) = resolve_week_number_start(active_year, *week_no, rule.week_start)?
        else {
            continue;
        };

        for weekday_offset in &weekday_offsets {
            let candidate_date = week_start
                .checked_add_days(Days::new(*weekday_offset))
                .ok_or(RecurrenceError::ArithmeticOverflow)?;
            if period == 0 && candidate_date < base_date {
                continue;
            }
            if selected_months.as_ref().is_some_and(|months| {
                u8::try_from(candidate_date.month())
                    .ok()
                    .is_none_or(|month| !months.contains(&month))
            }) {
                continue;
            }
            if selected_year_days
                .as_ref()
                .is_some_and(|dates| !dates.contains(&candidate_date))
            {
                continue;
            }
            if !rule.by_month_day.is_empty() {
                let Some(month_start) =
                    NaiveDate::from_ymd_opt(candidate_date.year(), candidate_date.month(), 1)
                else {
                    continue;
                };
                if !rule
                    .by_month_day
                    .iter()
                    .filter_map(|selector| resolve_month_day(month_start, *selector))
                    .any(|date| date == candidate_date)
                {
                    continue;
                }
            }
            candidate_dates.push(candidate_date);
        }
    }

    candidate_dates.sort_unstable();
    candidate_dates.dedup();

    let mut candidates = Vec::with_capacity(candidate_dates.len());
    for candidate_date in candidate_dates {
        if let Some(candidate) = shift_time_to_date(base, base_date, candidate_date)? {
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

fn resolve_year_day(year: i32, selector: i16) -> Option<NaiveDate> {
    if selector > 0 {
        return NaiveDate::from_yo_opt(year, u32::try_from(selector).ok()?);
    }

    let next_year = year.checked_add(1)?;
    let next_year_start = NaiveDate::from_ymd_opt(next_year, 1, 1)?;
    let days_from_end = u64::try_from(-i32::from(selector)).ok()?;
    let candidate = next_year_start.checked_sub_days(Days::new(days_from_end))?;
    (candidate.year() == year).then_some(candidate)
}

fn resolve_ordinal_weekday_in_year(
    year: i32,
    selector: RecurrenceOrdinalWeekday,
) -> Option<NaiveDate> {
    let year_start = NaiveDate::from_ymd_opt(year, 1, 1)?;
    let target = i64::try_from(selector.weekday.offset_from_monday()).ok()?;

    if selector.ordinal > 0 {
        let first = i64::from(year_start.weekday().num_days_from_monday());
        let weekday_delta = u64::try_from((target - first + 7) % 7).ok()?;
        let ordinal_weeks = u64::try_from(i16::from(selector.ordinal) - 1).ok()?;
        let total_days = weekday_delta.checked_add(ordinal_weeks.checked_mul(7)?)?;
        let candidate = year_start.checked_add_days(Days::new(total_days))?;
        return (candidate.year() == year).then_some(candidate);
    }

    let next_year = year.checked_add(1)?;
    let next_year_start = NaiveDate::from_ymd_opt(next_year, 1, 1)?;
    let last_day = next_year_start.checked_sub_days(Days::new(1))?;
    let last = i64::from(last_day.weekday().num_days_from_monday());
    let weekday_delta = u64::try_from((last - target + 7) % 7).ok()?;
    let ordinal_weeks = u64::try_from(-i16::from(selector.ordinal) - 1).ok()?;
    let total_days = weekday_delta.checked_add(ordinal_weeks.checked_mul(7)?)?;
    let candidate = last_day.checked_sub_days(Days::new(total_days))?;
    (candidate.year() == year).then_some(candidate)
}

fn yearly_recurrence_candidates(
    base: &TimeSpec,
    rule: &RecurrenceRule,
    period: u32,
) -> Result<Vec<TimeSpec>, RecurrenceError> {
    let base_date = recurrence_rule_date(base)?;
    let year_offset = rule
        .interval
        .checked_mul(period)
        .ok_or(RecurrenceError::ArithmeticOverflow)?;
    let year_offset =
        i32::try_from(year_offset).map_err(|_| RecurrenceError::ArithmeticOverflow)?;
    let active_year = base_date
        .year()
        .checked_add(year_offset)
        .ok_or(RecurrenceError::ArithmeticOverflow)?;

    let mut candidate_dates = if rule.by_year_day.is_empty() {
        Vec::new()
    } else {
        rule.by_year_day
            .iter()
            .filter_map(|selector| resolve_year_day(active_year, *selector))
            .collect::<Vec<_>>()
    };

    if !rule.by_year_day.is_empty() {
        if !rule.by_month.is_empty() {
            let selected_months = rule.by_month.iter().copied().collect::<HashSet<_>>();
            candidate_dates.retain(|date| {
                u8::try_from(date.month())
                    .ok()
                    .is_some_and(|month| selected_months.contains(&month))
            });
        }
        if !rule.by_weekday.is_empty() || !rule.by_month_weekday.is_empty() {
            candidate_dates.retain(|date| {
                let weekday = u64::from(date.weekday().num_days_from_monday());
                let matches_plain = rule
                    .by_weekday
                    .iter()
                    .any(|selector| selector.offset_from_monday() == weekday);
                let matches_ordinal = if rule.by_month.is_empty() {
                    rule.by_month_weekday
                        .iter()
                        .filter_map(|selector| {
                            resolve_ordinal_weekday_in_year(active_year, *selector)
                        })
                        .any(|candidate| candidate == *date)
                } else {
                    NaiveDate::from_ymd_opt(date.year(), date.month(), 1).is_some_and(
                        |month_start| {
                            rule.by_month_weekday
                                .iter()
                                .filter_map(|selector| {
                                    resolve_ordinal_weekday(month_start, *selector)
                                })
                                .any(|candidate| candidate == *date)
                        },
                    )
                };
                matches_plain || matches_ordinal
            });
        }
        if !rule.by_month_day.is_empty() {
            candidate_dates.retain(|date| {
                let Some(month_start) = NaiveDate::from_ymd_opt(date.year(), date.month(), 1)
                else {
                    return false;
                };
                rule.by_month_day
                    .iter()
                    .filter_map(|selector| resolve_month_day(month_start, *selector))
                    .any(|candidate| candidate == *date)
            });
        }
    } else if rule.by_month.is_empty() && !rule.by_month_weekday.is_empty() {
        if !rule.by_weekday.is_empty() {
            let year_start = NaiveDate::from_ymd_opt(active_year, 1, 1)
                .ok_or(RecurrenceError::ArithmeticOverflow)?;
            let year_start_weekday = u64::from(year_start.weekday().num_days_from_monday());
            let mut weekdays = rule.by_weekday.clone();
            weekdays.sort_by_key(|weekday| weekday.offset_from_monday());

            for weekday in weekdays {
                let first_offset =
                    (weekday.offset_from_monday() + 7 - year_start_weekday) % 7;
                let mut candidate_date = year_start
                    .checked_add_days(Days::new(first_offset))
                    .ok_or(RecurrenceError::ArithmeticOverflow)?;
                while candidate_date.year() == active_year {
                    candidate_dates.push(candidate_date);
                    candidate_date = candidate_date
                        .checked_add_days(Days::new(7))
                        .ok_or(RecurrenceError::ArithmeticOverflow)?;
                }
            }
        }

        for selector in &rule.by_month_weekday {
            if let Some(candidate_date) =
                resolve_ordinal_weekday_in_year(active_year, *selector)
            {
                candidate_dates.push(candidate_date);
            }
        }
    } else {
        let mut months = if rule.by_month.is_empty() {
            (1_u8..=12).collect::<Vec<_>>()
        } else {
            rule.by_month.clone()
        };
        months.sort_unstable();

        for month in months {
            let Some(month_start) = NaiveDate::from_ymd_opt(active_year, u32::from(month), 1)
            else {
                continue;
            };

            if !rule.by_weekday.is_empty() || !rule.by_month_weekday.is_empty() {
                let month_day_dates = (!rule.by_month_day.is_empty()).then(|| {
                    rule.by_month_day
                        .iter()
                        .filter_map(|selector| resolve_month_day(month_start, *selector))
                        .collect::<HashSet<_>>()
                });

                if !rule.by_weekday.is_empty() {
                    let month_start_weekday =
                        u64::from(month_start.weekday().num_days_from_monday());
                    let mut weekdays = rule.by_weekday.clone();
                    weekdays.sort_by_key(|weekday| weekday.offset_from_monday());

                    for weekday in weekdays {
                        let first_offset =
                            (weekday.offset_from_monday() + 7 - month_start_weekday) % 7;
                        let mut candidate_date = month_start
                            .checked_add_days(Days::new(first_offset))
                            .ok_or(RecurrenceError::ArithmeticOverflow)?;
                        while candidate_date.year() == active_year
                            && candidate_date.month() == u32::from(month)
                        {
                            if month_day_dates
                                .as_ref()
                                .is_none_or(|dates| dates.contains(&candidate_date))
                            {
                                candidate_dates.push(candidate_date);
                            }
                            candidate_date = candidate_date
                                .checked_add_days(Days::new(7))
                                .ok_or(RecurrenceError::ArithmeticOverflow)?;
                        }
                    }
                }

                for selector in &rule.by_month_weekday {
                    let Some(candidate_date) = resolve_ordinal_weekday(month_start, *selector)
                    else {
                        continue;
                    };
                    if month_day_dates
                        .as_ref()
                        .is_some_and(|dates| !dates.contains(&candidate_date))
                    {
                        continue;
                    }
                    candidate_dates.push(candidate_date);
                }
            } else if !rule.by_month_day.is_empty() {
                for day in &rule.by_month_day {
                    let Some(candidate_date) = resolve_month_day(month_start, *day) else {
                        continue;
                    };
                    candidate_dates.push(candidate_date);
                }
            } else {
                let Some(candidate_date) =
                    NaiveDate::from_ymd_opt(active_year, u32::from(month), base_date.day())
                else {
                    continue;
                };
                candidate_dates.push(candidate_date);
            }
        }
    }

    if rule.by_month.is_empty()
        && !rule.by_month_day.is_empty()
        && !rule.by_month_weekday.is_empty()
        && rule.by_year_day.is_empty()
    {
        candidate_dates.retain(|date| {
            let Some(month_start) = NaiveDate::from_ymd_opt(date.year(), date.month(), 1) else {
                return false;
            };
            rule.by_month_day
                .iter()
                .filter_map(|selector| resolve_month_day(month_start, *selector))
                .any(|candidate| candidate == *date)
        });
    }

    candidate_dates.retain(|candidate_date| period != 0 || *candidate_date >= base_date);
    candidate_dates.sort_unstable();
    candidate_dates.dedup();

    let mut candidates = Vec::with_capacity(candidate_dates.len());
    for candidate_date in candidate_dates {
        if let Some(candidate) = shift_time_to_date(base, base_date, candidate_date)? {
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

fn shift_time_to_date(
    time: &TimeSpec,
    base_date: NaiveDate,
    target_date: NaiveDate,
) -> Result<Option<TimeSpec>, RecurrenceError> {
    let delta_days = (target_date - base_date).num_days();
    match time {
        TimeSpec::DateOnly {
            start,
            end_exclusive,
        } => {
            let shifted_end = match end_exclusive {
                Some(end) => Some(
                    target_date
                        .checked_add_signed(Duration::days((*end - *start).num_days()))
                        .ok_or(RecurrenceError::ArithmeticOverflow)?,
                ),
                None => None,
            };
            Ok(Some(TimeSpec::DateOnly {
                start: target_date,
                end_exclusive: shifted_end,
            }))
        }
        TimeSpec::AllDay {
            start,
            end_exclusive,
        } => {
            let shifted_end = match end_exclusive {
                Some(end) => Some(
                    target_date
                        .checked_add_signed(Duration::days((*end - *start).num_days()))
                        .ok_or(RecurrenceError::ArithmeticOverflow)?,
                ),
                None => None,
            };
            Ok(Some(TimeSpec::AllDay {
                start: target_date,
                end_exclusive: shifted_end,
            }))
        }
        TimeSpec::Floating {
            start,
            end,
            source_timezone,
        } => {
            let shifted_start = start
                .checked_add_signed(Duration::days(delta_days))
                .ok_or(RecurrenceError::ArithmeticOverflow)?;
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
            let shifted_local = NaiveDateTime::new(target_date, local_start.time());
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

fn recurrence_generates_original_time(
    base: &TimeSpec,
    rule: &RecurrenceRule,
    target: &TimeSpec,
) -> Result<bool, RecurrenceError> {
    let target_key = recurrence_key(target)?;
    for rdate in &rule.rdates {
        if recurrence_key(rdate)? == target_key {
            return Ok(true);
        }
    }

    let target_date = recurrence_rule_date(target)?;
    let mut recurrence_period = 0_u32;
    let mut emitted = 0_u32;

    loop {
        if rule.count.is_some_and(|count| emitted >= count) {
            return Ok(false);
        }

        let candidates = recurrence_candidates_for_period(base, rule, recurrence_period)?;
        recurrence_period = recurrence_period
            .checked_add(1)
            .ok_or(RecurrenceError::ArithmeticOverflow)?;

        if candidates.is_empty() {
            continue;
        }

        for time in candidates {
            if rule.count.is_some_and(|count| emitted >= count) {
                return Ok(false);
            }

            let occurrence_date = recurrence_rule_date(&time)?;
            if rule.until.is_some_and(|until| occurrence_date > until) {
                return Ok(false);
            }

            emitted = emitted
                .checked_add(1)
                .ok_or(RecurrenceError::ArithmeticOverflow)?;

            if recurrence_key(&time)? == target_key {
                return Ok(true);
            }
            if occurrence_date >= target_date {
                return Ok(false);
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RecurrenceExpansionWindow {
    start: NaiveDate,
    end_exclusive: NaiveDate,
    display_timezone: Tz,
}

fn materialize_recurrence_occurrence(
    event: &TemporalEvent,
    rule: &RecurrenceRule,
    original_time: TimeSpec,
    recurrence_index: Option<u32>,
    origin: RecurrenceOccurrenceOrigin,
    window: RecurrenceExpansionWindow,
) -> Result<Option<EventOccurrence>, RecurrenceError> {
    let key = recurrence_key(&original_time)?;

    for exdate in &rule.exdates {
        if recurrence_key(exdate)? == key {
            return Ok(None);
        }
    }

    let mut time = original_time.clone();
    let mut status = event.status;
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

    if !time.overlaps_date_window(window.start, window.end_exclusive, window.display_timezone) {
        return Ok(None);
    }

    Ok(Some(EventOccurrence {
        id: occurrence_identity(event.id, &original_time)?,
        event_id: event.id,
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
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
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
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
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
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
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
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
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
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
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
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
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
    fn override_for_non_occurrence_is_rejected() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 1).expect("start");
        let invalid_target = TimeSpec::DateOnly {
            start: start + Duration::days(2),
            end_exclusive: None,
        };
        let mut event = TemporalEvent::new(
            "Finite weekly",
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
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: vec![RecurrenceOverride {
                original: invalid_target,
                replacement: Some(TimeSpec::DateOnly {
                    start: start + Duration::days(3),
                    end_exclusive: None,
                }),
                cancelled: false,
            }],
        });

        assert!(matches!(
            event.validate_recurrence(),
            Err(RecurrenceError::UnknownOverrideTarget(_))
        ));
    }

    #[test]
    fn weekly_byday_expands_selected_weekdays_in_chronological_order() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Mon Wed Fri",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Weekly,
            interval: 1,
            count: Some(5),
            until: None,
            week_start: Default::default(),
            by_weekday: vec![
                RecurrenceWeekday::Friday,
                RecurrenceWeekday::Monday,
                RecurrenceWeekday::Wednesday,
            ],
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(start, start + Duration::days(14), chrono_tz::UTC)
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 10, 2).expect("fri"),
                NaiveDate::from_ymd_opt(2026, 10, 5).expect("mon"),
                NaiveDate::from_ymd_opt(2026, 10, 7).expect("wed"),
                NaiveDate::from_ymd_opt(2026, 10, 9).expect("fri"),
                NaiveDate::from_ymd_opt(2026, 10, 12).expect("mon"),
            ]
        );
    }

    #[test]
    fn weekly_byday_interval_skips_inactive_weeks() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 5).expect("start");
        let mut event = TemporalEvent::new(
            "Alternate weeks",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Weekly,
            interval: 2,
            count: Some(4),
            until: None,
            week_start: Default::default(),
            by_weekday: vec![RecurrenceWeekday::Monday, RecurrenceWeekday::Wednesday],
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(start, start + Duration::days(28), chrono_tz::UTC)
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 10, 5).expect("m1"),
                NaiveDate::from_ymd_opt(2026, 10, 7).expect("w1"),
                NaiveDate::from_ymd_opt(2026, 10, 19).expect("m2"),
                NaiveDate::from_ymd_opt(2026, 10, 21).expect("w2"),
            ]
        );
    }

    #[test]
    fn weekly_byday_respects_custom_week_start_for_interval_grouping() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 4).expect("start");
        let mut event = TemporalEvent::new(
            "Sunday anchored",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Weekly,
            interval: 2,
            count: Some(4),
            until: None,
            week_start: RecurrenceWeekday::Sunday,
            by_weekday: vec![RecurrenceWeekday::Sunday, RecurrenceWeekday::Monday],
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(start, start + Duration::days(21), chrono_tz::UTC)
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 10, 4).expect("sun1"),
                NaiveDate::from_ymd_opt(2026, 10, 5).expect("mon1"),
                NaiveDate::from_ymd_opt(2026, 10, 18).expect("sun2"),
                NaiveDate::from_ymd_opt(2026, 10, 19).expect("mon2"),
            ]
        );
    }

    #[test]
    fn custom_week_start_requires_weekly_byday() {
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Weekly);
        rule.week_start = RecurrenceWeekday::Sunday;

        assert!(matches!(
            rule.validate(),
            Err(RecurrenceError::WeekStartRequiresWeekContext)
        ));
    }

    #[test]
    fn byday_is_rejected_for_non_weekly_frequency() {
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.by_weekday = vec![RecurrenceWeekday::Monday];

        assert!(matches!(
            rule.validate(),
            Err(RecurrenceError::ByWeekdayRequiresSupportedContext)
        ));
    }

    #[test]
    fn byday_rejects_duplicate_weekdays() {
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Weekly);
        rule.by_weekday = vec![RecurrenceWeekday::Monday, RecurrenceWeekday::Monday];

        assert!(matches!(
            rule.validate(),
            Err(RecurrenceError::DuplicateByWeekday("monday"))
        ));
    }

    #[test]
    fn exact_weekly_byday_preserves_source_wall_clock_across_dst() {
        let start_utc = DateTime::parse_from_rfc3339("2026-03-01T14:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        let mut event = TemporalEvent::new(
            "Sunday and Tuesday at nine",
            TimeSpec::Instant {
                start_utc,
                end_utc: None,
                source_timezone: Some("America/New_York".to_string()),
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Weekly,
            interval: 1,
            count: Some(3),
            until: None,
            week_start: Default::default(),
            by_weekday: vec![RecurrenceWeekday::Sunday, RecurrenceWeekday::Tuesday],
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
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

        assert_eq!(occurrences.len(), 3);
        let TimeSpec::Instant {
            start_utc: third, ..
        } = occurrences[2].time
        else {
            panic!("instant occurrence");
        };
        assert_eq!(
            third
                .with_timezone(&chrono_tz::America::New_York)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
            "2026-03-08 09:00"
        );
        assert_eq!(third.format("%H:%M").to_string(), "13:00");
    }

    #[test]
    fn weekly_byday_integrates_with_exdate_and_moved_override() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 5).expect("start");
        let excluded = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 10, 7).expect("wed"),
            end_exclusive: None,
        };
        let original = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 10, 9).expect("fri"),
            end_exclusive: None,
        };
        let replacement = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 10, 10).expect("sat"),
            end_exclusive: None,
        };
        let mut event = TemporalEvent::new(
            "Weekday exceptions",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Weekly,
            interval: 1,
            count: Some(4),
            until: None,
            week_start: Default::default(),
            by_weekday: vec![
                RecurrenceWeekday::Monday,
                RecurrenceWeekday::Wednesday,
                RecurrenceWeekday::Friday,
            ],
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: vec![excluded],
            overrides: vec![RecurrenceOverride {
                original: original.clone(),
                replacement: Some(replacement.clone()),
                cancelled: false,
            }],
        });

        let occurrences = event
            .occurrences_in_window(start, start + Duration::days(10), chrono_tz::UTC)
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 10, 5).expect("mon1"),
                NaiveDate::from_ymd_opt(2026, 10, 10).expect("moved"),
                NaiveDate::from_ymd_opt(2026, 10, 12).expect("mon2"),
            ]
        );
        let moved = occurrences
            .iter()
            .find(|occurrence| occurrence.original_time == original)
            .expect("moved occurrence");
        assert_eq!(moved.time, replacement);
        assert!(moved.override_applied);
    }

    #[test]
    fn monthly_plain_byday_expands_all_matching_weekdays() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 5).expect("start");
        let mut event = TemporalEvent::new(
            "Every Monday",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: Some(6),
            until: None,
            week_start: Default::default(),
            by_weekday: vec![RecurrenceWeekday::Monday],
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2026, 3, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 5).expect("jan 5"),
                NaiveDate::from_ymd_opt(2026, 1, 12).expect("jan 12"),
                NaiveDate::from_ymd_opt(2026, 1, 19).expect("jan 19"),
                NaiveDate::from_ymd_opt(2026, 1, 26).expect("jan 26"),
                NaiveDate::from_ymd_opt(2026, 2, 2).expect("feb 2"),
                NaiveDate::from_ymd_opt(2026, 2, 9).expect("feb 9"),
            ]
        );
    }

    #[test]
    fn monthly_plain_byday_with_by_set_pos_selects_last_weekday() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Last weekday of month",
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
            week_start: Default::default(),
            by_weekday: vec![
                RecurrenceWeekday::Monday,
                RecurrenceWeekday::Tuesday,
                RecurrenceWeekday::Wednesday,
                RecurrenceWeekday::Thursday,
                RecurrenceWeekday::Friday,
            ],
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: vec![-1],
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2026, 4, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 30).expect("jan"),
                NaiveDate::from_ymd_opt(2026, 2, 27).expect("feb"),
                NaiveDate::from_ymd_opt(2026, 3, 31).expect("mar"),
            ]
        );
    }

    #[test]
    fn monthly_plain_and_ordinal_byday_union_before_bymonthday_filter() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Mixed BYDAY filter",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: Some(2),
            until: None,
            week_start: Default::default(),
            by_weekday: vec![RecurrenceWeekday::Monday],
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![5, 30],
            by_month_weekday: vec![RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday)],
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2026, 2, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 5).expect("plain Monday"),
                NaiveDate::from_ymd_opt(2026, 1, 30).expect("ordinal Friday"),
            ]
        );
    }

    #[test]
    fn monthly_plain_byday_rejects_custom_week_start_without_week_context() {
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Monthly);
        rule.week_start = RecurrenceWeekday::Sunday;
        rule.by_weekday = vec![RecurrenceWeekday::Monday];
        assert!(matches!(
            rule.validate(),
            Err(RecurrenceError::WeekStartRequiresWeekContext)
        ));
    }

    #[test]
    fn exact_monthly_plain_byday_preserves_source_wall_clock_across_dst() {
        let start_utc = DateTime::parse_from_rfc3339("2026-03-01T14:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        let mut event = TemporalEvent::new(
            "Sundays at nine",
            TimeSpec::Instant {
                start_utc,
                end_utc: None,
                source_timezone: Some("America/New_York".to_string()),
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: Some(3),
            until: None,
            week_start: Default::default(),
            by_weekday: vec![RecurrenceWeekday::Sunday],
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                NaiveDate::from_ymd_opt(2026, 3, 1).expect("start"),
                NaiveDate::from_ymd_opt(2026, 4, 1).expect("end"),
                chrono_tz::America::New_York,
            )
            .expect("expand");

        assert_eq!(occurrences.len(), 3);
        let starts = occurrences
            .iter()
            .map(|occurrence| {
                let TimeSpec::Instant { start_utc, .. } = &occurrence.time else {
                    panic!("instant occurrence");
                };
                (
                    start_utc
                        .with_timezone(&chrono_tz::America::New_York)
                        .format("%Y-%m-%d %H:%M")
                        .to_string(),
                    start_utc.format("%H:%M").to_string(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            starts,
            vec![
                ("2026-03-01 09:00".to_string(), "14:00".to_string()),
                ("2026-03-08 09:00".to_string(), "13:00".to_string()),
                ("2026-03-15 09:00".to_string(), "13:00".to_string()),
            ]
        );
    }

    #[test]
    fn monthly_plain_byday_integrates_with_exdate_and_moved_override() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 5).expect("start");
        let excluded = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 1, 12).expect("excluded"),
            end_exclusive: None,
        };
        let original = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 1, 19).expect("original"),
            end_exclusive: None,
        };
        let replacement = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 1, 20).expect("replacement"),
            end_exclusive: None,
        };
        let mut event = TemporalEvent::new(
            "Monday exceptions",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: Some(4),
            until: None,
            week_start: Default::default(),
            by_weekday: vec![RecurrenceWeekday::Monday],
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: vec![excluded.clone()],
            overrides: vec![RecurrenceOverride {
                original: original.clone(),
                replacement: Some(replacement.clone()),
                cancelled: false,
            }],
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2026, 2, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert!(
            occurrences
                .iter()
                .all(|occurrence| occurrence.original_time != excluded)
        );
        let moved = occurrences
            .iter()
            .find(|occurrence| occurrence.original_time == original)
            .expect("moved occurrence");
        assert_eq!(moved.time, replacement);
        assert!(moved.override_applied);
    }

    #[test]
    fn yearly_by_week_no_and_byday_matches_rfc_week_twenty_example() {
        let start = NaiveDate::from_ymd_opt(1997, 5, 12).expect("start");
        let mut event = TemporalEvent::new(
            "Week twenty Monday",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(3),
            until: None,
            week_start: RecurrenceWeekday::Monday,
            by_weekday: vec![RecurrenceWeekday::Monday],
            by_month: Vec::new(),
            by_week_no: vec![20],
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2000, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(1997, 5, 12).expect("1997"),
                NaiveDate::from_ymd_opt(1998, 5, 11).expect("1998"),
                NaiveDate::from_ymd_opt(1999, 5, 17).expect("1999"),
            ]
        );
    }

    #[test]
    fn yearly_by_week_no_without_byday_preserves_dtstart_weekday() {
        let start = NaiveDate::from_ymd_opt(1997, 5, 14).expect("start");
        let mut event = TemporalEvent::new(
            "Week twenty DTSTART weekday",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(3),
            until: None,
            week_start: RecurrenceWeekday::Monday,
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: vec![20],
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2000, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(1997, 5, 14).expect("1997"),
                NaiveDate::from_ymd_opt(1998, 5, 13).expect("1998"),
                NaiveDate::from_ymd_opt(1999, 5, 19).expect("1999"),
            ]
        );
    }

    #[test]
    fn yearly_negative_by_week_no_honors_custom_week_start() {
        let start = NaiveDate::from_ymd_opt(2026, 12, 30).expect("start");
        let mut event = TemporalEvent::new(
            "Last Sunday-anchored week Wednesday",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(3),
            until: None,
            week_start: RecurrenceWeekday::Sunday,
            by_weekday: vec![RecurrenceWeekday::Wednesday],
            by_month: Vec::new(),
            by_week_no: vec![-1],
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2029, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 12, 30).expect("2026"),
                NaiveDate::from_ymd_opt(2027, 12, 29).expect("2027"),
                NaiveDate::from_ymd_opt(2028, 12, 27).expect("2028"),
            ]
        );
    }

    #[test]
    fn yearly_week_fifty_three_is_skipped_when_week_year_has_only_fifty_two_weeks() {
        let start = NaiveDate::from_ymd_opt(2026, 12, 31).expect("start");
        let mut event = TemporalEvent::new(
            "Week fifty three Thursday",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(2),
            until: None,
            week_start: RecurrenceWeekday::Monday,
            by_weekday: vec![RecurrenceWeekday::Thursday],
            by_month: Vec::new(),
            by_week_no: vec![53],
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2033, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 12, 31).expect("2026"),
                NaiveDate::from_ymd_opt(2032, 12, 30).expect("2032"),
            ]
        );
    }

    #[test]
    fn by_week_no_validation_rejects_wrong_frequency_range_duplicates_and_ordinal_byday() {
        let mut wrong_frequency = RecurrenceRule::new(RecurrenceFrequency::Monthly);
        wrong_frequency.by_week_no = vec![1];
        assert!(matches!(
            wrong_frequency.validate(),
            Err(RecurrenceError::ByWeekNoRequiresYearly)
        ));

        for invalid_week in [-54, 0, 54] {
            let mut invalid = RecurrenceRule::new(RecurrenceFrequency::Yearly);
            invalid.by_week_no = vec![invalid_week];
            assert!(matches!(
                invalid.validate(),
                Err(RecurrenceError::InvalidByWeekNo(value)) if value == invalid_week
            ));
        }

        let mut duplicate = RecurrenceRule::new(RecurrenceFrequency::Yearly);
        duplicate.by_week_no = vec![-1, -1];
        assert!(matches!(
            duplicate.validate(),
            Err(RecurrenceError::DuplicateByWeekNo(-1))
        ));

        let mut ordinal = RecurrenceRule::new(RecurrenceFrequency::Yearly);
        ordinal.by_month = vec![1];
        ordinal.by_week_no = vec![1];
        ordinal.by_month_weekday =
            vec![RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday)];
        assert!(matches!(
            ordinal.validate(),
            Err(RecurrenceError::OrdinalByWeekdayWithByWeekNo)
        ));
    }

    #[test]
    fn yearly_plain_byday_rejects_custom_week_start_without_week_number_context() {
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Yearly);
        rule.week_start = RecurrenceWeekday::Sunday;
        rule.by_weekday = vec![RecurrenceWeekday::Monday];

        assert!(matches!(
            rule.validate(),
            Err(RecurrenceError::WeekStartRequiresWeekContext)
        ));
    }

    #[test]
    fn yearly_by_week_no_filters_with_by_month_and_by_year_day() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Week filters",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(2),
            until: None,
            week_start: RecurrenceWeekday::Monday,
            by_weekday: vec![RecurrenceWeekday::Thursday],
            by_month: vec![1, 7],
            by_week_no: vec![1, 27],
            by_year_day: vec![1, 183],
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2027, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 1).expect("day 1"),
                NaiveDate::from_ymd_opt(2026, 7, 2).expect("day 183"),
            ]
        );
    }

    #[test]
    fn yearly_by_week_no_composes_with_by_set_pos() {
        let start = NaiveDate::from_ymd_opt(1997, 5, 12).expect("start");
        let mut event = TemporalEvent::new(
            "Last candidate across selected weeks",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(2),
            until: None,
            week_start: RecurrenceWeekday::Monday,
            by_weekday: vec![RecurrenceWeekday::Monday, RecurrenceWeekday::Wednesday],
            by_month: Vec::new(),
            by_week_no: vec![20, 21],
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: vec![-1],
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(1999, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(1997, 5, 21).expect("1997"),
                NaiveDate::from_ymd_opt(1998, 5, 20).expect("1998"),
            ]
        );
    }

    #[test]
    fn exact_yearly_by_week_no_preserves_source_wall_clock_across_dst() {
        let start_utc = DateTime::parse_from_rfc3339("2026-01-01T14:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        let mut event = TemporalEvent::new(
            "Week numbers at nine",
            TimeSpec::Instant {
                start_utc,
                end_utc: None,
                source_timezone: Some("America/New_York".to_string()),
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(2),
            until: None,
            week_start: RecurrenceWeekday::Monday,
            by_weekday: vec![RecurrenceWeekday::Thursday],
            by_month: Vec::new(),
            by_week_no: vec![1, 27],
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                NaiveDate::from_ymd_opt(2026, 1, 1).expect("start"),
                NaiveDate::from_ymd_opt(2026, 8, 1).expect("end"),
                chrono_tz::America::New_York,
            )
            .expect("expand");

        assert_eq!(occurrences.len(), 2);
        let TimeSpec::Instant {
            start_utc: summer, ..
        } = occurrences[1].time
        else {
            panic!("instant occurrence");
        };
        assert_eq!(
            summer
                .with_timezone(&chrono_tz::America::New_York)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
            "2026-07-02 09:00"
        );
        assert_eq!(summer.format("%H:%M").to_string(), "13:00");
    }

    #[test]
    fn yearly_by_week_no_integrates_with_exdate_and_moved_override() {
        let start = NaiveDate::from_ymd_opt(1997, 5, 12).expect("start");
        let excluded = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(1997, 5, 12).expect("week 20 monday"),
            end_exclusive: None,
        };
        let original = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(1997, 5, 14).expect("week 20 wednesday"),
            end_exclusive: None,
        };
        let replacement = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(1997, 5, 16).expect("moved"),
            end_exclusive: None,
        };
        let mut event = TemporalEvent::new(
            "Week-number exceptions",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(4),
            until: None,
            week_start: RecurrenceWeekday::Monday,
            by_weekday: vec![RecurrenceWeekday::Monday, RecurrenceWeekday::Wednesday],
            by_month: Vec::new(),
            by_week_no: vec![20],
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: vec![excluded.clone()],
            overrides: vec![RecurrenceOverride {
                original: original.clone(),
                replacement: Some(replacement.clone()),
                cancelled: false,
            }],
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(1998, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert!(
            occurrences
                .iter()
                .all(|occurrence| occurrence.original_time != excluded)
        );
        let moved = occurrences
            .iter()
            .find(|occurrence| occurrence.original_time == original)
            .expect("moved occurrence");
        assert_eq!(moved.time, replacement);
        assert!(moved.override_applied);
    }

    #[test]
    fn monthly_by_set_pos_selects_from_resolved_candidate_set_before_count() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Last selected month day",
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
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![1, 15, 31],
            by_month_weekday: Vec::new(),
            by_set_pos: vec![-1],
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2026, 4, 1).expect("end"),
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
                NaiveDate::from_ymd_opt(2026, 2, 15).expect("feb"),
                NaiveDate::from_ymd_opt(2026, 3, 31).expect("mar"),
            ]
        );
    }

    #[test]
    fn weekly_by_set_pos_supports_positive_and_negative_positions() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 5).expect("start");
        let mut event = TemporalEvent::new(
            "Week edges",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Weekly,
            interval: 1,
            count: Some(4),
            until: None,
            week_start: RecurrenceWeekday::Monday,
            by_weekday: vec![
                RecurrenceWeekday::Monday,
                RecurrenceWeekday::Wednesday,
                RecurrenceWeekday::Friday,
            ],
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: vec![1, -1],
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2026, 1, 19).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 5).expect("mon1"),
                NaiveDate::from_ymd_opt(2026, 1, 9).expect("fri1"),
                NaiveDate::from_ymd_opt(2026, 1, 12).expect("mon2"),
                NaiveDate::from_ymd_opt(2026, 1, 16).expect("fri2"),
            ]
        );
    }

    #[test]
    fn by_set_pos_deduplicates_alias_positions_and_skips_out_of_range_positions() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Position aliases",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: Some(2),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![1, 15, 31],
            by_month_weekday: Vec::new(),
            by_set_pos: vec![1, -3, -1, 10],
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2026, 2, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 1).expect("first"),
                NaiveDate::from_ymd_opt(2026, 1, 31).expect("last"),
            ]
        );
    }

    #[test]
    fn by_set_pos_validation_requires_selector_and_rejects_invalid_or_duplicate_positions() {
        let mut no_selector = RecurrenceRule::new(RecurrenceFrequency::Monthly);
        no_selector.by_set_pos = vec![-1];
        assert!(matches!(
            no_selector.validate(),
            Err(RecurrenceError::BySetPosRequiresSelector)
        ));

        for invalid_position in [-367, 0, 367] {
            let mut invalid = RecurrenceRule::new(RecurrenceFrequency::Monthly);
            invalid.by_month_day = vec![1, 15];
            invalid.by_set_pos = vec![invalid_position];
            assert!(matches!(
                invalid.validate(),
                Err(RecurrenceError::InvalidBySetPos(value)) if value == invalid_position
            ));
        }

        let mut duplicate = RecurrenceRule::new(RecurrenceFrequency::Monthly);
        duplicate.by_month_day = vec![1, 15];
        duplicate.by_set_pos = vec![-1, -1];
        assert!(matches!(
            duplicate.validate(),
            Err(RecurrenceError::DuplicateBySetPos(-1))
        ));
    }

    #[test]
    fn yearly_by_set_pos_applies_after_year_day_generation() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Last selected year day",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(2),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: vec![1, 100, -1],
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: vec![-1],
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2028, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 12, 31).expect("2026 last"),
                NaiveDate::from_ymd_opt(2027, 12, 31).expect("2027 last"),
            ]
        );
    }

    #[test]
    fn by_set_pos_integrates_with_exdate_and_moved_override_identity() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let excluded = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 1, 31).expect("jan last selected"),
            end_exclusive: None,
        };
        let original = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 2, 15).expect("feb last selected"),
            end_exclusive: None,
        };
        let replacement = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 2, 20).expect("moved"),
            end_exclusive: None,
        };
        let mut event = TemporalEvent::new(
            "Position exceptions",
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
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![1, 15, 31],
            by_month_weekday: Vec::new(),
            by_set_pos: vec![-1],
            rdates: Vec::new(),
            exdates: vec![excluded.clone()],
            overrides: vec![RecurrenceOverride {
                original: original.clone(),
                replacement: Some(replacement.clone()),
                cancelled: false,
            }],
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2026, 4, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert!(
            occurrences
                .iter()
                .all(|occurrence| occurrence.original_time != excluded)
        );
        let moved = occurrences
            .iter()
            .find(|occurrence| occurrence.original_time == original)
            .expect("moved occurrence");
        assert_eq!(moved.time, replacement);
        assert!(moved.override_applied);
    }

    #[test]
    fn yearly_by_year_day_supports_signed_selectors_and_skips_invalid_non_leap_day() {
        let start = NaiveDate::from_ymd_opt(2026, 4, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Signed year days",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(5),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: vec![366, -1, 100, 1],
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2028, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 4, 10).expect("day 100"),
                NaiveDate::from_ymd_opt(2026, 12, 31).expect("last day"),
                NaiveDate::from_ymd_opt(2027, 1, 1).expect("day 1"),
                NaiveDate::from_ymd_opt(2027, 4, 10).expect("day 100"),
                NaiveDate::from_ymd_opt(2027, 12, 31).expect("last day"),
            ]
        );
    }

    #[test]
    fn yearly_by_year_day_interval_tracks_leap_year_ordinals() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Every other year day sixty",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 2,
            count: Some(3),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: vec![60],
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2031, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 3, 1).expect("2026 day 60"),
                NaiveDate::from_ymd_opt(2028, 2, 29).expect("2028 day 60"),
                NaiveDate::from_ymd_opt(2030, 3, 1).expect("2030 day 60"),
            ]
        );
    }

    #[test]
    fn by_year_day_validation_rejects_wrong_frequency_zero_range_and_duplicates() {
        let mut wrong_frequency = RecurrenceRule::new(RecurrenceFrequency::Monthly);
        wrong_frequency.by_year_day = vec![1];
        assert!(matches!(
            wrong_frequency.validate(),
            Err(RecurrenceError::ByYearDayRequiresYearly)
        ));

        for invalid_day in [-367, 0, 367] {
            let mut invalid = RecurrenceRule::new(RecurrenceFrequency::Yearly);
            invalid.by_year_day = vec![invalid_day];
            assert!(matches!(
                invalid.validate(),
                Err(RecurrenceError::InvalidByYearDay(value)) if value == invalid_day
            ));
        }

        let mut duplicate = RecurrenceRule::new(RecurrenceFrequency::Yearly);
        duplicate.by_year_day = vec![-1, -1];
        assert!(matches!(
            duplicate.validate(),
            Err(RecurrenceError::DuplicateByYearDay(-1))
        ));
    }

    #[test]
    fn yearly_by_year_day_intersects_existing_month_scoped_selectors() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Year-day selector intersection",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(3),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: vec![1, 7],
            by_week_no: Vec::new(),
            by_year_day: vec![5, 30, 31, 212],
            by_month_day: vec![5, 30, 31],
            by_month_weekday: vec![
                RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday),
                RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday),
            ],
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2027, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 5).expect("jan first monday"),
                NaiveDate::from_ymd_opt(2026, 1, 30).expect("jan last friday"),
                NaiveDate::from_ymd_opt(2026, 7, 31).expect("jul last friday"),
            ]
        );
    }

    #[test]
    fn exact_yearly_by_year_day_preserves_source_wall_clock_across_dst() {
        let start_utc = DateTime::parse_from_rfc3339("2026-01-01T14:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        let mut event = TemporalEvent::new(
            "Year days at nine",
            TimeSpec::Instant {
                start_utc,
                end_utc: None,
                source_timezone: Some("America/New_York".to_string()),
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(2),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: vec![1, 182],
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                NaiveDate::from_ymd_opt(2026, 1, 1).expect("start"),
                NaiveDate::from_ymd_opt(2026, 8, 1).expect("end"),
                chrono_tz::America::New_York,
            )
            .expect("expand");

        assert_eq!(occurrences.len(), 2);
        let TimeSpec::Instant {
            start_utc: summer, ..
        } = occurrences[1].time
        else {
            panic!("instant occurrence");
        };
        assert_eq!(
            summer
                .with_timezone(&chrono_tz::America::New_York)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
            "2026-07-01 09:00"
        );
        assert_eq!(summer.format("%H:%M").to_string(), "13:00");
    }

    #[test]
    fn yearly_by_year_day_integrates_with_exdate_and_moved_override() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let excluded = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 4, 10).expect("day 100"),
            end_exclusive: None,
        };
        let original = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 12, 31).expect("last day"),
            end_exclusive: None,
        };
        let replacement = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2027, 1, 2).expect("moved"),
            end_exclusive: None,
        };
        let mut event = TemporalEvent::new(
            "Year-day exceptions",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(4),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: vec![100, -1],
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: vec![excluded.clone()],
            overrides: vec![RecurrenceOverride {
                original: original.clone(),
                replacement: Some(replacement.clone()),
                cancelled: false,
            }],
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2027, 2, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert!(
            occurrences
                .iter()
                .all(|occurrence| occurrence.original_time != excluded)
        );
        let moved = occurrences
            .iter()
            .find(|occurrence| occurrence.original_time == original)
            .expect("moved occurrence");
        assert_eq!(moved.time, replacement);
        assert!(moved.override_applied);
    }

    #[test]
    fn yearly_by_month_and_ordinal_byday_expands_month_scoped_weekdays() {
        let start = NaiveDate::from_ymd_opt(2026, 4, 15).expect("start");
        let mut event = TemporalEvent::new(
            "Selected month weekdays",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(5),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: vec![11, 3],
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: vec![
                RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday),
                RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday),
            ],
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2028, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 11, 2).expect("nov first monday"),
                NaiveDate::from_ymd_opt(2026, 11, 27).expect("nov last friday"),
                NaiveDate::from_ymd_opt(2027, 3, 1).expect("mar first monday"),
                NaiveDate::from_ymd_opt(2027, 3, 26).expect("mar last friday"),
                NaiveDate::from_ymd_opt(2027, 11, 1).expect("nov first monday"),
            ]
        );
    }

    #[test]
    fn yearly_ordinal_byday_uses_year_scope_without_bymonth() {
        let mut yearly = RecurrenceRule::new(RecurrenceFrequency::Yearly);
        yearly.by_month_weekday = vec![
            RecurrenceOrdinalWeekday::new(53, RecurrenceWeekday::Friday),
            RecurrenceOrdinalWeekday::new(-53, RecurrenceWeekday::Monday),
        ];
        yearly
            .validate()
            .expect("whole-year ordinal BYDAY supports ±53");

        yearly.by_month = vec![3];
        assert!(matches!(
            yearly.validate(),
            Err(RecurrenceError::InvalidOrdinalByWeekday(53))
        ));
    }

    #[test]
    fn yearly_month_day_and_ordinal_byday_intersect_within_selected_months() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Yearly selector intersection",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(3),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: vec![1, 7],
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![5, 30, 31],
            by_month_weekday: vec![
                RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday),
                RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday),
            ],
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2027, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 5).expect("jan first monday"),
                NaiveDate::from_ymd_opt(2026, 1, 30).expect("jan last friday"),
                NaiveDate::from_ymd_opt(2026, 7, 31).expect("jul last friday"),
            ]
        );
    }

    #[test]
    fn exact_yearly_by_month_and_ordinal_byday_preserves_source_wall_clock() {
        let start_utc = DateTime::parse_from_rfc3339("2026-01-05T14:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        let mut event = TemporalEvent::new(
            "Selected weekday at nine",
            TimeSpec::Instant {
                start_utc,
                end_utc: None,
                source_timezone: Some("America/New_York".to_string()),
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(2),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: vec![1, 7],
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: vec![RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday)],
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                NaiveDate::from_ymd_opt(2026, 1, 5).expect("start"),
                NaiveDate::from_ymd_opt(2026, 8, 1).expect("end"),
                chrono_tz::America::New_York,
            )
            .expect("expand");

        assert_eq!(occurrences.len(), 2);
        let TimeSpec::Instant {
            start_utc: summer, ..
        } = occurrences[1].time
        else {
            panic!("instant occurrence");
        };
        assert_eq!(
            summer
                .with_timezone(&chrono_tz::America::New_York)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
            "2026-07-06 09:00"
        );
        assert_eq!(summer.format("%H:%M").to_string(), "13:00");
    }

    #[test]
    fn yearly_by_month_and_month_day_expands_cartesian_product_in_date_order() {
        let start = NaiveDate::from_ymd_opt(2026, 6, 15).expect("start");
        let mut event = TemporalEvent::new(
            "Selected month days",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(6),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: vec![12, 1, 7],
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![-1, 1],
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2028, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 7, 1).expect("jul1"),
                NaiveDate::from_ymd_opt(2026, 7, 31).expect("jul31"),
                NaiveDate::from_ymd_opt(2026, 12, 1).expect("dec1"),
                NaiveDate::from_ymd_opt(2026, 12, 31).expect("dec31"),
                NaiveDate::from_ymd_opt(2027, 1, 1).expect("jan1"),
                NaiveDate::from_ymd_opt(2027, 1, 31).expect("jan31"),
            ]
        );
    }

    #[test]
    fn yearly_by_month_day_is_valid_without_explicit_by_month() {
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Yearly);
        rule.by_month_day = vec![1, -1];

        rule.validate().expect("yearly BYMONTHDAY");
    }

    #[test]
    fn yearly_by_month_and_month_day_interval_skips_inactive_years() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Alternate-year selected dates",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 2,
            count: Some(4),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: vec![1, 7],
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![4],
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2030, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 4).expect("jan26"),
                NaiveDate::from_ymd_opt(2026, 7, 4).expect("jul26"),
                NaiveDate::from_ymd_opt(2028, 1, 4).expect("jan28"),
                NaiveDate::from_ymd_opt(2028, 7, 4).expect("jul28"),
            ]
        );
    }

    #[test]
    fn exact_yearly_by_month_and_month_day_preserves_source_wall_clock_across_dst() {
        let start_utc = DateTime::parse_from_rfc3339("2026-01-01T14:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        let mut event = TemporalEvent::new(
            "Selected dates at nine",
            TimeSpec::Instant {
                start_utc,
                end_utc: None,
                source_timezone: Some("America/New_York".to_string()),
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(2),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: vec![1, 7],
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![1],
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                NaiveDate::from_ymd_opt(2026, 1, 1).expect("start"),
                NaiveDate::from_ymd_opt(2026, 8, 1).expect("end"),
                chrono_tz::America::New_York,
            )
            .expect("expand");

        assert_eq!(occurrences.len(), 2);
        let TimeSpec::Instant {
            start_utc: summer, ..
        } = occurrences[1].time
        else {
            panic!("instant occurrence");
        };
        assert_eq!(
            summer
                .with_timezone(&chrono_tz::America::New_York)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
            "2026-07-01 09:00"
        );
        assert_eq!(summer.format("%H:%M").to_string(), "13:00");
    }

    #[test]
    fn yearly_by_month_and_month_day_integrates_with_exdate_and_moved_override() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let excluded = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 1, 31).expect("jan31"),
            end_exclusive: None,
        };
        let original = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 7, 31).expect("jul31"),
            end_exclusive: None,
        };
        let replacement = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 8, 2).expect("moved"),
            end_exclusive: None,
        };
        let mut event = TemporalEvent::new(
            "Yearly selected-date exceptions",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(4),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: vec![1, 7],
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![-1],
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: vec![excluded.clone()],
            overrides: vec![RecurrenceOverride {
                original: original.clone(),
                replacement: Some(replacement.clone()),
                cancelled: false,
            }],
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2027, 2, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert!(
            occurrences
                .iter()
                .all(|occurrence| occurrence.original_time != excluded)
        );
        let moved = occurrences
            .iter()
            .find(|occurrence| occurrence.original_time == original)
            .expect("moved occurrence");
        assert_eq!(moved.time, replacement);
        assert!(moved.override_applied);
    }

    #[test]
    fn yearly_by_month_expands_selected_months_in_chronological_order() {
        let start = NaiveDate::from_ymd_opt(2026, 4, 15).expect("start");
        let mut event = TemporalEvent::new(
            "Selected months",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(5),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: vec![10, 1, 4],
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2028, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 4, 15).expect("apr26"),
                NaiveDate::from_ymd_opt(2026, 10, 15).expect("oct26"),
                NaiveDate::from_ymd_opt(2027, 1, 15).expect("jan27"),
                NaiveDate::from_ymd_opt(2027, 4, 15).expect("apr27"),
                NaiveDate::from_ymd_opt(2027, 10, 15).expect("oct27"),
            ]
        );
    }

    #[test]
    fn yearly_by_month_interval_skips_inactive_years() {
        let start = NaiveDate::from_ymd_opt(2026, 2, 10).expect("start");
        let mut event = TemporalEvent::new(
            "Alternate years",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 2,
            count: Some(4),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: vec![2, 8],
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2030, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 2, 10).expect("feb26"),
                NaiveDate::from_ymd_opt(2026, 8, 10).expect("aug26"),
                NaiveDate::from_ymd_opt(2028, 2, 10).expect("feb28"),
                NaiveDate::from_ymd_opt(2028, 8, 10).expect("aug28"),
            ]
        );
    }

    #[test]
    fn by_month_validation_rejects_wrong_frequency_invalid_and_duplicate_months() {
        let mut wrong_frequency = RecurrenceRule::new(RecurrenceFrequency::Monthly);
        wrong_frequency.by_month = vec![6];
        assert!(matches!(
            wrong_frequency.validate(),
            Err(RecurrenceError::ByMonthRequiresYearly)
        ));

        let mut invalid = RecurrenceRule::new(RecurrenceFrequency::Yearly);
        invalid.by_month = vec![0];
        assert!(matches!(
            invalid.validate(),
            Err(RecurrenceError::InvalidByMonth(0))
        ));

        let mut duplicate = RecurrenceRule::new(RecurrenceFrequency::Yearly);
        duplicate.by_month = vec![6, 6];
        assert!(matches!(
            duplicate.validate(),
            Err(RecurrenceError::DuplicateByMonth(6))
        ));
    }

    #[test]
    fn exact_yearly_by_month_preserves_source_wall_clock_across_dst() {
        let start_utc = DateTime::parse_from_rfc3339("2026-01-15T14:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        let mut event = TemporalEvent::new(
            "Winter and summer at nine",
            TimeSpec::Instant {
                start_utc,
                end_utc: None,
                source_timezone: Some("America/New_York".to_string()),
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(2),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: vec![1, 7],
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                NaiveDate::from_ymd_opt(2026, 1, 15).expect("start"),
                NaiveDate::from_ymd_opt(2026, 8, 1).expect("end"),
                chrono_tz::America::New_York,
            )
            .expect("expand");

        assert_eq!(occurrences.len(), 2);
        let TimeSpec::Instant {
            start_utc: summer, ..
        } = occurrences[1].time
        else {
            panic!("instant occurrence");
        };
        assert_eq!(
            summer
                .with_timezone(&chrono_tz::America::New_York)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
            "2026-07-15 09:00"
        );
        assert_eq!(summer.format("%H:%M").to_string(), "13:00");
    }

    #[test]
    fn yearly_by_month_integrates_with_exdate_and_moved_override() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 10).expect("start");
        let excluded = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 6, 10).expect("jun"),
            end_exclusive: None,
        };
        let original = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 12, 10).expect("dec"),
            end_exclusive: None,
        };
        let replacement = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 12, 12).expect("moved"),
            end_exclusive: None,
        };
        let mut event = TemporalEvent::new(
            "Yearly exceptions",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(4),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: vec![1, 6, 12],
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: vec![excluded],
            overrides: vec![RecurrenceOverride {
                original: original.clone(),
                replacement: Some(replacement.clone()),
                cancelled: false,
            }],
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2027, 2, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 10).expect("jan26"),
                NaiveDate::from_ymd_opt(2026, 12, 12).expect("moved"),
                NaiveDate::from_ymd_opt(2027, 1, 10).expect("jan27"),
            ]
        );
        let moved = occurrences
            .iter()
            .find(|occurrence| occurrence.original_time == original)
            .expect("moved occurrence");
        assert_eq!(moved.time, replacement);
        assert!(moved.override_applied);
    }

    #[test]
    fn monthly_ordinal_byday_expands_first_and_last_weekdays_in_date_order() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 10).expect("start");
        let mut event = TemporalEvent::new(
            "Ordinal weekdays",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: Some(4),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: vec![
                RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday),
                RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday),
            ],
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2026, 4, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 30).expect("jan last friday"),
                NaiveDate::from_ymd_opt(2026, 2, 2).expect("feb first monday"),
                NaiveDate::from_ymd_opt(2026, 2, 27).expect("feb last friday"),
                NaiveDate::from_ymd_opt(2026, 3, 2).expect("mar first monday"),
            ]
        );
    }

    #[test]
    fn monthly_ordinal_byday_skips_missing_fifth_weekday() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Fifth Monday",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: Some(2),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: vec![RecurrenceOrdinalWeekday::new(5, RecurrenceWeekday::Monday)],
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2026, 7, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 3, 30).expect("march fifth monday"),
                NaiveDate::from_ymd_opt(2026, 6, 29).expect("june fifth monday"),
            ]
        );
    }

    #[test]
    fn monthly_ordinal_byday_validation_is_bounded() {
        let mut wrong_frequency = RecurrenceRule::new(RecurrenceFrequency::Weekly);
        wrong_frequency.by_month_weekday =
            vec![RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday)];
        assert!(matches!(
            wrong_frequency.validate(),
            Err(RecurrenceError::OrdinalByWeekdayRequiresMonthlyOrYearly)
        ));

        for invalid_ordinal in [-6, 0, 6] {
            let mut invalid = RecurrenceRule::new(RecurrenceFrequency::Monthly);
            invalid.by_month_weekday = vec![RecurrenceOrdinalWeekday::new(
                invalid_ordinal,
                RecurrenceWeekday::Monday,
            )];
            assert!(matches!(
                invalid.validate(),
                Err(RecurrenceError::InvalidOrdinalByWeekday(value))
                    if value == invalid_ordinal
            ));
        }

        let mut duplicate = RecurrenceRule::new(RecurrenceFrequency::Monthly);
        duplicate.by_month_weekday = vec![
            RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday),
            RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday),
        ];
        assert!(matches!(
            duplicate.validate(),
            Err(RecurrenceError::DuplicateOrdinalByWeekday(-1, "friday"))
        ));
    }

    #[test]
    fn monthly_selector_combination_intersects_resolved_civil_dates() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Monthly selector intersection",
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
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![5, 30, 31],
            by_month_weekday: vec![
                RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday),
                RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday),
            ],
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2027, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 5).expect("jan first monday"),
                NaiveDate::from_ymd_opt(2026, 1, 30).expect("jan last friday"),
                NaiveDate::from_ymd_opt(2026, 7, 31).expect("jul last friday"),
            ]
        );
    }

    #[test]
    fn monthly_selector_intersection_preserves_override_target_validation() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let target = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 1, 5).expect("intersection target"),
            end_exclusive: None,
        };
        let mut event = TemporalEvent::new(
            "Intersection override",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: Some(1),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![5, 15],
            by_month_weekday: vec![RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday)],
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: vec![RecurrenceOverride {
                original: target.clone(),
                replacement: Some(TimeSpec::DateOnly {
                    start: NaiveDate::from_ymd_opt(2026, 1, 6).expect("moved"),
                    end_exclusive: None,
                }),
                cancelled: false,
            }],
        });

        event
            .validate_recurrence()
            .expect("valid intersection target");
        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2026, 2, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");
        assert_eq!(occurrences.len(), 1);
        assert_eq!(occurrences[0].original_time, target);
        assert!(occurrences[0].override_applied);
    }

    #[test]
    fn exact_monthly_ordinal_byday_preserves_source_wall_clock_across_dst() {
        let start_utc = DateTime::parse_from_rfc3339("2026-02-01T14:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        let mut event = TemporalEvent::new(
            "First Sunday at nine",
            TimeSpec::Instant {
                start_utc,
                end_utc: None,
                source_timezone: Some("America/New_York".to_string()),
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: Some(3),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: vec![RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Sunday)],
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                NaiveDate::from_ymd_opt(2026, 2, 1).expect("start"),
                NaiveDate::from_ymd_opt(2026, 5, 1).expect("end"),
                chrono_tz::America::New_York,
            )
            .expect("expand");

        assert_eq!(occurrences.len(), 3);
        let TimeSpec::Instant {
            start_utc: april, ..
        } = occurrences[2].time
        else {
            panic!("instant occurrence");
        };
        assert_eq!(
            april
                .with_timezone(&chrono_tz::America::New_York)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
            "2026-04-05 09:00"
        );
        assert_eq!(april.format("%H:%M").to_string(), "13:00");
    }

    #[test]
    fn monthly_ordinal_byday_integrates_with_exdate_and_moved_override() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let excluded = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 1, 30).expect("jan last friday"),
            end_exclusive: None,
        };
        let original = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 2, 27).expect("feb last friday"),
            end_exclusive: None,
        };
        let replacement = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 3, 1).expect("moved"),
            end_exclusive: None,
        };
        let mut event = TemporalEvent::new(
            "Last Friday exceptions",
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
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: vec![RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday)],
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: vec![excluded.clone()],
            overrides: vec![RecurrenceOverride {
                original: original.clone(),
                replacement: Some(replacement.clone()),
                cancelled: false,
            }],
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2026, 4, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert!(
            occurrences
                .iter()
                .all(|occurrence| occurrence.original_time != excluded)
        );
        let moved = occurrences
            .iter()
            .find(|occurrence| occurrence.original_time == original)
            .expect("moved occurrence");
        assert_eq!(moved.time, replacement);
        assert!(moved.override_applied);
    }

    #[test]
    fn monthly_by_month_day_supports_negative_days_from_month_end() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 20).expect("start");
        let mut event = TemporalEvent::new(
            "Month end selectors",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: Some(5),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![-1, 1, -2],
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2026, 3, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 30).expect("jan30"),
                NaiveDate::from_ymd_opt(2026, 1, 31).expect("jan31"),
                NaiveDate::from_ymd_opt(2026, 2, 1).expect("feb1"),
                NaiveDate::from_ymd_opt(2026, 2, 27).expect("feb27"),
                NaiveDate::from_ymd_opt(2026, 2, 28).expect("feb28"),
            ]
        );
    }

    #[test]
    fn monthly_by_month_day_deduplicates_aliases_that_resolve_to_same_date() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Alias selectors",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: Some(2),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![1, -31],
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2026, 3, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 1).expect("jan1"),
                NaiveDate::from_ymd_opt(2026, 2, 1).expect("feb1"),
            ]
        );
    }

    #[test]
    fn by_month_day_validation_accepts_negative_range_and_rejects_zero_or_out_of_range() {
        let mut valid = RecurrenceRule::new(RecurrenceFrequency::Monthly);
        valid.by_month_day = vec![-31, -1, 1, 31];
        valid.validate().expect("signed BYMONTHDAY range");

        for invalid_day in [-32, 0, 32] {
            let mut invalid = RecurrenceRule::new(RecurrenceFrequency::Monthly);
            invalid.by_month_day = vec![invalid_day];
            assert!(matches!(
                invalid.validate(),
                Err(RecurrenceError::InvalidByMonthDay(day)) if day == invalid_day
            ));
        }
    }

    #[test]
    fn negative_by_month_day_integrates_with_exdate_and_override_identity() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let excluded = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 1, 31).expect("jan31"),
            end_exclusive: None,
        };
        let original = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 2, 28).expect("feb28"),
            end_exclusive: None,
        };
        let replacement = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 3, 2).expect("moved"),
            end_exclusive: None,
        };
        let mut event = TemporalEvent::new(
            "Last day exceptions",
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
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![-1],
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: vec![excluded.clone()],
            overrides: vec![RecurrenceOverride {
                original: original.clone(),
                replacement: Some(replacement.clone()),
                cancelled: false,
            }],
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2026, 4, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");
        let moved = occurrences
            .iter()
            .find(|occurrence| occurrence.original_time == original)
            .expect("moved last-day occurrence");

        assert_eq!(moved.time, replacement);
        assert!(moved.override_applied);
        assert!(
            occurrences
                .iter()
                .all(|occurrence| occurrence.original_time != excluded)
        );
    }

    #[test]
    fn monthly_by_month_day_expands_multiple_days_and_skips_invalid_dates() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 20).expect("start");
        let mut event = TemporalEvent::new(
            "Month days",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: Some(5),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![31, 1, 15],
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2026, 4, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 31).expect("jan31"),
                NaiveDate::from_ymd_opt(2026, 2, 1).expect("feb1"),
                NaiveDate::from_ymd_opt(2026, 2, 15).expect("feb15"),
                NaiveDate::from_ymd_opt(2026, 3, 1).expect("mar1"),
                NaiveDate::from_ymd_opt(2026, 3, 15).expect("mar15"),
            ]
        );
    }

    #[test]
    fn monthly_by_month_day_interval_skips_inactive_months() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Every other month",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 2,
            count: Some(4),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![1, 15],
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
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
                NaiveDate::from_ymd_opt(2026, 1, 1).expect("jan1"),
                NaiveDate::from_ymd_opt(2026, 1, 15).expect("jan15"),
                NaiveDate::from_ymd_opt(2026, 3, 1).expect("mar1"),
                NaiveDate::from_ymd_opt(2026, 3, 15).expect("mar15"),
            ]
        );
    }

    #[test]
    fn by_month_day_validation_rejects_wrong_frequency_invalid_and_duplicate_days() {
        let mut wrong_frequency = RecurrenceRule::new(RecurrenceFrequency::Weekly);
        wrong_frequency.by_month_day = vec![15];
        assert!(matches!(
            wrong_frequency.validate(),
            Err(RecurrenceError::ByMonthDayRequiresMonthlyOrYearly)
        ));

        let mut invalid = RecurrenceRule::new(RecurrenceFrequency::Monthly);
        invalid.by_month_day = vec![0];
        assert!(matches!(
            invalid.validate(),
            Err(RecurrenceError::InvalidByMonthDay(0))
        ));

        let mut duplicate = RecurrenceRule::new(RecurrenceFrequency::Monthly);
        duplicate.by_month_day = vec![15, 15];
        assert!(matches!(
            duplicate.validate(),
            Err(RecurrenceError::DuplicateByMonthDay(15))
        ));
    }

    #[test]
    fn exact_monthly_by_month_day_preserves_source_wall_clock_across_dst() {
        let start_utc = DateTime::parse_from_rfc3339("2026-02-15T14:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        let mut event = TemporalEvent::new(
            "First and fifteenth at nine",
            TimeSpec::Instant {
                start_utc,
                end_utc: None,
                source_timezone: Some("America/New_York".to_string()),
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: Some(3),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![1, 15],
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                NaiveDate::from_ymd_opt(2026, 2, 15).expect("start"),
                NaiveDate::from_ymd_opt(2026, 3, 20).expect("end"),
                chrono_tz::America::New_York,
            )
            .expect("expand");

        assert_eq!(occurrences.len(), 3);
        let TimeSpec::Instant {
            start_utc: third, ..
        } = occurrences[2].time
        else {
            panic!("instant occurrence");
        };
        assert_eq!(
            third
                .with_timezone(&chrono_tz::America::New_York)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
            "2026-03-15 09:00"
        );
        assert_eq!(third.format("%H:%M").to_string(), "13:00");
    }

    #[test]
    fn monthly_by_month_day_integrates_with_exdate_and_moved_override() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let excluded = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 1, 15).expect("jan15"),
            end_exclusive: None,
        };
        let original = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 2, 1).expect("feb1"),
            end_exclusive: None,
        };
        let replacement = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 2, 3).expect("feb3"),
            end_exclusive: None,
        };
        let mut event = TemporalEvent::new(
            "Monthly exceptions",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Monthly,
            interval: 1,
            count: Some(4),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![1, 15],
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: vec![excluded],
            overrides: vec![RecurrenceOverride {
                original: original.clone(),
                replacement: Some(replacement.clone()),
                cancelled: false,
            }],
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2026, 3, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert_eq!(
            occurrences
                .iter()
                .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
                .collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 1).expect("jan1"),
                NaiveDate::from_ymd_opt(2026, 2, 3).expect("moved"),
                NaiveDate::from_ymd_opt(2026, 2, 15).expect("feb15"),
            ]
        );
        let moved = occurrences
            .iter()
            .find(|occurrence| occurrence.original_time == original)
            .expect("moved occurrence");
        assert_eq!(moved.time, replacement);
        assert!(moved.override_applied);
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
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
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
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
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
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
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

    #[test]
    fn yearly_plain_byday_is_valid_without_byweekno() {
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Yearly);
        rule.by_weekday = vec![RecurrenceWeekday::Monday];

        assert_eq!(rule.validate(), Ok(()));
    }

    #[test]
    fn yearly_plain_byday_expands_across_years_from_dtstart() {
        let start = NaiveDate::from_ymd_opt(2026, 12, 28).expect("start");
        let mut event = TemporalEvent::new(
            "Yearly Mondays",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(3),
            until: None,
            week_start: Default::default(),
            by_weekday: vec![RecurrenceWeekday::Monday],
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                NaiveDate::from_ymd_opt(2026, 12, 1).expect("window start"),
                NaiveDate::from_ymd_opt(2027, 1, 20).expect("window end"),
                chrono_tz::UTC,
            )
            .expect("occurrences");
        let dates = occurrences
            .iter()
            .map(|occurrence| recurrence_rule_date(&occurrence.time).expect("date"))
            .collect::<Vec<_>>();

        assert_eq!(
            dates,
            vec![
                NaiveDate::from_ymd_opt(2026, 12, 28).expect("date"),
                NaiveDate::from_ymd_opt(2027, 1, 4).expect("date"),
                NaiveDate::from_ymd_opt(2027, 1, 11).expect("date"),
            ]
        );
    }

    #[test]
    fn yearly_plain_byday_respects_bymonth() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut event = TemporalEvent::new(
            "March Mondays",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(5),
            until: None,
            week_start: Default::default(),
            by_weekday: vec![RecurrenceWeekday::Monday],
            by_month: vec![3],
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                NaiveDate::from_ymd_opt(2026, 1, 1).expect("window start"),
                NaiveDate::from_ymd_opt(2026, 4, 1).expect("window end"),
                chrono_tz::UTC,
            )
            .expect("occurrences");
        let dates = occurrences
            .iter()
            .map(|occurrence| recurrence_rule_date(&occurrence.time).expect("date"))
            .collect::<Vec<_>>();

        assert_eq!(
            dates,
            vec![
                NaiveDate::from_ymd_opt(2026, 3, 2).expect("date"),
                NaiveDate::from_ymd_opt(2026, 3, 9).expect("date"),
                NaiveDate::from_ymd_opt(2026, 3, 16).expect("date"),
                NaiveDate::from_ymd_opt(2026, 3, 23).expect("date"),
                NaiveDate::from_ymd_opt(2026, 3, 30).expect("date"),
            ]
        );
    }

    #[test]
    fn yearly_plain_byday_filters_byyearday() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Early-year Monday",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(2),
            until: None,
            week_start: Default::default(),
            by_weekday: vec![RecurrenceWeekday::Monday],
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: vec![1, 2, 3, 4, 5],
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                NaiveDate::from_ymd_opt(2026, 1, 1).expect("window start"),
                NaiveDate::from_ymd_opt(2027, 1, 10).expect("window end"),
                chrono_tz::UTC,
            )
            .expect("occurrences");
        let dates = occurrences
            .iter()
            .map(|occurrence| recurrence_rule_date(&occurrence.time).expect("date"))
            .collect::<Vec<_>>();

        assert_eq!(
            dates,
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 5).expect("date"),
                NaiveDate::from_ymd_opt(2027, 1, 4).expect("date"),
            ]
        );
    }

    #[test]
    fn yearly_plain_byday_with_bysetpos_selects_last_weekday_of_month() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Last January weekday",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(2),
            until: None,
            week_start: Default::default(),
            by_weekday: vec![
                RecurrenceWeekday::Monday,
                RecurrenceWeekday::Tuesday,
                RecurrenceWeekday::Wednesday,
                RecurrenceWeekday::Thursday,
                RecurrenceWeekday::Friday,
            ],
            by_month: vec![1],
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: Vec::new(),
            by_set_pos: vec![-1],
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                NaiveDate::from_ymd_opt(2026, 1, 1).expect("window start"),
                NaiveDate::from_ymd_opt(2027, 2, 1).expect("window end"),
                chrono_tz::UTC,
            )
            .expect("occurrences");
        let dates = occurrences
            .iter()
            .map(|occurrence| recurrence_rule_date(&occurrence.time).expect("date"))
            .collect::<Vec<_>>();

        assert_eq!(
            dates,
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 30).expect("date"),
                NaiveDate::from_ymd_opt(2027, 1, 29).expect("date"),
            ]
        );
    }

    #[test]
    fn yearly_byyearday_unions_plain_and_ordinal_byday() {
        let start = NaiveDate::from_ymd_opt(2026, 3, 1).expect("start");
        let mut event = TemporalEvent::new(
            "March BYDAY union",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(2),
            until: None,
            week_start: Default::default(),
            by_weekday: vec![RecurrenceWeekday::Monday],
            by_month: vec![3],
            by_week_no: Vec::new(),
            by_year_day: vec![61, 62, 63, 64, 65, 66, 67],
            by_month_day: Vec::new(),
            by_month_weekday: vec![RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Tuesday)],
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                NaiveDate::from_ymd_opt(2026, 3, 1).expect("window start"),
                NaiveDate::from_ymd_opt(2026, 3, 10).expect("window end"),
                chrono_tz::UTC,
            )
            .expect("occurrences");
        let dates = occurrences
            .iter()
            .map(|occurrence| recurrence_rule_date(&occurrence.time).expect("date"))
            .collect::<Vec<_>>();

        assert_eq!(
            dates,
            vec![
                NaiveDate::from_ymd_opt(2026, 3, 2).expect("date"),
                NaiveDate::from_ymd_opt(2026, 3, 3).expect("date"),
            ]
        );
    }


    #[test]
    fn yearly_ordinal_byday_expands_first_and_last_weekdays_of_year() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Year ordinal weekdays",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(4),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: vec![
                RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday),
                RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday),
            ],
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let dates = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2028, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand")
            .iter()
            .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
            .collect::<Vec<_>>();

        assert_eq!(
            dates,
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 5).expect("first monday"),
                NaiveDate::from_ymd_opt(2026, 12, 25).expect("last friday"),
                NaiveDate::from_ymd_opt(2027, 1, 4).expect("first monday"),
                NaiveDate::from_ymd_opt(2027, 12, 31).expect("last friday"),
            ]
        );
    }

    #[test]
    fn yearly_ordinal_byday_skips_missing_fifty_third_weekday() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Fifty-third Monday",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(1),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: vec![RecurrenceOrdinalWeekday::new(
                53,
                RecurrenceWeekday::Monday,
            )],
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let dates = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2030, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand")
            .iter()
            .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
            .collect::<Vec<_>>();

        assert_eq!(dates, vec![NaiveDate::from_ymd_opt(2029, 12, 31).expect("53rd Monday")]);
    }

    #[test]
    fn yearly_plain_and_ordinal_byday_union_at_year_scope() {
        let start = NaiveDate::from_ymd_opt(2026, 12, 20).expect("start");
        let mut event = TemporalEvent::new(
            "Year BYDAY union",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(3),
            until: None,
            week_start: Default::default(),
            by_weekday: vec![RecurrenceWeekday::Monday],
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: vec![RecurrenceOrdinalWeekday::new(
                -1,
                RecurrenceWeekday::Friday,
            )],
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let dates = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2027, 1, 15).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand")
            .iter()
            .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
            .collect::<Vec<_>>();

        assert_eq!(
            dates,
            vec![
                NaiveDate::from_ymd_opt(2026, 12, 21).expect("monday"),
                NaiveDate::from_ymd_opt(2026, 12, 25).expect("last friday"),
                NaiveDate::from_ymd_opt(2026, 12, 28).expect("monday"),
            ]
        );
    }

    #[test]
    fn yearly_ordinal_byday_filters_byyearday() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Year ordinal filtered",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(2),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: vec![1, 2, 3, 4, 5, 6, 7],
            by_month_day: Vec::new(),
            by_month_weekday: vec![RecurrenceOrdinalWeekday::new(
                1,
                RecurrenceWeekday::Monday,
            )],
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let dates = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2027, 1, 10).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand")
            .iter()
            .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
            .collect::<Vec<_>>();

        assert_eq!(
            dates,
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 5).expect("date"),
                NaiveDate::from_ymd_opt(2027, 1, 4).expect("date"),
            ]
        );
    }


    #[test]
    fn exact_yearly_ordinal_byday_preserves_source_wall_clock_across_dst() {
        let start_utc = DateTime::parse_from_rfc3339("2026-01-01T14:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        let mut event = TemporalEvent::new(
            "Tenth Sunday",
            TimeSpec::Instant {
                start_utc,
                end_utc: None,
                source_timezone: Some("America/New_York".to_string()),
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(2),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: vec![RecurrenceOrdinalWeekday::new(
                10,
                RecurrenceWeekday::Sunday,
            )],
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let occurrences = event
            .occurrences_in_window(
                NaiveDate::from_ymd_opt(2026, 1, 1).expect("start"),
                NaiveDate::from_ymd_opt(2028, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        let source_zone = chrono_tz::America::New_York;
        let local = occurrences
            .iter()
            .map(|occurrence| match &occurrence.time {
                TimeSpec::Instant { start_utc, .. } => start_utc.with_timezone(&source_zone),
                other => panic!("expected instant, got {other:?}"),
            })
            .collect::<Vec<_>>();

        assert_eq!(local[0].date_naive(), NaiveDate::from_ymd_opt(2026, 3, 8).unwrap());
        assert_eq!(local[1].date_naive(), NaiveDate::from_ymd_opt(2027, 3, 7).unwrap());
        assert_eq!(local[0].format("%H:%M").to_string(), "09:00");
        assert_eq!(local[1].format("%H:%M").to_string(), "09:00");
        assert_ne!(local[0].offset().to_string(), local[1].offset().to_string());
    }

    #[test]
    fn yearly_ordinal_byday_integrates_with_exdate_and_moved_override() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let excluded = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 1, 5).expect("first monday"),
            end_exclusive: None,
        };
        let original = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 12, 25).expect("last friday"),
            end_exclusive: None,
        };
        let replacement = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 12, 26).expect("replacement"),
            end_exclusive: None,
        };
        let mut event = TemporalEvent::new(
            "Year ordinal exceptions",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(4),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month_weekday: vec![
                RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday),
                RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday),
            ],
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: vec![excluded.clone()],
            overrides: vec![RecurrenceOverride {
                original: original.clone(),
                replacement: Some(replacement.clone()),
                cancelled: false,
            }],
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2028, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert!(
            occurrences
                .iter()
                .all(|occurrence| occurrence.original_time != excluded)
        );
        let moved = occurrences
            .iter()
            .find(|occurrence| occurrence.original_time == original)
            .expect("moved occurrence");
        assert_eq!(moved.time, replacement);
        assert!(moved.override_applied);
        assert_eq!(
            moved.id,
            occurrence_identity(event.id, &original).expect("stable identity")
        );
    }

    #[test]
    fn yearly_by_month_day_without_bymonth_expands_across_all_months() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 20).expect("start");
        let mut event = TemporalEvent::new(
            "First and last day",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(5),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![1, -1],
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let dates = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2026, 4, 2).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand")
            .iter()
            .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
            .collect::<Vec<_>>();

        assert_eq!(
            dates,
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 31).expect("date"),
                NaiveDate::from_ymd_opt(2026, 2, 1).expect("date"),
                NaiveDate::from_ymd_opt(2026, 2, 28).expect("date"),
                NaiveDate::from_ymd_opt(2026, 3, 1).expect("date"),
                NaiveDate::from_ymd_opt(2026, 3, 31).expect("date"),
            ]
        );
    }

    #[test]
    fn yearly_bymonthday_intersects_whole_year_ordinal_byday() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut event = TemporalEvent::new(
            "Ordinal intersection",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(2),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![5, 25],
            by_month_weekday: vec![
                RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday),
                RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday),
            ],
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let dates = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2027, 1, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand")
            .iter()
            .filter_map(|occurrence| occurrence.time.display_date(chrono_tz::UTC))
            .collect::<Vec<_>>();

        assert_eq!(
            dates,
            vec![
                NaiveDate::from_ymd_opt(2026, 1, 5).expect("first monday"),
                NaiveDate::from_ymd_opt(2026, 12, 25).expect("last friday"),
            ]
        );
    }

    #[test]
    fn exact_yearly_bymonthday_without_bymonth_preserves_source_wall_clock() {
        let start_utc = DateTime::parse_from_rfc3339("2026-01-15T14:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        let mut event = TemporalEvent::new(
            "Monthly fifteenth",
            TimeSpec::Instant {
                start_utc,
                end_utc: None,
                source_timezone: Some("America/New_York".to_string()),
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(3),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![15],
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            overrides: Vec::new(),
        });

        let source_zone = chrono_tz::America::New_York;
        let local = event
            .occurrences_in_window(
                NaiveDate::from_ymd_opt(2026, 1, 1).expect("start"),
                NaiveDate::from_ymd_opt(2026, 4, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand")
            .iter()
            .map(|occurrence| match &occurrence.time {
                TimeSpec::Instant { start_utc, .. } => start_utc.with_timezone(&source_zone),
                other => panic!("expected instant, got {other:?}"),
            })
            .collect::<Vec<_>>();

        assert_eq!(local.len(), 3);
        assert!(local.iter().all(|date| date.format("%H:%M").to_string() == "09:00"));
        assert_eq!(local[1].with_timezone(&Utc).hour(), 14);
        assert_eq!(local[2].with_timezone(&Utc).hour(), 13);
    }

    #[test]
    fn yearly_bymonthday_without_bymonth_integrates_with_exdate_and_override() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let excluded = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 2, 1).expect("excluded"),
            end_exclusive: None,
        };
        let original = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 2, 28).expect("original"),
            end_exclusive: None,
        };
        let replacement = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 3, 2).expect("replacement"),
            end_exclusive: None,
        };
        let mut event = TemporalEvent::new(
            "Month-day exceptions",
            TimeSpec::DateOnly {
                start,
                end_exclusive: None,
            },
        );
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Yearly,
            interval: 1,
            count: Some(6),
            until: None,
            week_start: Default::default(),
            by_weekday: Vec::new(),
            by_month: Vec::new(),
            by_week_no: Vec::new(),
            by_year_day: Vec::new(),
            by_month_day: vec![1, -1],
            by_month_weekday: Vec::new(),
            by_set_pos: Vec::new(),
            rdates: Vec::new(),
            exdates: vec![excluded.clone()],
            overrides: vec![RecurrenceOverride {
                original: original.clone(),
                replacement: Some(replacement.clone()),
                cancelled: false,
            }],
        });

        let occurrences = event
            .occurrences_in_window(
                start,
                NaiveDate::from_ymd_opt(2026, 4, 1).expect("end"),
                chrono_tz::UTC,
            )
            .expect("expand");

        assert!(
            occurrences
                .iter()
                .all(|occurrence| occurrence.original_time != excluded)
        );
        let moved = occurrences
            .iter()
            .find(|occurrence| occurrence.original_time == original)
            .expect("moved");
        assert_eq!(moved.time, replacement);
        assert!(moved.override_applied);
        assert_eq!(
            moved.id,
            occurrence_identity(event.id, &original).expect("stable identity")
        );
    }

}
