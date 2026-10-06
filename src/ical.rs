use std::{collections::HashSet, fmt};

use chrono::NaiveDate;

use crate::domain::{
    RecurrenceError, RecurrenceFrequency, RecurrenceOrdinalWeekday, RecurrenceRule,
    RecurrenceWeekday,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IcalRecurrenceError {
    EmptyRule,
    MissingFrequency,
    DuplicatePart(String),
    UnknownPart(String),
    InvalidPart {
        name: String,
        value: String,
    },
    CountUntilConflict,
    UnsupportedUntilDateTime(String),
    Domain(RecurrenceError),
}

impl fmt::Display for IcalRecurrenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyRule => formatter.write_str("RRULE must not be empty"),
            Self::MissingFrequency => formatter.write_str("RRULE requires exactly one FREQ part"),
            Self::DuplicatePart(name) => write!(formatter, "duplicate RRULE part {name}"),
            Self::UnknownPart(name) => write!(formatter, "unsupported RRULE part {name}"),
            Self::InvalidPart { name, value } => {
                write!(formatter, "invalid RRULE {name} value {value}")
            }
            Self::CountUntilConflict => {
                formatter.write_str("RRULE COUNT and UNTIL must not both be present")
            }
            Self::UnsupportedUntilDateTime(value) => write!(
                formatter,
                "RRULE UNTIL date-time {value} cannot be represented by Ephemeris' civil-date recurrence bound"
            ),
            Self::Domain(error) => write!(formatter, "invalid RRULE recurrence definition: {error}"),
        }
    }
}

impl std::error::Error for IcalRecurrenceError {}

impl From<RecurrenceError> for IcalRecurrenceError {
    fn from(value: RecurrenceError) -> Self {
        Self::Domain(value)
    }
}

/// Parse an RFC 5545 RRULE value into Ephemeris' canonical recurrence rule.
///
/// The input may be a bare RECUR value or begin with `RRULE:`. RDATE, EXDATE,
/// and RECURRENCE-ID overrides are separate iCalendar properties and are not
/// represented by this codec. UNTIL is accepted only in DATE form because the
/// canonical Ephemeris bound is currently a civil date rather than a date-time.
pub fn parse_rrule(raw: &str) -> Result<RecurrenceRule, IcalRecurrenceError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(IcalRecurrenceError::EmptyRule);
    }
    let value = raw
        .get(..6)
        .filter(|prefix| prefix.eq_ignore_ascii_case("RRULE:"))
        .map_or(raw, |_| &raw[6..]);

    let mut seen = HashSet::new();
    let mut frequency = None;
    let mut interval = 1_u32;
    let mut count = None;
    let mut until = None;
    let mut week_start = RecurrenceWeekday::Monday;
    let mut by_weekday = Vec::new();
    let mut by_month = Vec::new();
    let mut by_week_no = Vec::new();
    let mut by_year_day = Vec::new();
    let mut by_month_day = Vec::new();
    let mut by_month_weekday = Vec::new();
    let mut by_hour = Vec::new();
    let mut by_minute = Vec::new();
    let mut by_second = Vec::new();
    let mut by_set_pos = Vec::new();

    for part in value.split(';') {
        let (name, raw_value) = part
            .split_once('=')
            .ok_or_else(|| IcalRecurrenceError::InvalidPart {
                name: part.trim().to_ascii_uppercase(),
                value: String::new(),
            })?;
        let name = name.trim().to_ascii_uppercase();
        let raw_value = raw_value.trim();
        if name.is_empty() || raw_value.is_empty() {
            return Err(IcalRecurrenceError::InvalidPart {
                name,
                value: raw_value.to_string(),
            });
        }
        if !seen.insert(name.clone()) {
            return Err(IcalRecurrenceError::DuplicatePart(name));
        }

        match name.as_str() {
            "FREQ" => {
                frequency = Some(
                    RecurrenceFrequency::parse(raw_value).ok_or_else(|| {
                        IcalRecurrenceError::InvalidPart {
                            name,
                            value: raw_value.to_string(),
                        }
                    })?,
                );
            }
            "INTERVAL" => {
                interval = parse_scalar::<u32>(&name, raw_value)?;
            }
            "COUNT" => {
                count = Some(parse_scalar::<u32>(&name, raw_value)?);
            }
            "UNTIL" => {
                until = Some(parse_until_date(raw_value)?);
            }
            "WKST" => {
                week_start = parse_weekday_code(&name, raw_value)?;
            }
            "BYDAY" => {
                for token in split_list(&name, raw_value)? {
                    let (ordinal, weekday) = parse_byday_token(token)?;
                    if let Some(ordinal) = ordinal {
                        by_month_weekday.push(RecurrenceOrdinalWeekday::new(ordinal, weekday));
                    } else {
                        by_weekday.push(weekday);
                    }
                }
            }
            "BYMONTH" => by_month = parse_list::<u8>(&name, raw_value)?,
            "BYWEEKNO" => by_week_no = parse_list::<i8>(&name, raw_value)?,
            "BYYEARDAY" => by_year_day = parse_list::<i16>(&name, raw_value)?,
            "BYMONTHDAY" => by_month_day = parse_list::<i8>(&name, raw_value)?,
            "BYHOUR" => by_hour = parse_list::<u8>(&name, raw_value)?,
            "BYMINUTE" => by_minute = parse_list::<u8>(&name, raw_value)?,
            "BYSECOND" => by_second = parse_list::<u8>(&name, raw_value)?,
            "BYSETPOS" => by_set_pos = parse_list::<i16>(&name, raw_value)?,
            _ => return Err(IcalRecurrenceError::UnknownPart(name)),
        }
    }

    let frequency = frequency.ok_or(IcalRecurrenceError::MissingFrequency)?;
    if count.is_some() && until.is_some() {
        return Err(IcalRecurrenceError::CountUntilConflict);
    }

    let rule = RecurrenceRule {
        frequency,
        interval,
        count,
        until,
        week_start,
        by_weekday,
        by_month,
        by_week_no,
        by_year_day,
        by_month_day,
        by_month_weekday,
        by_hour,
        by_minute,
        by_second,
        by_set_pos,
        rdates: Vec::new(),
        exdates: Vec::new(),
        overrides: Vec::new(),
    };
    rule.validate()?;
    Ok(rule)
}

