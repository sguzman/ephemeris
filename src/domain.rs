use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
            },
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
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

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
