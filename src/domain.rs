use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
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
}

impl EventStatus {
    pub const ALL: [Self; 13] = [
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
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.as_str() == raw)
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
        match raw {
            "official" => Self::Official,
            "first_party" => Self::FirstParty,
            "secondary" => Self::Secondary,
            "aggregator" => Self::Aggregator,
            "manual" => Self::Manual,
            "derived" => Self::Derived,
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
        match raw {
            "taria" => Self::Taria,
            "ics" => Self::Ics,
            "webcal" => Self::Webcal,
            "api" => Self::Api,
            "caldav" => Self::CalDav,
            "json" => Self::Json,
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
    AllDay {
        start: NaiveDate,
        end_exclusive: Option<NaiveDate>,
    },
    Instant {
        start_utc: DateTime<Utc>,
        end_utc: Option<DateTime<Utc>>,
        source_timezone: Option<String>,
    },
    Floating {
        start: NaiveDateTime,
        end: Option<NaiveDateTime>,
    },
}

impl TimeSpec {
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::AllDay { .. } => "all_day",
            Self::Instant { .. } => "instant",
            Self::Floating { .. } => "floating",
        }
    }

    pub fn display_date(&self, timezone: Tz) -> NaiveDate {
        match self {
            Self::AllDay { start, .. } => *start,
            Self::Instant { start_utc, .. } => start_utc.with_timezone(&timezone).date_naive(),
            Self::Floating { start, .. } => start.date(),
        }
    }

    pub fn occurs_on(&self, day: NaiveDate, timezone: Tz) -> bool {
        match self {
            Self::AllDay {
                start,
                end_exclusive,
            } => {
                let end = end_exclusive.unwrap_or_else(|| start.succ_opt().unwrap_or(*start));
                day >= *start && day < end
            }
            Self::Instant { start_utc, .. } => {
                start_utc.with_timezone(&timezone).date_naive() == day
            }
            Self::Floating { start, .. } => start.date() == day,
        }
    }

    pub fn display_time_label(&self, timezone: Tz) -> String {
        match self {
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
            Self::Floating { start, .. } => format!("{} · floating", start.format("%H:%M")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemporalSource {
    pub id: Uuid,
    pub name: String,
    pub publisher: Option<String>,
    pub authority: SourceAuthority,
    pub kind: SourceKind,
    pub locator: Option<String>,
    pub enabled: bool,
    pub read_only: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TemporalSource {
    pub fn new(name: impl Into<String>, kind: SourceKind, authority: SourceAuthority) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            publisher: None,
            authority,
            kind,
            locator: None,
            enabled: true,
            read_only: true,
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

    pub fn display_date(&self, timezone: Tz) -> NaiveDate {
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

        assert_eq!(time.display_date(chrono_tz::UTC), start);
        assert_eq!(time.display_date(chrono_tz::Pacific::Honolulu), start);
        assert_eq!(time.display_date(chrono_tz::Asia::Tokyo), start);
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
}