/// Serialize the RRULE portion of an Ephemeris recurrence rule.
///
/// Exception properties (RDATE, EXDATE, and occurrence overrides) are not
/// emitted here. DATE-TIME UNTIL cannot be produced because Ephemeris'
/// canonical recurrence bound is a civil date.
pub fn format_rrule(rule: &RecurrenceRule) -> Result<String, IcalRecurrenceError> {
    rule.validate()?;
    if rule.count.is_some() && rule.until.is_some() {
        return Err(IcalRecurrenceError::CountUntilConflict);
    }

    let mut parts = vec![format!(
        "FREQ={}",
        rule.frequency.as_str().to_ascii_uppercase()
    )];

    if rule.interval != 1 {
        parts.push(format!("INTERVAL={}", rule.interval));
    }
    if let Some(count) = rule.count {
        parts.push(format!("COUNT={count}"));
    }
    if let Some(until) = rule.until {
        parts.push(format!("UNTIL={}", until.format("%Y%m%d")));
    }
    if rule.week_start != RecurrenceWeekday::Monday {
        parts.push(format!("WKST={}", weekday_code(rule.week_start)));
    }
    push_list(&mut parts, "BYMONTH", &rule.by_month);
    push_list(&mut parts, "BYWEEKNO", &rule.by_week_no);
    push_list(&mut parts, "BYYEARDAY", &rule.by_year_day);
    push_list(&mut parts, "BYMONTHDAY", &rule.by_month_day);

    if !rule.by_weekday.is_empty() || !rule.by_month_weekday.is_empty() {
        let mut values = rule
            .by_weekday
            .iter()
            .map(|weekday| weekday_code(*weekday).to_string())
            .collect::<Vec<_>>();
        values.extend(rule.by_month_weekday.iter().map(|selector| {
            format!("{}{}", selector.ordinal, weekday_code(selector.weekday))
        }));
        parts.push(format!("BYDAY={}", values.join(",")));
    }

    push_list(&mut parts, "BYHOUR", &rule.by_hour);
    push_list(&mut parts, "BYMINUTE", &rule.by_minute);
    push_list(&mut parts, "BYSECOND", &rule.by_second);
    push_list(&mut parts, "BYSETPOS", &rule.by_set_pos);

    Ok(parts.join(";"))
}

fn parse_until_date(raw: &str) -> Result<NaiveDate, IcalRecurrenceError> {
    if raw.contains('T') || raw.contains('t') {
        return Err(IcalRecurrenceError::UnsupportedUntilDateTime(
            raw.to_string(),
        ));
    }
    NaiveDate::parse_from_str(raw, "%Y%m%d").map_err(|_| IcalRecurrenceError::InvalidPart {
        name: "UNTIL".to_string(),
        value: raw.to_string(),
    })
}

fn parse_byday_token(
    raw: &str,
) -> Result<(Option<i8>, RecurrenceWeekday), IcalRecurrenceError> {
    if raw.len() < 2 || !raw.is_ascii() {
        return Err(IcalRecurrenceError::InvalidPart {
            name: "BYDAY".to_string(),
            value: raw.to_string(),
        });
    }
    let split = raw.len() - 2;
    let (ordinal_raw, weekday_raw) = raw.split_at(split);
    let weekday = parse_weekday_code("BYDAY", weekday_raw)?;
    if ordinal_raw.is_empty() {
        return Ok((None, weekday));
    }

    let ordinal = ordinal_raw
        .parse::<i8>()
        .map_err(|_| IcalRecurrenceError::InvalidPart {
            name: "BYDAY".to_string(),
            value: raw.to_string(),
        })?;
    if ordinal == 0 {
        return Err(IcalRecurrenceError::InvalidPart {
            name: "BYDAY".to_string(),
            value: raw.to_string(),
        });
    }
    Ok((Some(ordinal), weekday))
}

fn parse_weekday_code(
    name: &str,
    raw: &str,
) -> Result<RecurrenceWeekday, IcalRecurrenceError> {
    match raw.trim().to_ascii_uppercase().as_str() {
        "MO" => Ok(RecurrenceWeekday::Monday),
        "TU" => Ok(RecurrenceWeekday::Tuesday),
        "WE" => Ok(RecurrenceWeekday::Wednesday),
        "TH" => Ok(RecurrenceWeekday::Thursday),
        "FR" => Ok(RecurrenceWeekday::Friday),
        "SA" => Ok(RecurrenceWeekday::Saturday),
        "SU" => Ok(RecurrenceWeekday::Sunday),
        _ => Err(IcalRecurrenceError::InvalidPart {
            name: name.to_string(),
            value: raw.to_string(),
        }),
    }
}

const fn weekday_code(weekday: RecurrenceWeekday) -> &'static str {
    match weekday {
        RecurrenceWeekday::Monday => "MO",
        RecurrenceWeekday::Tuesday => "TU",
        RecurrenceWeekday::Wednesday => "WE",
        RecurrenceWeekday::Thursday => "TH",
        RecurrenceWeekday::Friday => "FR",
        RecurrenceWeekday::Saturday => "SA",
        RecurrenceWeekday::Sunday => "SU",
    }
}

fn split_list<'a>(
    name: &str,
    raw: &'a str,
) -> Result<Vec<&'a str>, IcalRecurrenceError> {
    let values = raw.split(',').map(str::trim).collect::<Vec<_>>();
    if values.is_empty() || values.iter().any(|value| value.is_empty()) {
        return Err(IcalRecurrenceError::InvalidPart {
            name: name.to_string(),
            value: raw.to_string(),
        });
    }
    Ok(values)
}

fn parse_list<T>(name: &str, raw: &str) -> Result<Vec<T>, IcalRecurrenceError>
where
    T: std::str::FromStr,
{
    split_list(name, raw)?
        .into_iter()
        .map(|value| parse_scalar(name, value))
        .collect()
}

fn parse_scalar<T>(name: &str, raw: &str) -> Result<T, IcalRecurrenceError>
where
    T: std::str::FromStr,
{
    raw.parse::<T>()
        .map_err(|_| IcalRecurrenceError::InvalidPart {
            name: name.to_string(),
            value: raw.to_string(),
        })
}

fn push_list<T>(parts: &mut Vec<String>, name: &str, values: &[T])
where
    T: fmt::Display,
{
    if values.is_empty() {
        return;
    }
    parts.push(format!(
        "{name}={}",
        values
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_and_prefixed_rrules_case_insensitively() {
        let bare = parse_rrule("FREQ=weekly;BYDAY=MO,WE,FR;COUNT=5").expect("bare");
        let prefixed = parse_rrule("RrUlE:FREQ=WEEKLY;BYDAY=MO,WE,FR;COUNT=5").expect("prefixed");

        assert_eq!(bare, prefixed);
        assert_eq!(bare.frequency, RecurrenceFrequency::Weekly);
        assert_eq!(bare.count, Some(5));
        assert_eq!(
            bare.by_weekday,
            vec![
                RecurrenceWeekday::Monday,
                RecurrenceWeekday::Wednesday,
                RecurrenceWeekday::Friday
            ]
        );
    }

    #[test]
    fn parses_all_supported_rrule_parts() {
        let rule = parse_rrule(
            "RRULE:FREQ=YEARLY;INTERVAL=2;COUNT=4;WKST=SU;BYMONTH=1,7;BYWEEKNO=1,-1;BYYEARDAY=1,-1;BYMONTHDAY=1,-1;BYDAY=MO;BYHOUR=9,17;BYMINUTE=0,30;BYSECOND=0,45;BYSETPOS=1,-1",
        )
        .expect("rule");

        assert_eq!(rule.frequency, RecurrenceFrequency::Yearly);
        assert_eq!(rule.interval, 2);
        assert_eq!(rule.count, Some(4));
        assert_eq!(rule.week_start, RecurrenceWeekday::Sunday);
        assert_eq!(rule.by_month, vec![1, 7]);
        assert_eq!(rule.by_week_no, vec![1, -1]);
        assert_eq!(rule.by_year_day, vec![1, -1]);
        assert_eq!(rule.by_month_day, vec![1, -1]);
        assert_eq!(rule.by_weekday, vec![RecurrenceWeekday::Monday]);
        assert!(rule.by_month_weekday.is_empty());
        assert_eq!(rule.by_hour, vec![9, 17]);
        assert_eq!(rule.by_minute, vec![0, 30]);
        assert_eq!(rule.by_second, vec![0, 45]);
        assert_eq!(rule.by_set_pos, vec![1, -1]);
    }

    #[test]
    fn parses_ordinal_byday_in_supported_monthly_context() {
        let rule = parse_rrule("FREQ=MONTHLY;BYDAY=1MO,-1FR").expect("rule");

        assert!(rule.by_weekday.is_empty());
        assert_eq!(
            rule.by_month_weekday,
            vec![
                RecurrenceOrdinalWeekday::new(1, RecurrenceWeekday::Monday),
                RecurrenceOrdinalWeekday::new(-1, RecurrenceWeekday::Friday),
            ]
        );
    }

    #[test]
    fn accepts_rfc_date_until_and_formats_it_canonically() {
        let rule = parse_rrule("FREQ=DAILY;UNTIL=20261231").expect("rule");
        assert_eq!(
            rule.until,
            Some(NaiveDate::from_ymd_opt(2026, 12, 31).expect("date"))
        );
        assert_eq!(
            format_rrule(&rule).expect("format"),
            "FREQ=DAILY;UNTIL=20261231"
        );
    }

    #[test]
    fn rejects_until_datetime_instead_of_truncating_precision() {
        assert_eq!(
            parse_rrule("FREQ=DAILY;UNTIL=20261231T230000Z"),
            Err(IcalRecurrenceError::UnsupportedUntilDateTime(
                "20261231T230000Z".to_string()
            ))
        );
    }

    #[test]
    fn rejects_count_and_until_together() {
        assert_eq!(
            parse_rrule("FREQ=DAILY;COUNT=3;UNTIL=20261231"),
            Err(IcalRecurrenceError::CountUntilConflict)
        );
    }

    #[test]
    fn rejects_missing_duplicate_and_unknown_parts() {
        assert_eq!(
            parse_rrule("INTERVAL=2"),
            Err(IcalRecurrenceError::MissingFrequency)
        );
        assert_eq!(
            parse_rrule("FREQ=DAILY;FREQ=WEEKLY"),
            Err(IcalRecurrenceError::DuplicatePart("FREQ".to_string()))
        );
        assert_eq!(
            parse_rrule("FREQ=DAILY;RSCALE=GREGORIAN"),
            Err(IcalRecurrenceError::UnknownPart("RSCALE".to_string()))
        );
    }

    #[test]
    fn rejects_malformed_byday_tokens() {
        assert!(matches!(
            parse_rrule("FREQ=MONTHLY;BYDAY=0MO"),
            Err(IcalRecurrenceError::InvalidPart { name, .. }) if name == "BYDAY"
        ));
        assert!(matches!(
            parse_rrule("FREQ=MONTHLY;BYDAY=1XX"),
            Err(IcalRecurrenceError::InvalidPart { name, .. }) if name == "BYDAY"
        ));
        assert!(matches!(
            parse_rrule("FREQ=MONTHLY;BYDAY=éMO"),
            Err(IcalRecurrenceError::InvalidPart { name, .. }) if name == "BYDAY"
        ));
    }

    #[test]
    fn delegates_selector_ranges_and_context_to_domain_validation() {
        assert!(matches!(
            parse_rrule("FREQ=YEARLY;BYMONTH=13"),
            Err(IcalRecurrenceError::Domain(RecurrenceError::InvalidByMonth(13)))
        ));
        assert!(matches!(
            parse_rrule("FREQ=DAILY;BYWEEKNO=1"),
            Err(IcalRecurrenceError::Domain(RecurrenceError::ByWeekNoRequiresYearly))
        ));
        assert!(matches!(
            parse_rrule("FREQ=DAILY;BYSECOND=60"),
            Err(IcalRecurrenceError::Domain(
                RecurrenceError::BySecondLeapSecondUnsupported
            ))
        ));
    }

    #[test]
    fn formats_stable_rfc_order_with_freq_first() {
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Monthly);
        rule.interval = 2;
        rule.count = Some(6);
        rule.by_weekday = vec![
            RecurrenceWeekday::Monday,
            RecurrenceWeekday::Tuesday,
            RecurrenceWeekday::Wednesday,
            RecurrenceWeekday::Thursday,
            RecurrenceWeekday::Friday,
        ];
        rule.by_set_pos = vec![-1];

        assert_eq!(
            format_rrule(&rule).expect("format"),
            "FREQ=MONTHLY;INTERVAL=2;COUNT=6;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1"
        );
    }

    #[test]
    fn roundtrips_supported_rule_without_exception_properties() {
        let original = parse_rrule(
            "FREQ=YEARLY;INTERVAL=3;WKST=SU;BYMONTH=1,7;BYDAY=MO,-1FR;BYHOUR=9;BYMINUTE=30;BYSETPOS=-1",
        )
        .expect("parse");
        let encoded = format_rrule(&original).expect("format");
        let decoded = parse_rrule(&encoded).expect("reparse");

        assert_eq!(decoded, original);
        assert!(decoded.rdates.is_empty());
        assert!(decoded.exdates.is_empty());
        assert!(decoded.overrides.is_empty());
    }

    #[test]
    fn formatter_refuses_non_rfc_count_until_combination() {
        let mut rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        rule.count = Some(3);
        rule.until = Some(NaiveDate::from_ymd_opt(2026, 12, 31).expect("date"));

        assert_eq!(
            format_rrule(&rule),
            Err(IcalRecurrenceError::CountUntilConflict)
        );
    }

    #[test]
    fn formatter_omits_default_interval_and_week_start() {
        let rule = RecurrenceRule::new(RecurrenceFrequency::Daily);
        assert_eq!(format_rrule(&rule).expect("format"), "FREQ=DAILY");
    }
}
